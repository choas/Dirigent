use std::path::{Path, PathBuf};

use crate::error::DirigentError;

#[derive(Debug, Clone)]
pub(crate) struct WorktreeInfo {
    pub name: String,
    pub path: PathBuf,
    pub is_current: bool,
    pub is_locked: bool,
    pub is_main: bool,
}

fn build_worktree_info(
    p: PathBuf,
    branch: Option<String>,
    is_locked: bool,
    is_first: bool,
    current: &Path,
) -> WorktreeInfo {
    let canon_wt = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
    let name = branch
        .and_then(|b| b.strip_prefix("refs/heads/").map(|s| s.to_string()))
        .unwrap_or_else(|| {
            p.file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| "main".to_string())
        });
    let is_current = canon_wt == current || current.starts_with(&canon_wt);
    WorktreeInfo {
        name,
        path: p,
        is_current,
        is_locked,
        is_main: is_first,
    }
}

pub(crate) fn list_worktrees(repo_path: &Path) -> crate::error::Result<Vec<WorktreeInfo>> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        return Err(DirigentError::GitCommand(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let current = std::fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    let mut worktrees = Vec::new();

    let mut wt_path: Option<PathBuf> = None;
    let mut branch: Option<String> = None;
    let mut is_bare = false;
    let mut is_locked = false;

    for line in stdout.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if let Some(p) = wt_path.take() {
                if !is_bare {
                    worktrees.push(build_worktree_info(
                        p,
                        branch.take(),
                        is_locked,
                        worktrees.is_empty(),
                        &current,
                    ));
                }
            }
            is_bare = false;
            is_locked = false;
            branch = None;
        } else if let Some(rest) = line.strip_prefix("worktree ") {
            wt_path = Some(PathBuf::from(rest));
        } else if line == "bare" {
            is_bare = true;
        } else if line.starts_with("locked") {
            is_locked = true;
        } else if let Some(rest) = line.strip_prefix("branch ") {
            branch = Some(rest.to_string());
        }
    }

    Ok(worktrees)
}

/// List branch names available for worktree creation (local + remote-tracking),
/// excluding branches already checked out in an existing worktree.
pub(crate) fn list_branches(repo_path: &Path) -> crate::error::Result<Vec<String>> {
    use std::collections::BTreeSet;

    // Fetch latest remote state and prune stale remote-tracking branches
    // so the list reflects what actually exists on the remote.
    fetch_and_prune(repo_path);

    // Collect branches already checked out in worktrees so we can exclude them.
    let checked_out: std::collections::HashSet<String> = list_worktrees(repo_path)?
        .iter()
        .map(|wt| wt.name.clone())
        .collect();

    // Collect local branches whose upstream was deleted (e.g. after PR merge).
    let gone = collect_gone_branches(repo_path);

    let mut branches = BTreeSet::new();
    collect_local_branches(repo_path, &checked_out, &gone, &mut branches)?;
    collect_remote_branches(repo_path, &checked_out, &mut branches)?;
    Ok(branches.into_iter().collect())
}

/// Fetch latest remote refs and prune stale remote-tracking branches.
/// Best-effort / fire-and-forget: the process is spawned in the background
/// so callers on the UI thread are not blocked by network I/O.
fn fetch_and_prune(repo_path: &Path) {
    use std::process::{Command, Stdio};
    let _ = Command::new("git")
        .args(["fetch", "--prune"])
        .current_dir(repo_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn(); // fire-and-forget — avoids blocking the UI thread
}

/// Return the set of local branch names whose upstream tracking branch has
/// been deleted (marked `[gone]`), e.g. after a PR was merged and the remote
/// branch removed.
fn collect_gone_branches(repo_path: &Path) -> std::collections::HashSet<String> {
    use std::process::Command;
    let mut gone = std::collections::HashSet::new();
    let Ok(output) = Command::new("git")
        .args([
            "for-each-ref",
            "--format=%(refname:short) %(upstream:track)",
            "refs/heads/",
        ])
        .current_dir(repo_path)
        .output()
    else {
        return gone;
    };
    if !output.status.success() {
        return gone;
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        // The format is "branchname [gone]" — match the trailing tracking status,
        // not an arbitrary substring (avoids false positives on branch names).
        if line.trim_end().ends_with("[gone]") {
            if let Some(name) = line.split_whitespace().next() {
                gone.insert(name.to_string());
            }
        }
    }
    gone
}

fn collect_local_branches(
    repo_path: &Path,
    checked_out: &std::collections::HashSet<String>,
    gone: &std::collections::HashSet<String>,
    branches: &mut std::collections::BTreeSet<String>,
) -> crate::error::Result<()> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["branch", "--format=%(refname:short)"])
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        return Ok(());
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let name = line.trim().to_string();
        if !name.is_empty() && !checked_out.contains(&name) && !gone.contains(&name) {
            branches.insert(name);
        }
    }
    Ok(())
}

fn collect_remote_branches(
    repo_path: &Path,
    checked_out: &std::collections::HashSet<String>,
    branches: &mut std::collections::BTreeSet<String>,
) -> crate::error::Result<()> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["branch", "-r", "--format=%(refname:short)"])
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        return Ok(());
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let full = line.trim();
        if full.is_empty() || full.contains("HEAD") {
            continue;
        }
        // Strip "origin/" (or any remote name) prefix.
        let short = full.find('/').map_or(full, |pos| &full[pos + 1..]);
        if !short.is_empty() && !checked_out.contains(short) {
            branches.insert(short.to_string());
        }
    }
    Ok(())
}

pub(crate) fn create_worktree(repo_path: &Path, name: &str) -> crate::error::Result<PathBuf> {
    use std::process::Command;

    // Always resolve the *main* worktree so new worktrees are created as
    // siblings of it, even when called from a linked (secondary) worktree.
    let worktrees = list_worktrees(repo_path)?;
    let main_wt = worktrees
        .iter()
        .find(|wt| wt.is_main)
        .ok_or_else(|| DirigentError::GitCommand("no main worktree found".into()))?;
    let workdir = main_wt.path.clone();
    let parent = workdir
        .parent()
        .ok_or_else(|| DirigentError::GitCommand("no parent directory".into()))?;

    // Use the last path component for the directory name to avoid nested dirs
    // when the branch name contains slashes (e.g. "claude/feature-xyz").
    let dir_name = name.rsplit('/').next().unwrap_or(name);
    let wt_path = parent.join(dir_name);

    // First try without -b: this works for local branches AND remote-tracking
    // branches (git auto-creates a local tracking branch from e.g.
    // origin/claude/add-deno-support-brnCw).
    let output = Command::new("git")
        .args(["worktree", "add", &wt_path.to_string_lossy(), name])
        .current_dir(repo_path)
        .output()?;

    // If the branch didn't exist at all, fall back to creating it with -b.
    if !output.status.success() {
        let output = Command::new("git")
            .args(["worktree", "add", "-b", name, &wt_path.to_string_lossy()])
            .current_dir(repo_path)
            .output()?;

        if !output.status.success() {
            return Err(DirigentError::GitCommand(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
    }

    // Copy .claude/ settings from the main worktree so the new worktree
    // inherits Claude Code permissions, hooks, and project configuration.
    copy_claude_settings(&workdir, &wt_path);

    Ok(wt_path)
}

/// Recursively copy the `.claude/` directory from `src_root` to `dst_root`,
/// skipping the `worktrees/` subdirectory (Claude Code manages that itself).
fn copy_claude_settings(src_root: &Path, dst_root: &Path) {
    let src_dir = src_root.join(".claude");
    if !src_dir.is_dir() {
        return;
    }
    let dst_dir = dst_root.join(".claude");
    if let Err(e) = copy_dir_recursive(&src_dir, &dst_dir, &["worktrees"]) {
        eprintln!("warning: failed to copy .claude/ settings to worktree: {e}");
    }
}

/// Recursively copy `src` to `dst`, skipping entries whose file name matches
/// any entry in `skip`.
fn copy_dir_recursive(src: &Path, dst: &Path, skip: &[&str]) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if skip.iter().any(|s| *s == name.to_string_lossy().as_ref()) {
            continue;
        }
        let src_path = entry.path();
        let dst_path = dst.join(&name);
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path, &[])?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// Switch the current branch by running `git checkout <branch>`.
pub(crate) fn checkout_branch(repo_path: &Path, branch: &str) -> crate::error::Result<()> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["checkout", branch])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        return Err(DirigentError::GitCommand(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn remove_worktree(
    repo_path: &Path,
    wt_path: &Path,
    force: bool,
) -> crate::error::Result<()> {
    use std::process::Command;

    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    let wt_str = wt_path.to_string_lossy();
    args.push(&wt_str);

    let output = Command::new("git")
        .args(&args)
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        return Err(DirigentError::GitCommand(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    Ok(())
}
