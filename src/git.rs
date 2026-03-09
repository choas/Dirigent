use std::collections::HashSet;

use git2::{Repository, Signature, StatusOptions};
use std::path::{Path, PathBuf};

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

/// Returns relative paths of all files with uncommitted changes (modified, new, deleted, staged).
pub(crate) fn get_dirty_files(path: &Path) -> HashSet<String> {
    let mut result = HashSet::new();
    let repo = match Repository::discover(path) {
        Ok(r) => r,
        Err(_) => return result,
    };
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    if let Ok(statuses) = repo.statuses(Some(&mut opts)) {
        for entry in statuses.iter() {
            let s = entry.status();
            if s.intersects(
                git2::Status::WT_MODIFIED
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::WT_NEW
                    | git2::Status::INDEX_NEW
                    | git2::Status::WT_DELETED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::WT_RENAMED
                    | git2::Status::INDEX_RENAMED,
            ) {
                if let Some(p) = entry.path() {
                    result.insert(p.to_string());
                }
            }
        }
    }
    result
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
        let message = full_message
            .lines()
            .next()
            .unwrap_or("")
            .to_string();
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
pub(crate) fn get_working_diff(repo_path: &Path, files: &[String]) -> Option<String> {
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

/// Commit only the changes described by `diff_text`, not the full working tree state.
/// Uses `git apply --cached` so each cue commits exactly its own diff.
pub(crate) fn commit_diff(
    repo_path: &Path,
    diff_text: &str,
    commit_message: &str,
) -> Result<String, String> {
    use std::io::Write;
    use std::process::Command;

    // Dry-run: validate the diff before applying
    let mut check = Command::new("git")
        .args(["apply", "--cached", "--check"])
        .current_dir(repo_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run git apply --check: {e}"))?;

    check
        .stdin
        .take()
        .unwrap()
        .write_all(diff_text.as_bytes())
        .map_err(|e| format!("failed to pipe diff: {e}"))?;

    let check_output = check.wait_with_output().map_err(|e| e.to_string())?;
    if !check_output.status.success() {
        let stderr = String::from_utf8_lossy(&check_output.stderr);
        return Err(format!("diff validation failed: {stderr}"));
    }

    // Apply the diff to the index only (--cached), leaving working tree untouched
    let mut child = Command::new("git")
        .args(["apply", "--cached"])
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

pub(crate) fn revert_files(repo_path: &Path, file_paths: &[String]) -> Result<(), String> {
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

pub(crate) fn generate_commit_message(cue_text: &str) -> String {
    let summary = if cue_text.len() > 68 {
        format!("{}...", &cue_text[..65])
    } else {
        cue_text.to_string()
    };
    if cue_text.len() > 68 {
        format!("Dirigent: {}\n\n{}", summary, cue_text)
    } else {
        format!("Dirigent: {}", summary)
    }
}

// -- Worktree support --

#[derive(Debug, Clone)]
pub(crate) struct WorktreeInfo {
    pub name: String,
    pub path: PathBuf,
    pub is_current: bool,
    pub is_locked: bool,
}

pub(crate) fn list_worktrees(repo_path: &Path) -> Result<Vec<WorktreeInfo>, String> {
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

pub(crate) fn create_worktree(repo_path: &Path, name: &str) -> Result<PathBuf, String> {
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

pub(crate) fn remove_worktree(repo_path: &Path, wt_path: &Path) -> Result<(), String> {
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
            index
                .add_path(std::path::Path::new("hello.txt"))
                .unwrap();
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
            index
                .add_path(std::path::Path::new("tracked.txt"))
                .unwrap();
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
        assert!(dirty.contains("tracked.txt"));
        assert!(dirty.contains("new.txt"));
    }
}
