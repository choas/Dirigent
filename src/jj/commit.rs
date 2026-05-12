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

/// Describe the current working-copy commit and create a new empty change.
/// This is the jj equivalent of `git add -A && git commit -m "..."`.
pub(crate) fn jj_commit_all(
    repo_path: &Path,
    commit_message: &str,
    jj_path: &str,
) -> crate::error::Result<String> {
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
