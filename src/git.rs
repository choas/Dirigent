use std::collections::{HashMap, HashSet};

use git2::{Repository, Signature, StatusOptions};
use std::path::{Path, PathBuf};

use crate::error::DirigentError;

#[derive(Debug, Clone)]
pub(crate) struct GitInfo {
    pub branch: String,
    pub last_commit_hash: String,
    pub last_commit_message: String,
    pub modified_count: usize,
    pub added_count: usize,
    pub deleted_count: usize,
}

pub(crate) fn read_git_info(path: &Path) -> Option<GitInfo> {
    let repo = Repository::discover(path).ok()?;

    let branch = {
        if let Ok(head) = repo.head() {
            if head.is_branch() {
                head.shorthand().unwrap_or("HEAD").to_string()
            } else {
                "HEAD detached".to_string()
            }
        } else {
            "no commits".to_string()
        }
    };

    let (hash, message) = {
        if let Ok(head) = repo.head() {
            if let Ok(commit) = head.peel_to_commit() {
                let h = format!("{}", commit.id());
                let short = h.chars().take(7).collect::<String>();
                let msg = commit
                    .message()
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                (short, msg)
            } else {
                (String::new(), String::new())
            }
        } else {
            (String::new(), String::new())
        }
    };

    let (modified, added, deleted) = {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true).recurse_untracked_dirs(true);
        if let Ok(statuses) = repo.statuses(Some(&mut opts)) {
            let mut m = 0;
            let mut a = 0;
            let mut d = 0;
            for entry in statuses.iter() {
                let s = entry.status();
                if s.intersects(
                    git2::Status::WT_MODIFIED
                        | git2::Status::INDEX_MODIFIED
                        | git2::Status::WT_RENAMED
                        | git2::Status::INDEX_RENAMED,
                ) {
                    m += 1;
                } else if s.intersects(git2::Status::WT_NEW | git2::Status::INDEX_NEW) {
                    a += 1;
                } else if s.intersects(git2::Status::WT_DELETED | git2::Status::INDEX_DELETED) {
                    d += 1;
                }
            }
            (m, a, d)
        } else {
            (0, 0, 0)
        }
    };

    Some(GitInfo {
        branch,
        last_commit_hash: hash,
        last_commit_message: message,
        modified_count: modified,
        added_count: added,
        deleted_count: deleted,
    })
}

/// Returns relative paths of all files with uncommitted changes, mapped to their
/// git status letter (M = modified, A = added/new, D = deleted, R = renamed, ? = untracked).
pub(crate) fn get_dirty_files(path: &Path) -> HashMap<String, char> {
    let mut result = HashMap::new();
    let repo = match Repository::discover(path) {
        Ok(r) => r,
        Err(_) => return result,
    };
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    if let Ok(statuses) = repo.statuses(Some(&mut opts)) {
        for entry in statuses.iter() {
            let s = entry.status();
            let letter = if s.intersects(git2::Status::WT_DELETED | git2::Status::INDEX_DELETED) {
                'D'
            } else if s.intersects(git2::Status::WT_RENAMED | git2::Status::INDEX_RENAMED) {
                'R'
            } else if s.intersects(git2::Status::WT_MODIFIED | git2::Status::INDEX_MODIFIED) {
                'M'
            } else if s.intersects(git2::Status::INDEX_NEW) {
                'A'
            } else if s.intersects(git2::Status::WT_NEW) {
                '?'
            } else {
                continue;
            };
            if let Some(p) = entry.path() {
                result.insert(p.to_string(), letter);
            }
        }
    }
    result
}

/// Returns the number of commits the local branch is ahead of its remote tracking branch.
/// When there is no remote tracking branch (e.g. a new worktree branch), compares against
/// the default branch (main/master) on origin instead.
pub(crate) fn get_ahead_of_remote(path: &Path) -> usize {
    let repo = match Repository::discover(path) {
        Ok(r) => r,
        Err(_) => return 0,
    };
    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => return 0,
    };
    let local_oid = match head.target() {
        Some(oid) => oid,
        None => return 0,
    };
    let branch_name = match head.shorthand() {
        Some(name) => name.to_string(),
        None => return 0,
    };
    let upstream_ref = format!("refs/remotes/origin/{}", branch_name);
    let remote_oid = match repo.refname_to_id(&upstream_ref) {
        Ok(oid) => oid,
        Err(_) => {
            // No remote tracking branch — compare against origin's default branch
            let default_oid = repo
                .refname_to_id("refs/remotes/origin/main")
                .or_else(|_| repo.refname_to_id("refs/remotes/origin/master"))
                .ok();
            match default_oid {
                Some(oid) => {
                    return repo
                        .graph_ahead_behind(local_oid, oid)
                        .map(|(ahead, _)| ahead)
                        .unwrap_or(0);
                }
                None => return 0,
            }
        }
    };
    match repo.graph_ahead_behind(local_oid, remote_oid) {
        Ok((ahead, _behind)) => ahead,
        Err(_) => 0,
    }
}

pub(crate) fn format_status_summary(info: &GitInfo) -> String {
    format!(
        "~{} +{} -{}",
        info.modified_count, info.added_count, info.deleted_count
    )
}

// -- Commit history --

#[derive(Debug, Clone)]
pub(crate) struct CommitInfo {
    pub full_hash: String,
    pub short_hash: String,
    pub message: String,
    pub body: String,
    pub author: String,
    pub time_ago: String,
}

pub(crate) fn read_commit_history(path: &Path, limit: usize) -> Vec<CommitInfo> {
    let repo = match Repository::discover(path) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let mut revwalk = match repo.revwalk() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    if revwalk.push_head().is_err() {
        return Vec::new();
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let mut commits = Vec::new();
    for oid in revwalk.take(limit) {
        let oid = match oid {
            Ok(o) => o,
            Err(_) => continue,
        };
        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let hash = format!("{}", commit.id());
        let short_hash = hash.chars().take(7).collect::<String>();
        let full_message = commit.message().unwrap_or("");
        let message = full_message.lines().next().unwrap_or("").to_string();
        let body = full_message.trim().to_string();
        let author = commit.author().name().unwrap_or("").to_string();
        let secs = commit.time().seconds();
        let diff = now - secs;
        let time_ago = if diff < 60 {
            "just now".to_string()
        } else if diff < 3600 {
            format!("{}m ago", diff / 60)
        } else if diff < 86400 {
            format!("{}h ago", diff / 3600)
        } else {
            format!("{}d ago", diff / 86400)
        };
        commits.push(CommitInfo {
            full_hash: hash,
            short_hash,
            message,
            body,
            author,
            time_ago,
        });
    }
    commits
}

pub(crate) fn count_commits(path: &Path) -> usize {
    let repo = match Repository::discover(path) {
        Ok(r) => r,
        Err(_) => return 0,
    };
    let mut revwalk = match repo.revwalk() {
        Ok(r) => r,
        Err(_) => return 0,
    };
    if revwalk.push_head().is_err() {
        return 0;
    }
    revwalk.count()
}

pub(crate) fn get_commit_diff(path: &Path, commit_hash: &str) -> Option<String> {
    use std::process::Command;
    let output = Command::new("git")
        .args(["diff-tree", "--root", "-p", commit_hash])
        .current_dir(path)
        .output()
        .ok()?;

    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        Some(text)
    } else {
        None
    }
}

/// Get the working-tree diff for specific files (or all files if empty).
/// Returns the `git diff` output, or None if there are no changes.
/// Also generates diffs for untracked (new) files so they appear in review and commits.
pub(crate) fn get_working_diff(repo_path: &Path, files: &[String]) -> Option<String> {
    use std::process::Command;

    // Helper: make a path relative to repo root
    let make_relative = |f: &str| -> String {
        let path = Path::new(f);
        if path.is_absolute() {
            path.strip_prefix(repo_path)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string()
        } else {
            f.to_string()
        }
    };

    // Collect relative paths
    let rel_files: Vec<String> = if files.is_empty() {
        Vec::new()
    } else {
        files.iter().map(|f| make_relative(f)).collect()
    };

    // Get diff for tracked/modified files
    let mut cmd = Command::new("git");
    cmd.arg("diff").current_dir(repo_path);
    if !rel_files.is_empty() {
        cmd.arg("--");
        for f in &rel_files {
            cmd.arg(f);
        }
    }

    let output = cmd.output().ok()?;
    let mut diff = if output.status.success() {
        String::from_utf8_lossy(&output.stdout).to_string()
    } else {
        String::new()
    };

    // Find untracked files and generate new-file diffs for them
    let untracked = find_untracked_files(repo_path);
    let check_files: Vec<&String> = if rel_files.is_empty() {
        // All untracked files
        untracked.iter().collect()
    } else {
        // Only untracked files that are in our file list
        rel_files
            .iter()
            .filter(|f| untracked.contains(f.as_str()))
            .collect()
    };

    for rel_path in check_files {
        let full_path = repo_path.join(rel_path);
        if let Ok(contents) = std::fs::read_to_string(&full_path) {
            let line_count = contents.lines().count().max(1);
            diff.push_str(&format!("diff --git a/{rel_path} b/{rel_path}\n"));
            diff.push_str("new file mode 100644\n");
            diff.push_str("--- /dev/null\n");
            diff.push_str(&format!("+++ b/{rel_path}\n"));
            diff.push_str(&format!("@@ -0,0 +1,{line_count} @@\n"));
            for line in contents.lines() {
                diff.push('+');
                diff.push_str(line);
                diff.push('\n');
            }
            // Ensure trailing newline marker if file doesn't end with one
            if !contents.ends_with('\n') {
                diff.push_str("\\ No newline at end of file\n");
            }
        }
    }

    if diff.trim().is_empty() {
        None
    } else {
        Some(diff)
    }
}

/// Returns relative paths of all untracked files in the repo.
fn find_untracked_files(repo_path: &Path) -> HashSet<String> {
    let mut result = HashSet::new();
    let repo = match Repository::discover(repo_path) {
        Ok(r) => r,
        Err(_) => return result,
    };
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    if let Ok(statuses) = repo.statuses(Some(&mut opts)) {
        for entry in statuses.iter() {
            let s = entry.status();
            if s.intersects(git2::Status::WT_NEW) && !s.intersects(git2::Status::INDEX_NEW) {
                if let Some(p) = entry.path() {
                    result.insert(p.to_string());
                }
            }
        }
    }
    result
}

/// Like parse_diff_file_paths but also strips a directory prefix if present.
/// Use this when the diff may have been generated with paths relative to a parent dir.
pub(crate) fn parse_diff_file_paths_for_repo(repo_path: &Path, diff_text: &str) -> Vec<String> {
    let dir_name = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir_prefix = if dir_name.is_empty() {
        String::new()
    } else {
        format!("{}/", dir_name)
    };

    let mut paths = Vec::new();
    for line in diff_text.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            let path = rest.trim();
            // Strip dir prefix if present
            let path = path.strip_prefix(dir_prefix.as_str()).unwrap_or(path);
            let path = path.to_string();
            if !path.is_empty() && !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

/// Commit the working-tree state of files touched by `diff_text`.
/// This stages the actual files the user sees (including any post-run formatting),
/// so the committed state matches the working tree and files appear clean afterwards.
pub(crate) fn commit_diff(
    repo_path: &Path,
    diff_text: &str,
    commit_message: &str,
) -> crate::error::Result<String> {
    use std::process::Command;

    // Parse all file paths from the diff (both source and destination) to handle
    // additions, modifications, and deletions.
    let dir_name = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir_prefix = if dir_name.is_empty() {
        String::new()
    } else {
        format!("{}/", dir_name)
    };

    let mut file_paths = Vec::new();
    for line in diff_text.lines() {
        let rest = if let Some(r) = line.strip_prefix("+++ b/") {
            Some(r)
        } else if let Some(r) = line.strip_prefix("--- a/") {
            Some(r)
        } else {
            None
        };
        if let Some(rest) = rest {
            let path = rest.trim();
            let path = path.strip_prefix(dir_prefix.as_str()).unwrap_or(path);
            let path = path.to_string();
            if !path.is_empty() && !file_paths.contains(&path) {
                file_paths.push(path);
            }
        }
    }

    if file_paths.is_empty() {
        return Err(DirigentError::GitCommand(
            "no files to commit — diff contains no file paths".into(),
        ));
    }

    // Stage the working-tree state of the affected files.
    let output = Command::new("git")
        .arg("add")
        .arg("-A")
        .arg("--")
        .args(&file_paths)
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "staging files failed: {stderr}"
        )));
    }

    // Now commit whatever is staged
    let repo = Repository::discover(repo_path)?;
    let mut index = repo.index()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let sig = repo
        .signature()
        .unwrap_or_else(|_| Signature::now("Dirigent", "Dirigent@local").unwrap());

    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());

    // Nothing changed in the index
    if let Some(ref parent_commit) = parent {
        if parent_commit.tree_id() == tree_id {
            return Err(DirigentError::GitCommand(
                "nothing to commit — diff already applied".into(),
            ));
        }
    }

    let parents: Vec<&git2::Commit> = parent.iter().collect();

    let oid = repo.commit(Some("HEAD"), &sig, &sig, commit_message, &tree, &parents)?;

    // Reset the index back to the newly created commit so it doesn't stay staged.
    // Use the returned OID rather than repo.head() to avoid stale refdb cache.
    let new_commit = repo.find_commit(oid)?;
    repo.reset(new_commit.as_object(), git2::ResetType::Mixed, None)?;

    Ok(format!("{}", oid))
}

pub(crate) fn revert_files(repo_path: &Path, file_paths: &[String]) -> crate::error::Result<()> {
    use std::process::Command;

    if file_paths.is_empty() {
        return Ok(());
    }

    let mut cmd = Command::new("git");
    cmd.arg("checkout").arg("--");
    for f in file_paths {
        cmd.arg(f);
    }
    let output = cmd.current_dir(repo_path).output()?;

    if !output.status.success() {
        return Err(DirigentError::GitCommand(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(())
}

/// Stage all changes (tracked + untracked) and commit with the given message.
pub(crate) fn commit_all(repo_path: &Path, commit_message: &str) -> crate::error::Result<String> {
    use std::process::Command;

    // Stage all changes including untracked files
    let output = Command::new("git")
        .args(["add", "-A"])
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "git add -A failed: {}",
            stderr
        )));
    }

    // Commit
    let repo = Repository::discover(repo_path)?;
    let mut index = repo.index()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let sig = repo
        .signature()
        .unwrap_or_else(|_| Signature::now("Dirigent", "Dirigent@local").unwrap());

    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());

    if let Some(ref parent_commit) = parent {
        if parent_commit.tree_id() == tree_id {
            return Err(DirigentError::GitCommand(
                "nothing to commit — no uncommitted changes".into(),
            ));
        }
    }

    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let oid = repo.commit(Some("HEAD"), &sig, &sig, commit_message, &tree, &parents)?;

    // Reset index so it doesn't stay staged.
    // Use the returned OID rather than repo.head() to avoid stale refdb cache.
    let new_commit = repo.find_commit(oid)?;
    repo.reset(new_commit.as_object(), git2::ResetType::Mixed, None)?;

    Ok(format!("{}", oid))
}

pub(crate) fn generate_commit_message(cue_text: &str) -> String {
    let summary = if cue_text.len() > 68 {
        format!("{}...", crate::app::truncate_str(cue_text, 65))
    } else {
        cue_text.to_string()
    };
    if cue_text.len() > 68 {
        format!("Dirigent: {}\n\n{}", summary, cue_text)
    } else {
        format!("Dirigent: {}", summary)
    }
}

/// Push the current branch to its remote (typically `origin`).
/// When there is no remote tracking branch (e.g. a new worktree branch), pushes with
/// `-u origin <branch>` to set up tracking.
/// Returns the remote name and branch that was pushed (e.g. "origin/main").
pub(crate) fn git_push(repo_path: &Path) -> crate::error::Result<String> {
    use std::process::Command;

    // Determine current branch
    let repo = Repository::discover(repo_path)?;
    let head = repo
        .head()
        .map_err(|e| DirigentError::GitCommand(format!("cannot determine HEAD: {}", e)))?;
    let branch_name = head
        .shorthand()
        .ok_or_else(|| DirigentError::GitCommand("HEAD is not on a branch".into()))?
        .to_string();

    // Check if there is a remote tracking branch
    let upstream_ref = format!("refs/remotes/origin/{}", branch_name);
    let has_upstream = repo.refname_to_id(&upstream_ref).is_ok();

    let output = if has_upstream {
        Command::new("git")
            .args(["push", "--porcelain", "--follow-tags"])
            .current_dir(repo_path)
            .output()?
    } else {
        // No upstream — push with -u to set up tracking
        Command::new("git")
            .args([
                "push",
                "-u",
                "origin",
                &branch_name,
                "--porcelain",
                "--follow-tags",
            ])
            .current_dir(repo_path)
            .output()?
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "git push failed: {}",
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(format!(
        "Pushed {} ({})",
        branch_name,
        stdout.lines().next().unwrap_or("ok").trim()
    ))
}

/// Pull the current branch from its remote (typically `origin`).
/// Returns a summary string describing the result.
pub(crate) fn git_pull(repo_path: &Path) -> crate::error::Result<String> {
    use std::process::Command;

    // Determine current branch
    let repo = Repository::discover(repo_path)?;
    let head = repo
        .head()
        .map_err(|e| DirigentError::GitCommand(format!("cannot determine HEAD: {}", e)))?;
    let branch_name = head
        .shorthand()
        .ok_or_else(|| DirigentError::GitCommand("HEAD is not on a branch".into()))?
        .to_string();

    let output = Command::new("git")
        .args(["pull", "--ff-only"])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "git pull failed: {}",
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let summary = stdout.lines().next().unwrap_or("ok").trim();
    Ok(format!("Pulled {} ({})", branch_name, summary))
}

// -- Worktree support --

#[derive(Debug, Clone)]
pub(crate) struct WorktreeInfo {
    pub name: String,
    pub path: PathBuf,
    pub is_current: bool,
    pub is_locked: bool,
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
                    let canon_wt = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
                    let name = branch
                        .take()
                        .and_then(|b| b.rsplit('/').next().map(|s| s.to_string()))
                        .unwrap_or_else(|| {
                            p.file_name()
                                .map(|f| f.to_string_lossy().to_string())
                                .unwrap_or_else(|| "main".to_string())
                        });
                    let is_current = canon_wt == current || current.starts_with(&canon_wt);
                    worktrees.push(WorktreeInfo {
                        name,
                        path: p,
                        is_current,
                        is_locked,
                    });
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

pub(crate) fn create_worktree(repo_path: &Path, name: &str) -> crate::error::Result<PathBuf> {
    use std::process::Command;

    let repo = Repository::discover(repo_path)?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| DirigentError::GitCommand("no workdir".into()))?
        .to_path_buf();
    let parent = workdir
        .parent()
        .ok_or_else(|| DirigentError::GitCommand("no parent directory".into()))?;
    let wt_path = parent.join(name);

    let output = Command::new("git")
        .args(["worktree", "add", "-b", name, &wt_path.to_string_lossy()])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        return Err(DirigentError::GitCommand(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    Ok(wt_path)
}

pub(crate) fn remove_worktree(repo_path: &Path, wt_path: &Path) -> crate::error::Result<()> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["worktree", "remove", &wt_path.to_string_lossy()])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        return Err(DirigentError::GitCommand(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    Ok(())
}

/// Create a GitHub pull request for the current branch using `gh pr create`.
/// Returns the PR URL on success.
pub(crate) fn create_pull_request(
    repo_path: &Path,
    title: &str,
    body: &str,
    base: &str,
    draft: bool,
) -> crate::error::Result<String> {
    use std::process::Command;

    let mut cmd = Command::new("gh");
    cmd.arg("pr")
        .arg("create")
        .arg("--title")
        .arg(title)
        .arg("--body")
        .arg(body)
        .arg("--base")
        .arg(base);
    if draft {
        cmd.arg("--draft");
    }

    let output = cmd.current_dir(repo_path).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "gh pr create failed: {}",
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.trim().to_string())
}

/// Get the default branch name for the repository (e.g. "main" or "master").
/// Falls back to "main" if detection fails.
pub(crate) fn get_default_branch(repo_path: &Path) -> String {
    use std::process::Command;

    // Try gh api first (most reliable for GitHub repos)
    if let Ok(output) = Command::new("gh")
        .args([
            "repo",
            "view",
            "--json",
            "defaultBranchRef",
            "-q",
            ".defaultBranchRef.name",
        ])
        .current_dir(repo_path)
        .output()
    {
        if output.status.success() {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !branch.is_empty() {
                return branch;
            }
        }
    }

    // Fallback: check refs/remotes/origin/HEAD
    let repo = match Repository::discover(repo_path) {
        Ok(r) => r,
        Err(_) => return "main".to_string(),
    };
    if let Ok(reference) = repo.find_reference("refs/remotes/origin/HEAD") {
        if let Some(target) = reference.symbolic_target() {
            if let Some(branch) = target.strip_prefix("refs/remotes/origin/") {
                return branch.to_string();
            }
        }
    }

    "main".to_string()
}

/// Build a PR body from the commits on the current branch that are ahead of the base branch.
pub(crate) fn build_pr_body(repo_path: &Path, base: &str) -> String {
    use std::process::Command;

    let output = Command::new("git")
        .args(["log", "--oneline", &format!("origin/{}..HEAD", base)])
        .current_dir(repo_path)
        .output();

    let commits = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => String::new(),
    };

    if commits.is_empty() {
        return String::new();
    }

    let bullet_list: String = commits
        .lines()
        .map(|line| format!("- {}", line))
        .collect::<Vec<_>>()
        .join("\n");

    format!("## Changes\n\n{}", bullet_list)
}

/// Returns the path to the main (non-linked) worktree / main repo.
/// The first entry in `git worktree list` is always the main worktree.
pub(crate) fn main_worktree_path(repo_path: &Path) -> crate::error::Result<PathBuf> {
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
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("worktree ") {
            return Ok(PathBuf::from(rest));
        }
    }

    Err(DirigentError::GitCommand(
        "no main worktree found".into(),
    ))
}

/// Archives the worktree's .Dirigent/Dirigent.db to the main repo's
/// .Dirigent/archives/<branch-name>.db before removal.
/// Returns Ok(Some(archive_path)) if archived, Ok(None) if no DB existed.
pub(crate) fn archive_worktree_db(
    main_repo_path: &Path,
    worktree_path: &Path,
    worktree_name: &str,
) -> crate::error::Result<Option<PathBuf>> {
    let src_db = worktree_path.join(".Dirigent").join("Dirigent.db");
    if !src_db.exists() {
        return Ok(None);
    }

    let archives_dir = main_repo_path.join(".Dirigent").join("archives");
    std::fs::create_dir_all(&archives_dir).map_err(|e| {
        DirigentError::GitCommand(format!("failed to create archives dir: {}", e))
    })?;

    // Sanitize worktree name for use as filename (replace path separators)
    let safe_name = worktree_name.replace(['/', '\\'], "-");

    let mut target = archives_dir.join(format!("{}.db", safe_name));
    if target.exists() {
        // Append UTC timestamp to avoid collision
        let now = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S");
        target = archives_dir.join(format!("{}_{}.db", safe_name, now));
    }

    std::fs::copy(&src_db, &target).map_err(|e| {
        DirigentError::GitCommand(format!("failed to archive worktree DB: {}", e))
    })?;

    Ok(Some(target))
}

/// Archived worktree DB entry.
#[derive(Debug, Clone)]
pub(crate) struct ArchivedDb {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub modified: std::time::SystemTime,
}

/// Lists all archived worktree DBs in <main_repo>/.Dirigent/archives/.
pub(crate) fn list_archived_dbs(main_repo_path: &Path) -> Vec<ArchivedDb> {
    let archives_dir = main_repo_path.join(".Dirigent").join("archives");
    let entries = match std::fs::read_dir(&archives_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut result = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("db") {
            if let Ok(meta) = entry.metadata() {
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                result.push(ArchivedDb {
                    name,
                    path,
                    size_bytes: meta.len(),
                    modified,
                });
            }
        }
    }
    // Sort by modified time, newest first
    result.sort_by(|a, b| b.modified.cmp(&a.modified));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- format_status_summary --

    #[test]
    fn format_status_summary_basic() {
        let info = GitInfo {
            branch: "main".to_string(),
            last_commit_hash: "abc1234".to_string(),
            last_commit_message: "init".to_string(),
            modified_count: 3,
            added_count: 1,
            deleted_count: 2,
        };
        assert_eq!(format_status_summary(&info), "~3 +1 -2");
    }

    #[test]
    fn format_status_summary_zeros() {
        let info = GitInfo {
            branch: "main".to_string(),
            last_commit_hash: String::new(),
            last_commit_message: String::new(),
            modified_count: 0,
            added_count: 0,
            deleted_count: 0,
        };
        assert_eq!(format_status_summary(&info), "~0 +0 -0");
    }

    // -- generate_commit_message --

    #[test]
    fn generate_commit_message_short() {
        let msg = generate_commit_message("Fix typo");
        assert_eq!(msg, "Dirigent: Fix typo");
    }

    #[test]
    fn generate_commit_message_long_truncates() {
        let long_text = "A".repeat(100);
        let msg = generate_commit_message(&long_text);
        // Should have truncated summary with "..." and full body
        assert!(msg.starts_with("Dirigent: "));
        assert!(msg.contains("..."));
        assert!(msg.contains(&long_text));
    }

    #[test]
    fn generate_commit_message_boundary_68() {
        let exactly_68 = "B".repeat(68);
        let msg = generate_commit_message(&exactly_68);
        assert_eq!(msg, format!("Dirigent: {}", exactly_68));
        assert!(!msg.contains("..."));
    }

    // -- parse_diff_file_paths_for_repo --

    #[test]
    fn parse_diff_file_paths_simple() {
        let diff = "\
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
 fn main() {}
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,1 +1,1 @@
-old
+new
";
        let paths = parse_diff_file_paths_for_repo(Path::new("/tmp/myproject"), diff);
        assert_eq!(paths, vec!["src/main.rs", "src/lib.rs"]);
    }

    #[test]
    fn parse_diff_file_paths_strips_repo_prefix() {
        let diff = "\
--- a/myproject/src/app.rs
+++ b/myproject/src/app.rs
@@ -1,1 +1,1 @@
-x
+y
";
        let paths = parse_diff_file_paths_for_repo(Path::new("/home/user/myproject"), diff);
        assert_eq!(paths, vec!["src/app.rs"]);
    }

    #[test]
    fn parse_diff_file_paths_no_duplicates() {
        let diff = "\
--- a/f.rs
+++ b/f.rs
@@ -1,1 +1,1 @@
-a
+b
--- a/f.rs
+++ b/f.rs
@@ -10,1 +10,1 @@
-c
+d
";
        let paths = parse_diff_file_paths_for_repo(Path::new("/tmp/proj"), diff);
        assert_eq!(paths, vec!["f.rs"]);
    }

    #[test]
    fn parse_diff_file_paths_empty_diff() {
        let paths = parse_diff_file_paths_for_repo(Path::new("/tmp/proj"), "");
        assert!(paths.is_empty());
    }

    // -- read_git_info with a temp repo --

    #[test]
    fn read_git_info_on_temp_repo() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();

        // Create an initial commit so HEAD exists
        let sig = Signature::now("Test", "test@test.com").unwrap();
        let tree_id = {
            let mut index = repo.index().unwrap();
            let file_path = dir.path().join("hello.txt");
            std::fs::write(&file_path, "hello").unwrap();
            index.add_path(std::path::Path::new("hello.txt")).unwrap();
            index.write().unwrap();
            index.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial commit", &tree, &[])
            .unwrap();

        let info = read_git_info(dir.path()).expect("should read git info");
        assert!(
            info.branch == "main" || info.branch == "master",
            "branch should be main or master, got: {}",
            info.branch
        );
        assert_eq!(info.last_commit_message, "initial commit");
        assert_eq!(info.last_commit_hash.len(), 7);
        assert_eq!(info.modified_count, 0);
        assert_eq!(info.added_count, 0);
        assert_eq!(info.deleted_count, 0);
    }

    #[test]
    fn read_git_info_nonexistent_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_git_info(dir.path()).is_none());
    }

    #[test]
    fn get_dirty_files_detects_changes() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();

        let sig = Signature::now("Test", "test@test.com").unwrap();
        let tree_id = {
            let mut index = repo.index().unwrap();
            std::fs::write(dir.path().join("tracked.txt"), "v1").unwrap();
            index.add_path(std::path::Path::new("tracked.txt")).unwrap();
            index.write().unwrap();
            index.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();

        // Modify the tracked file and add an untracked file
        std::fs::write(dir.path().join("tracked.txt"), "v2").unwrap();
        std::fs::write(dir.path().join("new.txt"), "new").unwrap();

        let dirty = get_dirty_files(dir.path());
        assert_eq!(dirty.get("tracked.txt"), Some(&'M'));
        assert_eq!(dirty.get("new.txt"), Some(&'?'));
    }
}
