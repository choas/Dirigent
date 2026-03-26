use std::path::Path;

use git2::Repository;

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
