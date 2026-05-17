use std::path::{Path, PathBuf};

use crate::error::DirigentError;
use crate::git::WorktreeInfo;

/// List jj workspaces (equivalent to `git worktree list`).
pub(crate) fn jj_list_workspaces(
    repo_path: &Path,
    jj_path: &str,
) -> crate::error::Result<Vec<WorktreeInfo>> {
    let output = super::jj_cmd(jj_path)
        .args(["workspace", "list"])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        return Err(DirigentError::GitCommand(
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

        // For the default workspace, use the repo_path; for others, try sibling
        let ws_path = if name == "default" {
            repo_path.to_path_buf()
        } else {
            repo_path
                .parent()
                .map(|p| p.join(&name))
                .unwrap_or_else(|| repo_path.join(&name))
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
        .ok_or_else(|| DirigentError::GitCommand("no parent directory".into()))?;

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
        return Err(DirigentError::GitCommand(
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
        return Err(DirigentError::GitCommand(
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
    let output = super::jj_cmd(jj_path)
        .args(["new", bookmark])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        return Err(DirigentError::GitCommand(
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

/// Generate a jj workspace name for a cue, e.g. `cue-42-add-authentication`.
pub(crate) fn cue_workspace_name(cue_id: i64, cue_text: &str) -> String {
    let slug = cue_slug(cue_text);
    if slug.is_empty() {
        format!("cue-{}", cue_id)
    } else {
        format!("cue-{}-{}", cue_id, slug)
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

/// List available bookmarks (equivalent to `git branch` list).
pub(crate) fn jj_list_bookmarks(
    repo_path: &Path,
    jj_path: &str,
) -> crate::error::Result<Vec<String>> {
    let output = super::jj_cmd(jj_path)
        .args(["bookmark", "list", "--all-remotes"])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let bookmarks: Vec<String> = stdout
        .lines()
        .filter_map(|line| {
            let name = line.split(':').next()?.trim();
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect();

    Ok(bookmarks)
}
