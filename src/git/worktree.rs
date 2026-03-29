use std::path::{Path, PathBuf};

use git2::Repository;

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
    use std::process::Command;

    // Collect branches already checked out in worktrees so we can exclude them.
    let checked_out: std::collections::HashSet<String> = list_worktrees(repo_path)?
        .iter()
        .map(|wt| wt.name.clone())
        .collect();

    let mut branches = BTreeSet::new();

    // Local branches.
    let output = Command::new("git")
        .args(["branch", "--format=%(refname:short)"])
        .current_dir(repo_path)
        .output()?;
    if output.status.success() {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let name = line.trim().to_string();
            if !name.is_empty() && !checked_out.contains(&name) {
                branches.insert(name);
            }
        }
    }

    // Remote-tracking branches (strip the remote prefix, e.g. "origin/foo" -> "foo").
    let output = Command::new("git")
        .args(["branch", "-r", "--format=%(refname:short)"])
        .current_dir(repo_path)
        .output()?;
    if output.status.success() {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let full = line.trim();
            if full.is_empty() || full.contains("HEAD") {
                continue;
            }
            // Strip "origin/" (or any remote name) prefix.
            let name = if let Some(pos) = full.find('/') {
                &full[pos + 1..]
            } else {
                full
            };
            if !name.is_empty() && !checked_out.contains(name) {
                branches.insert(name.to_string());
            }
        }
    }

    Ok(branches.into_iter().collect())
}

pub(crate) fn create_worktree(repo_path: &Path, name: &str) -> crate::error::Result<PathBuf> {
    use std::process::Command;

    let repo = Repository::discover(repo_path)?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| DirigentError::GitCommand("no workdir".into()))?
        .to_path_buf();
    drop(repo);
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

    Ok(wt_path)
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
