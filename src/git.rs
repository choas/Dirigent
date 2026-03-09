use git2::{Repository, Signature, StatusOptions};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct GitInfo {
    pub branch: String,
    pub last_commit_hash: String,
    pub last_commit_message: String,
    pub modified_count: usize,
    pub added_count: usize,
    pub deleted_count: usize,
}

pub fn read_git_info(path: &Path) -> Option<GitInfo> {
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

pub fn format_status_summary(info: &GitInfo) -> String {
    format!(
        "~{} +{} -{}",
        info.modified_count, info.added_count, info.deleted_count
    )
}

// -- Commit history --

#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub full_hash: String,
    pub short_hash: String,
    pub message: String,
    pub author: String,
    pub time_ago: String,
}

pub fn read_commit_history(path: &Path, limit: usize) -> Vec<CommitInfo> {
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
        let message = commit
            .message()
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("")
            .to_string();
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
            author,
            time_ago,
        });
    }
    commits
}

pub fn get_commit_diff(path: &Path, commit_hash: &str) -> Option<String> {
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

#[derive(Debug)]
pub enum ApplyError {
    SpawnFailed(std::io::Error),
    ApplyFailed { stderr: String },
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::SpawnFailed(e) => write!(f, "failed to spawn git apply: {e}"),
            ApplyError::ApplyFailed { stderr } => write!(f, "git apply failed: {stderr}"),
        }
    }
}

/// Get the working-tree diff for specific files (or all files if empty).
/// Returns the `git diff` output, or None if there are no changes.
pub fn get_working_diff(repo_path: &Path, files: &[String]) -> Option<String> {
    use std::process::Command;

    let mut cmd = Command::new("git");
    cmd.arg("diff").current_dir(repo_path);
    if !files.is_empty() {
        cmd.arg("--");
        for f in files {
            // Make paths relative to repo root if they're absolute
            let path = Path::new(f);
            let rel = if path.is_absolute() {
                path.strip_prefix(repo_path)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string()
            } else {
                f.clone()
            };
            cmd.arg(rel);
        }
    }

    let output = cmd.output().ok()?;
    if output.status.success() {
        let diff = String::from_utf8_lossy(&output.stdout).to_string();
        if diff.trim().is_empty() {
            None
        } else {
            Some(diff)
        }
    } else {
        None
    }
}

pub fn apply_diff(repo_path: &Path, diff_text: &str) -> Result<(), ApplyError> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // Strip path prefixes that don't match the working directory.
    // Claude may generate diffs with paths like "Dirigent-egui/src/foo.rs"
    // when we're already inside "Dirigent-egui/".
    let fixed_diff = fix_diff_paths(repo_path, diff_text);

    // Try progressively more lenient apply strategies
    let strategies: &[&[&str]] = &[
        &["apply", "--allow-empty", "-"],
        &["apply", "--allow-empty", "--whitespace=fix", "-"],
        &["apply", "--allow-empty", "--whitespace=fix", "-C1", "-"],
    ];

    let mut last_err = String::new();
    for args in strategies {
        let mut child = Command::new("git")
            .args(*args)
            .current_dir(repo_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(ApplyError::SpawnFailed)?;

        if let Some(ref mut stdin) = child.stdin {
            let _ = stdin.write_all(fixed_diff.as_bytes());
        }
        drop(child.stdin.take());

        let output = child.wait_with_output().map_err(ApplyError::SpawnFailed)?;

        if output.status.success() {
            return Ok(());
        }
        last_err = String::from_utf8_lossy(&output.stderr).to_string();
    }

    Err(ApplyError::ApplyFailed { stderr: last_err })
}

/// Fix diff paths when Claude generates paths relative to a parent directory.
/// E.g. "--- a/Dirigent-egui/src/app.rs" when cwd is already "Dirigent-egui/"
/// becomes "--- a/src/app.rs".
fn fix_diff_paths(repo_path: &Path, diff_text: &str) -> String {
    // Get the repo directory name to detect prefix issues
    let dir_name = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    if dir_name.is_empty() {
        return diff_text.to_string();
    }

    let dir_slash = format!("{}/", dir_name);

    let mut result = String::with_capacity(diff_text.len());
    for line in diff_text.lines() {
        if line.starts_with("--- a/") || line.starts_with("+++ b/") {
            // "--- a/Dirigent-egui/src/foo.rs" -> "--- a/src/foo.rs"
            let (prefix, rest) = line.split_at(6); // "--- a/" or "+++ b/"
            if let Some(stripped) = rest.strip_prefix(&dir_slash) {
                result.push_str(prefix);
                result.push_str(stripped);
            } else {
                result.push_str(line);
            }
        } else if line.starts_with("diff --git ") {
            // "diff --git a/Dirigent-egui/src/foo.rs b/Dirigent-egui/src/foo.rs"
            let fixed = line
                .replace(&format!("a/{}", dir_slash), "a/")
                .replace(&format!("b/{}", dir_slash), "b/");
            result.push_str(&fixed);
        } else {
            result.push_str(line);
        }
        result.push('\n');
    }
    result
}

pub fn parse_diff_file_paths(diff_text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in diff_text.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            let path = rest.trim().to_string();
            if !path.is_empty() && !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

/// Like parse_diff_file_paths but also strips a directory prefix if present.
/// Use this when the diff may have been generated with paths relative to a parent dir.
pub fn parse_diff_file_paths_for_repo(repo_path: &Path, diff_text: &str) -> Vec<String> {
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

pub fn stage_and_commit(
    repo_path: &Path,
    file_paths: &[String],
    commit_message: &str,
) -> Result<String, String> {
    let repo = Repository::discover(repo_path).map_err(|e| format!("not a repo: {e}"))?;

    if file_paths.is_empty() {
        return Err("no files to stage".to_string());
    }

    let mut index = repo.index().map_err(|e| e.to_string())?;
    for file_path in file_paths {
        let p = Path::new(file_path);
        let full_path = repo.workdir().unwrap_or(repo_path).join(p);
        if full_path.exists() {
            index.add_path(p).map_err(|e| e.to_string())?;
        } else {
            index.remove_path(p).map_err(|e| e.to_string())?;
        }
    }
    index.write().map_err(|e| e.to_string())?;

    let tree_id = index.write_tree().map_err(|e| e.to_string())?;
    let tree = repo.find_tree(tree_id).map_err(|e| e.to_string())?;

    let sig = repo
        .signature()
        .unwrap_or_else(|_| Signature::now("Dirigent", "Dirigent@local").unwrap());

    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let parents: Vec<&git2::Commit> = parent.iter().collect();

    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, commit_message, &tree, &parents)
        .map_err(|e| e.to_string())?;

    Ok(format!("{}", oid))
}

/// Commit only the changes described by `diff_text`, not the full working tree state.
/// Uses `git apply --cached` so each cue commits exactly its own diff.
pub fn commit_diff(
    repo_path: &Path,
    diff_text: &str,
    commit_message: &str,
) -> Result<String, String> {
    use std::io::Write;
    use std::process::Command;

    // Apply the stored diff to the index only (--cached), leaving working tree untouched
    let mut child = Command::new("git")
        .args(["apply", "--cached", "--allow-empty"])
        .current_dir(repo_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run git apply: {e}"))?;

    child
        .stdin
        .take()
        .unwrap()
        .write_all(diff_text.as_bytes())
        .map_err(|e| format!("failed to pipe diff: {e}"))?;

    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git apply --cached failed: {stderr}"));
    }

    // Now commit whatever is staged
    let repo = Repository::discover(repo_path).map_err(|e| format!("not a repo: {e}"))?;
    let mut index = repo.index().map_err(|e| e.to_string())?;
    let tree_id = index.write_tree().map_err(|e| e.to_string())?;
    let tree = repo.find_tree(tree_id).map_err(|e| e.to_string())?;

    let sig = repo
        .signature()
        .unwrap_or_else(|_| Signature::now("Dirigent", "Dirigent@local").unwrap());

    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());

    // Nothing changed in the index
    if let Some(ref parent_commit) = parent {
        if parent_commit.tree_id() == tree_id {
            return Err("nothing to commit — diff already applied".to_string());
        }
    }

    let parents: Vec<&git2::Commit> = parent.iter().collect();

    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, commit_message, &tree, &parents)
        .map_err(|e| e.to_string())?;

    // Reset the index back to HEAD so it doesn't stay staged
    let head_commit = repo
        .head()
        .map_err(|e| e.to_string())?
        .peel_to_commit()
        .map_err(|e| e.to_string())?;
    repo.reset(head_commit.as_object(), git2::ResetType::Mixed, None)
        .map_err(|e| e.to_string())?;

    Ok(format!("{}", oid))
}

pub fn revert_files(repo_path: &Path, file_paths: &[String]) -> Result<(), String> {
    use std::process::Command;

    if file_paths.is_empty() {
        return Ok(());
    }

    let mut cmd = Command::new("git");
    cmd.arg("checkout").arg("--");
    for f in file_paths {
        cmd.arg(f);
    }
    let output = cmd
        .current_dir(repo_path)
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    Ok(())
}

pub fn generate_commit_message(cue_text: &str) -> String {
    let summary = if cue_text.len() > 68 {
        format!("{}...", &cue_text[..65])
    } else {
        cue_text.to_string()
    };
    format!("Dirigent: {}", summary)
}

// -- Worktree support --

#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    pub name: String,
    pub path: PathBuf,
    pub is_current: bool,
    pub is_locked: bool,
}

pub fn list_worktrees(repo_path: &Path) -> Result<Vec<WorktreeInfo>, String> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
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
                    let canon_wt =
                        std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
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

pub fn create_worktree(repo_path: &Path, name: &str) -> Result<PathBuf, String> {
    use std::process::Command;

    let repo = Repository::discover(repo_path).map_err(|e| e.to_string())?;
    let workdir = repo.workdir().ok_or("no workdir")?.to_path_buf();
    let parent = workdir.parent().ok_or("no parent directory")?;
    let wt_path = parent.join(name);

    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            "-b",
            name,
            &wt_path.to_string_lossy(),
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    Ok(wt_path)
}

pub fn remove_worktree(repo_path: &Path, wt_path: &Path) -> Result<(), String> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["worktree", "remove", &wt_path.to_string_lossy()])
        .current_dir(repo_path)
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    Ok(())
}
