use std::collections::HashMap;
use std::path::Path;

use git2::{Oid, Repository};

#[derive(Debug, Clone)]
pub(crate) struct CommitInfo {
    pub full_hash: String,
    pub short_hash: String,
    pub message: String,
    pub body: String,
    pub author: String,
    pub time_ago: String,
    pub parent_hashes: Vec<String>,
    pub branch_labels: Vec<String>,
    pub tag_labels: Vec<String>,
    pub is_merge: bool,
}

pub(crate) fn read_commit_history(path: &Path, limit: usize) -> Vec<CommitInfo> {
    let repo = match Repository::discover(path) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    // Build branch label map: Oid -> Vec<branch name>
    let mut branch_map = build_branch_map(&repo);
    // Build tag label map: Oid -> Vec<tag name>
    let tag_map = build_tag_map(&repo);

    let mut revwalk = match repo.revwalk() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    // Topological + time sorting ensures proper graph layout when
    // multiple branch histories are interleaved.
    revwalk
        .set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)
        .ok();

    // Detect detached HEAD and record its OID for labeling.
    let head_detached = repo.head_detached().unwrap_or(false);
    let head_oid = repo
        .head()
        .ok()
        .and_then(|h| h.peel_to_commit().ok())
        .map(|c| c.id());

    // Push HEAD (works for both normal and detached HEAD).
    if let Some(oid) = head_oid {
        revwalk.push(oid).ok();
        // Add "HEAD" label for detached HEAD so the graph shows it.
        if head_detached {
            branch_map
                .entry(oid)
                .or_default()
                .insert(0, "HEAD".to_string());
        }
    }

    // Push all local branch tips to include orphan branches
    // (branches with no common ancestor with HEAD).
    if let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) {
        for branch in branches.flatten() {
            let (branch_ref, _) = branch;
            if let Ok(commit) = branch_ref.get().peel_to_commit() {
                revwalk.push(commit.id()).ok();
            }
        }
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    revwalk
        .take(limit)
        .flatten()
        .filter_map(|oid| commit_to_info(&repo, oid, &branch_map, &tag_map, now))
        .collect()
}

fn commit_to_info(
    repo: &Repository,
    oid: Oid,
    branch_map: &HashMap<Oid, Vec<String>>,
    tag_map: &HashMap<Oid, Vec<String>>,
    now: i64,
) -> Option<CommitInfo> {
    let commit = repo.find_commit(oid).ok()?;
    let hash = format!("{}", commit.id());
    let short_hash = super::short_hash(&hash);
    let full_message = commit.message().unwrap_or("");
    let message = full_message.lines().next().unwrap_or("").to_string();
    let body = full_message.trim().to_string();
    let author = commit.author().name().unwrap_or("").to_string();
    let parent_hashes: Vec<String> = commit.parent_ids().map(|id| format!("{}", id)).collect();
    let is_merge = parent_hashes.len() > 1;
    let branch_labels = branch_map.get(&commit.id()).cloned().unwrap_or_default();
    let tag_labels = tag_map.get(&commit.id()).cloned().unwrap_or_default();
    let time_ago = format_time_ago(now - commit.time().seconds());
    Some(CommitInfo {
        full_hash: hash,
        short_hash,
        message,
        body,
        author,
        time_ago,
        parent_hashes,
        branch_labels,
        tag_labels,
        is_merge,
    })
}

fn format_time_ago(diff: i64) -> String {
    if diff < 0 {
        return "in the future".to_string();
    }
    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

fn build_branch_map(repo: &Repository) -> HashMap<Oid, Vec<String>> {
    let mut map: HashMap<Oid, Vec<String>> = HashMap::new();
    if let Ok(branches) = repo.branches(None) {
        for branch in branches.flatten() {
            let (branch_ref, _branch_type) = branch;
            if let Some(name) = branch_ref.name().ok().flatten() {
                if let Ok(commit) = branch_ref.get().peel_to_commit() {
                    map.entry(commit.id()).or_default().push(name.to_string());
                }
            }
        }
    }
    map
}

fn build_tag_map(repo: &Repository) -> HashMap<Oid, Vec<String>> {
    let mut map: HashMap<Oid, Vec<String>> = HashMap::new();
    if let Ok(tag_names) = repo.tag_names(None) {
        for name in tag_names.iter().flatten() {
            if let Ok(reference) = repo.revparse_single(name) {
                let oid = reference
                    .peel_to_commit()
                    .map(|c| c.id())
                    .unwrap_or_else(|_| reference.id());
                map.entry(oid).or_default().push(name.to_string());
            }
        }
    }
    map
}

pub(crate) fn count_commits(path: &Path) -> usize {
    // Use `git rev-list --count --all` which is much faster than walking
    // the entire history via revwalk (O(1) with pack index vs O(n)).
    use std::process::Command;
    let output = Command::new("git")
        .args(["rev-list", "--count", "--all"])
        .current_dir(path)
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .trim()
            .parse()
            .unwrap_or(0),
        _ => 0,
    }
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
