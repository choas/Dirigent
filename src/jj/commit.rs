use std::path::Path;

use crate::error::DirigentError;

/// Push the current bookmarks to the remote via `jj git push`.
pub(crate) fn jj_push(repo_path: &Path, jj_path: &str) -> crate::error::Result<String> {
    let output = super::jj_cmd(jj_path)
        .args(["git", "push"])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "jj git push failed: {}",
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let summary = stdout.lines().next().unwrap_or("ok").trim();
    Ok(format!("Pushed ({})", summary))
}

/// Fetch from remote via `jj git fetch`.
pub(crate) fn jj_pull(repo_path: &Path, jj_path: &str) -> crate::error::Result<String> {
    let output = super::jj_cmd(jj_path)
        .args(["git", "fetch"])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "jj git fetch failed: {}",
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let summary = stdout.lines().next().unwrap_or("ok").trim();
    Ok(format!("Fetched ({})", summary))
}

/// Read bookmarks for a given revision (e.g. `@`, `@-`).
fn bookmarks_for_rev(repo_path: &Path, rev: &str, jj_path: &str) -> Vec<String> {
    let output = super::jj_cmd(jj_path)
        .args(["log", "-r", rev, "--no-graph", "-T", "bookmarks"])
        .current_dir(repo_path)
        .output();
    match output {
        Ok(o) if o.status.success() => {
            let raw = String::from_utf8_lossy(&o.stdout);
            raw.split_whitespace()
                .map(|s| s.trim_end_matches('*').to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Describe the current working-copy commit and create a new empty change.
/// This is the jj equivalent of `git add -A && git commit -m "..."`.
///
/// After committing, any bookmarks on the parent (`@-` before the commit,
/// which becomes `@--` after) are advanced to the newly committed change
/// (`@-` after the commit). This mimics git's branch-advancement behaviour.
pub(crate) fn jj_commit_all(
    repo_path: &Path,
    commit_message: &str,
    jj_path: &str,
) -> crate::error::Result<String> {
    // Before committing, check whether @ already carries a bookmark.
    // If not, remember the parent's bookmarks so we can advance them.
    let wc_bookmarks = bookmarks_for_rev(repo_path, "@", jj_path);
    let parent_bookmarks = if wc_bookmarks.is_empty() {
        bookmarks_for_rev(repo_path, "@-", jj_path)
    } else {
        Vec::new()
    };

    // `jj commit` describes the current change and creates a new empty child.
    let output = super::jj_cmd(jj_path)
        .args(["commit", "-m", commit_message])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "jj commit failed: {}",
            stderr.trim()
        )));
    }

    // Get the change ID of the commit we just finalized (now @-)
    let id_output = super::jj_cmd(jj_path)
        .args([
            "log",
            "-r",
            "@-",
            "--no-graph",
            "-T",
            "change_id.shortest(7)",
        ])
        .current_dir(repo_path)
        .output()?;

    let change_id = if id_output.status.success() {
        String::from_utf8_lossy(&id_output.stdout)
            .trim()
            .to_string()
    } else {
        "unknown".to_string()
    };

    // Advance the parent's bookmarks to the committed change so the
    // bookmark tracks forward, matching git's branch behaviour.
    for bm in &parent_bookmarks {
        let _ = super::jj_cmd(jj_path)
            .args(["bookmark", "set", bm, "-r", "@-"])
            .current_dir(repo_path)
            .output();
    }

    Ok(change_id)
}

/// Commit specific files from a diff by squashing changes.
/// In jj, all file edits are already part of the working-copy commit,
/// so we describe it and finalize.
pub(crate) fn jj_commit_diff(
    repo_path: &Path,
    _diff_text: &str,
    commit_message: &str,
    jj_path: &str,
) -> crate::error::Result<String> {
    // In jj, all changes are already tracked in the working copy commit.
    // We just describe and finalize.
    jj_commit_all(repo_path, commit_message, jj_path)
}

/// Set (or move) a bookmark on a specific revision.
pub(crate) fn jj_set_bookmark(
    repo_path: &Path,
    name: &str,
    rev: &str,
    jj_path: &str,
) -> crate::error::Result<()> {
    let output = super::jj_cmd(jj_path)
        .args(["bookmark", "set", name, "-r", rev])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "jj bookmark set failed: {}",
            stderr.trim()
        )));
    }
    Ok(())
}

/// Delete a bookmark.
pub(crate) fn jj_delete_bookmark(
    repo_path: &Path,
    name: &str,
    jj_path: &str,
) -> crate::error::Result<()> {
    let output = super::jj_cmd(jj_path)
        .args(["bookmark", "delete", name])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "jj bookmark delete failed: {}",
            stderr.trim()
        )));
    }
    Ok(())
}

/// Revert specific files by restoring them from the parent commit.
pub(crate) fn jj_revert_files(
    repo_path: &Path,
    file_paths: &[String],
    jj_path: &str,
) -> crate::error::Result<()> {
    if file_paths.is_empty() {
        return Ok(());
    }

    for path in file_paths {
        let output = super::jj_cmd(jj_path)
            .args(["restore", "--from", "@-", path])
            .current_dir(repo_path)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DirigentError::GitCommand(format!(
                "jj restore failed for {}: {}",
                path,
                stderr.trim()
            )));
        }
    }
    Ok(())
}
