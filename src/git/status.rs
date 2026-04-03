use std::collections::HashMap;
use std::path::Path;

use git2::{BranchType, Repository, StatusOptions};

#[derive(Debug, Clone)]
pub(crate) struct GitInfo {
    pub branch: String,
    pub last_commit_hash: String,
    pub last_commit_message: String,
    pub modified_count: usize,
    pub added_count: usize,
    pub deleted_count: usize,
    pub conflicted_count: usize,
}

pub(crate) fn read_git_info(path: &Path) -> Option<GitInfo> {
    let repo = Repository::discover(path).ok()?;
    let branch = resolve_branch_name(&repo);
    let (hash, message) = resolve_last_commit(&repo);
    let (modified, added, deleted, conflicted) = count_status_entries(&repo);

    Some(GitInfo {
        branch,
        last_commit_hash: hash,
        last_commit_message: message,
        modified_count: modified,
        added_count: added,
        deleted_count: deleted,
        conflicted_count: conflicted,
    })
}

fn resolve_branch_name(repo: &Repository) -> String {
    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => return "no commits".to_string(),
    };
    if head.is_branch() {
        head.shorthand().unwrap_or("HEAD").to_string()
    } else {
        "HEAD detached".to_string()
    }
}

fn resolve_last_commit(repo: &Repository) -> (String, String) {
    let commit = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let commit = match commit {
        Some(c) => c,
        None => return (String::new(), String::new()),
    };
    let h = format!("{}", commit.id());
    let short = super::short_hash(&h);
    let msg = commit
        .message()
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("")
        .to_string();
    (short, msg)
}

fn count_status_entries(repo: &Repository) -> (usize, usize, usize, usize) {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    let statuses = match repo.statuses(Some(&mut opts)) {
        Ok(s) => s,
        Err(_) => return (0, 0, 0, 0),
    };
    let mut m = 0;
    let mut a = 0;
    let mut d = 0;
    let mut u = 0;
    for entry in statuses.iter() {
        let s = entry.status();
        if s.intersects(git2::Status::CONFLICTED) {
            u += 1;
        } else if s.intersects(
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
    (m, a, d, u)
}

fn status_letter(s: git2::Status) -> Option<char> {
    if s.intersects(git2::Status::CONFLICTED) {
        Some('U')
    } else if s.intersects(git2::Status::WT_DELETED | git2::Status::INDEX_DELETED) {
        Some('D')
    } else if s.intersects(git2::Status::WT_RENAMED | git2::Status::INDEX_RENAMED) {
        Some('R')
    } else if s.intersects(git2::Status::WT_MODIFIED | git2::Status::INDEX_MODIFIED) {
        Some('M')
    } else if s.intersects(git2::Status::INDEX_NEW) {
        Some('A')
    } else if s.intersects(git2::Status::WT_NEW) {
        Some('?')
    } else {
        None
    }
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
    let statuses = match repo.statuses(Some(&mut opts)) {
        Ok(s) => s,
        Err(_) => return result,
    };
    for entry in statuses.iter() {
        if let (Some(letter), Some(p)) = (status_letter(entry.status()), entry.path()) {
            result.insert(p.to_string(), letter);
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
    // Try to resolve the branch's configured upstream tracking branch
    let remote_oid = repo
        .find_branch(&branch_name, BranchType::Local)
        .ok()
        .and_then(|branch| branch.upstream().ok())
        .and_then(|upstream| upstream.get().target());
    let remote_oid = match remote_oid {
        Some(oid) => oid,
        None => {
            // No configured upstream — compare against origin's default branch
            let default_oid = repo
                .find_reference("refs/remotes/origin/HEAD")
                .and_then(|r| r.resolve())
                .and_then(|r| {
                    r.target()
                        .ok_or_else(|| git2::Error::from_str("symbolic ref has no target"))
                })
                .or_else(|_| repo.refname_to_id("refs/remotes/origin/main"))
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
    let base = format!(
        "~{} +{} -{}",
        info.modified_count, info.added_count, info.deleted_count
    );
    if info.conflicted_count > 0 {
        format!("{} U{}", base, info.conflicted_count)
    } else {
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Signature;

    #[test]
    fn format_status_summary_basic() {
        let info = GitInfo {
            branch: "main".to_string(),
            last_commit_hash: "abc1234".to_string(),
            last_commit_message: "init".to_string(),
            modified_count: 3,
            added_count: 1,
            deleted_count: 2,
            conflicted_count: 0,
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
            conflicted_count: 0,
        };
        assert_eq!(format_status_summary(&info), "~0 +0 -0");
    }

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
        assert_eq!(info.conflicted_count, 0);
    }

    #[test]
    fn format_status_summary_with_conflicts() {
        let info = GitInfo {
            branch: "main".to_string(),
            last_commit_hash: "abc1234".to_string(),
            last_commit_message: "merge".to_string(),
            modified_count: 0,
            added_count: 0,
            deleted_count: 0,
            conflicted_count: 2,
        };
        assert_eq!(format_status_summary(&info), "~0 +0 -0 U2");
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
