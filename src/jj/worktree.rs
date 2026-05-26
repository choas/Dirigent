use std::path::{Path, PathBuf};

use crate::error::DirigentError;
use crate::git::WorktreeInfo;

/// List jj workspaces (equivalent to `git worktree list`).
pub(crate) fn jj_list_workspaces(
    repo_path: &Path,
    jj_path: &str,
) -> crate::error::Result<Vec<WorktreeInfo>> {
    let output = super::jj_cmd(jj_path)
        .args(["workspace", "list", "--color", "never"])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        return Err(DirigentError::JjCommand(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let current = std::fs::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    let mut workspaces = Vec::new();

    for (i, line) in stdout.lines().enumerate() {
        // Format: "workspace_name: change_id description"
        let name = line.split(':').next().unwrap_or(line).trim().to_string();

        if name.is_empty() {
            continue;
        }

        // For the default workspace, use the repo_path; for others, derive the
        // sibling directory the same way jj_create_workspace builds it.
        let ws_path = if name == "default" {
            repo_path.to_path_buf()
        } else {
            let dir_name = name.rsplit('/').next().unwrap_or(&name);
            repo_path
                .parent()
                .map(|p| p.join(dir_name))
                .unwrap_or_else(|| repo_path.join(dir_name))
        };

        let canon_ws = std::fs::canonicalize(&ws_path).unwrap_or_else(|_| ws_path.clone());
        let is_current = canon_ws == current || current.starts_with(&canon_ws);

        workspaces.push(WorktreeInfo {
            name,
            path: ws_path,
            is_current,
            is_locked: false,
            is_main: i == 0,
        });
    }

    Ok(workspaces)
}

/// Create a new jj workspace (equivalent to `git worktree add`).
pub(crate) fn jj_create_workspace(
    repo_path: &Path,
    name: &str,
    jj_path: &str,
) -> crate::error::Result<PathBuf> {
    let parent = repo_path
        .parent()
        .ok_or_else(|| DirigentError::JjCommand("no parent directory".into()))?;

    let dir_name = name.rsplit('/').next().unwrap_or(name);
    let ws_path = parent.join(dir_name);

    let output = super::jj_cmd(jj_path)
        .args([
            "workspace",
            "add",
            "--name",
            name,
            &ws_path.to_string_lossy(),
        ])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        return Err(DirigentError::JjCommand(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    // Copy .claude/ settings
    let src_claude = repo_path.join(".claude");
    if src_claude.is_dir() {
        let dst_claude = ws_path.join(".claude");
        if let Err(e) = copy_dir_recursive(&src_claude, &dst_claude, &["worktrees"]) {
            eprintln!("warning: failed to copy .claude/ settings to workspace: {e}");
        }
    }

    Ok(ws_path)
}

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

/// Remove a jj workspace (equivalent to `git worktree remove`).
pub(crate) fn jj_remove_workspace(
    repo_path: &Path,
    ws_name: &str,
    jj_path: &str,
) -> crate::error::Result<()> {
    let output = super::jj_cmd(jj_path)
        .args(["workspace", "forget", ws_name])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        return Err(DirigentError::JjCommand(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    Ok(())
}

/// Switch to a bookmark (equivalent to `git checkout <branch>`).
pub(crate) fn jj_checkout_bookmark(
    repo_path: &Path,
    bookmark: &str,
    jj_path: &str,
) -> crate::error::Result<()> {
    // Use `jj new` instead of `jj edit`: it creates a fresh working-copy
    // change on top of the bookmark and cleanly checks out the target tree,
    // matching the behaviour of `git switch`.
    let output = super::jj_cmd(jj_path)
        .args(["new", bookmark])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        return Err(DirigentError::JjCommand(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(())
}

fn cue_slug(cue_text: &str) -> String {
    let raw: String = cue_text
        .chars()
        .take(60)
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let parts: Vec<&str> = raw.split('-').filter(|s| !s.is_empty()).collect();
    let joined = parts.join("-");
    if joined.len() > 30 {
        let end = joined[..30]
            .rfind('-')
            .unwrap_or(joined.floor_char_boundary(30));
        joined[..end].to_string()
    } else {
        joined
    }
}

/// Generate a jj bookmark name for a cue, e.g. `cue/42-add-authentication`.
pub(crate) fn cue_bookmark_name(cue_id: i64, cue_text: &str) -> String {
    let slug = cue_slug(cue_text);
    if slug.is_empty() {
        format!("cue/{}", cue_id)
    } else {
        format!("cue/{}-{}", cue_id, slug)
    }
}

/// Push status for a jj bookmark relative to its remote tracking branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BookmarkPushStatus {
    /// Bookmark exists at @origin and matches the local version.
    Synced,
    /// Bookmark has no @origin tracking branch (local-only or only @git).
    NotPushed,
}

/// A bookmark with its name and remote push status.
#[derive(Clone, Debug)]
pub(crate) struct BookmarkInfo {
    pub(crate) name: String,
    pub(crate) push_status: BookmarkPushStatus,
}

/// List available bookmarks (equivalent to `git branch` list).
pub(crate) fn jj_list_bookmarks(
    repo_path: &Path,
    jj_path: &str,
) -> crate::error::Result<Vec<String>> {
    Ok(jj_list_bookmarks_with_status(repo_path, jj_path)?
        .into_iter()
        .map(|b| b.name)
        .collect())
}

/// List bookmarks with their remote push status.
pub(crate) fn jj_list_bookmarks_with_status(
    repo_path: &Path,
    jj_path: &str,
) -> crate::error::Result<Vec<BookmarkInfo>> {
    let output = super::jj_cmd(jj_path)
        .args(["bookmark", "list", "--all-remotes", "--color", "never"])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::JjCommand(format!(
            "jj bookmark list failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut local_names: Vec<String> = Vec::new();
    let mut has_origin: std::collections::HashSet<String> = std::collections::HashSet::new();

    for line in stdout.lines() {
        let name = match line.split(':').next() {
            Some(n) => n.trim(),
            None => continue,
        };
        if name.is_empty() {
            continue;
        }

        if let Some(at_pos) = name.find('@') {
            let base = &name[..at_pos];
            let remote = &name[at_pos + 1..];
            if remote == "origin" {
                has_origin.insert(base.to_string());
            }
        } else {
            local_names.push(name.to_string());
        }
    }

    let bookmarks = local_names
        .into_iter()
        .map(|name| {
            let push_status = if has_origin.contains(&name) {
                BookmarkPushStatus::Synced
            } else {
                BookmarkPushStatus::NotPushed
            };
            BookmarkInfo { name, push_status }
        })
        .collect();

    Ok(bookmarks)
}

/// A bookmark flagged as a likely tool artifact.
#[derive(Clone, Debug)]
pub(crate) struct SuspiciousBookmark {
    pub name: String,
    pub reason: String,
    /// The decoded name if the corruption is character-doubling.
    pub decoded: Option<String>,
}

/// Check whether every character in `name` appears exactly twice in sequence
/// (e.g. "ffiixx" → "fix"). Returns the decoded string if so.
fn decode_doubled(name: &str) -> Option<String> {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() < 4 || chars.len() % 2 != 0 {
        return None;
    }
    let mut decoded = String::with_capacity(chars.len() / 2);
    let mut i = 0;
    while i < chars.len() {
        if i + 1 < chars.len() && chars[i] == chars[i + 1] {
            decoded.push(chars[i]);
            i += 2;
        } else {
            return None;
        }
    }
    Some(decoded)
}

const ARTIFACT_PREFIXES: &[(&str, &str)] = &[
    ("gitbutler/", "GitButler internal bookmark"),
    ("butler/", "GitButler internal bookmark"),
];

const MAX_SANE_BOOKMARK_LEN: usize = 80;

/// Classify a single bookmark name as suspicious or not.
pub(crate) fn check_bookmark(name: &str) -> Option<SuspiciousBookmark> {
    // 1. Character-doubling (GitButler artifact)
    if let Some(decoded) = decode_doubled(name) {
        return Some(SuspiciousBookmark {
            name: name.to_string(),
            reason: format!("doubled characters (likely \"{}\")", decoded),
            decoded: Some(decoded),
        });
    }

    // 2. Known tool prefixes
    for &(prefix, label) in ARTIFACT_PREFIXES {
        if name.starts_with(prefix) {
            return Some(SuspiciousBookmark {
                name: name.to_string(),
                reason: label.to_string(),
                decoded: None,
            });
        }
    }

    // 3. Excessively long names
    if name.len() > MAX_SANE_BOOKMARK_LEN {
        return Some(SuspiciousBookmark {
            name: name.to_string(),
            reason: format!("unusually long ({} chars)", name.len()),
            decoded: None,
        });
    }

    None
}

/// Scan all bookmarks and return those that look like tool artifacts.
pub(crate) fn jj_find_suspicious_bookmarks(
    repo_path: &Path,
    jj_path: &str,
) -> crate::error::Result<Vec<SuspiciousBookmark>> {
    let bookmarks = jj_list_bookmarks(repo_path, jj_path)?;
    Ok(bookmarks.iter().filter_map(|b| check_bookmark(b)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubled_chars_detected() {
        let s = check_bookmark("ffiixx--bbaacckkeenndd--ffoorrmmaattttiinngg");
        assert!(s.is_some());
        let s = s.unwrap();
        assert_eq!(s.decoded.as_deref(), Some("fix-backend-formatting"));
        assert!(s.reason.contains("doubled"));
    }

    #[test]
    fn normal_bookmark_passes() {
        assert!(check_bookmark("main").is_none());
        assert!(check_bookmark("cue/42-add-auth").is_none());
        assert!(check_bookmark("fix-backend-formatting").is_none());
    }

    #[test]
    fn long_bookmark_flagged() {
        let long = "ab".repeat(50);
        let s = check_bookmark(&long);
        assert!(s.is_some());
        assert!(s.unwrap().reason.contains("long"));
    }

    #[test]
    fn gitbutler_prefix_flagged() {
        let s = check_bookmark("gitbutler/integration");
        assert!(s.is_some());
        assert!(s.unwrap().reason.contains("GitButler"));
    }

    #[test]
    fn odd_length_not_doubled() {
        assert!(check_bookmark("abc").is_none());
    }
}
