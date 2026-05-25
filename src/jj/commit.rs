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
        return Err(DirigentError::JjCommand(format!(
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
        return Err(DirigentError::JjCommand(format!(
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
/// When `advance_bookmarks` is true, any bookmarks on the parent (`@-` before
/// the commit, which becomes `@--` after) are advanced to the newly committed
/// change (`@-` after the commit). This mimics git's branch-advancement
/// behaviour. Set to false for per-cue workspace commits where only the
/// cue-specific bookmark should track the new commit.
pub(crate) fn jj_commit_all(
    repo_path: &Path,
    commit_message: &str,
    jj_path: &str,
    advance_bookmarks: bool,
) -> crate::error::Result<String> {
    let parent_bookmarks = if advance_bookmarks {
        // Before committing, check whether @ already carries a bookmark.
        // If not, remember the parent's bookmarks so we can advance them.
        let wc_bookmarks = bookmarks_for_rev(repo_path, "@", jj_path);
        if wc_bookmarks.is_empty() {
            bookmarks_for_rev(repo_path, "@-", jj_path)
        } else {
            Vec::new()
        }
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
        return Err(DirigentError::JjCommand(format!(
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

/// Commit only the files referenced in `diff_text`.
///
/// In jj every edit is already part of the working-copy commit (`@`).
/// To commit a subset we temporarily revert the *other* dirty files to
/// their parent state, commit, then write the originals back so they
/// reappear in the new `@`.
pub(crate) fn jj_commit_diff(
    repo_path: &Path,
    diff_text: &str,
    commit_message: &str,
    jj_path: &str,
) -> crate::error::Result<String> {
    let diff_files = crate::git::parse_diff_file_paths_for_repo(repo_path, diff_text);

    if diff_files.is_empty() {
        return Err(DirigentError::JjCommand(
            "no files to commit — diff contains no file paths".into(),
        ));
    }

    let dirty = super::status::jj_get_dirty_files(repo_path, jj_path);
    let other_files: Vec<String> = dirty
        .keys()
        .filter(|f| !diff_files.contains(f))
        .cloned()
        .collect();

    if other_files.is_empty() {
        return jj_commit_all(repo_path, commit_message, jj_path, true);
    }

    // Snapshot each non-diff file so we can put it back after the commit.
    // `None` means the file was deleted in the working copy (doesn't exist on disk).
    let saved: Vec<(String, Option<Vec<u8>>)> = other_files
        .iter()
        .map(|f| {
            let content = std::fs::read(repo_path.join(f)).ok();
            (f.clone(), content)
        })
        .collect();

    // Revert non-diff files to parent state so they aren't part of the commit.
    let mut restore_args: Vec<&str> = vec!["restore", "--from", "@-", "--"];
    for f in &other_files {
        restore_args.push(f);
    }
    let _ = super::jj_cmd(jj_path)
        .args(&restore_args)
        .current_dir(repo_path)
        .output();

    let result = jj_commit_all(repo_path, commit_message, jj_path, true);

    // Put the non-diff files back so they remain as pending changes in the new @.
    for (rel, content) in &saved {
        let full = repo_path.join(rel);
        match content {
            Some(data) => {
                if let Some(parent) = full.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&full, data);
            }
            None => {
                let _ = std::fs::remove_file(&full);
            }
        }
    }

    result
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
        return Err(DirigentError::JjCommand(format!(
            "jj bookmark delete failed: {}",
            stderr.trim()
        )));
    }
    Ok(())
}

/// Count how many non-empty commits sit between `trunk()` and a bookmark.
#[allow(dead_code)]
pub(crate) fn jj_bookmark_commit_count(repo_path: &Path, bookmark: &str, jj_path: &str) -> usize {
    let revset = format!("trunk()..{}", bookmark);
    let output = super::jj_cmd(jj_path)
        .args([
            "log",
            "--no-graph",
            "-r",
            &revset,
            "-T",
            r#"if(!empty, change_id ++ "\n")"#,
        ])
        .current_dir(repo_path)
        .output();
    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            stdout.lines().filter(|l| !l.trim().is_empty()).count()
        }
        _ => 0,
    }
}

/// Squash all commits on a bookmark into a single commit.
///
/// Finds all commits between `trunk()` and the bookmark, then squashes each
/// one (from newest to oldest) into its parent, leaving a single commit.
/// Returns the number of commits that were squashed.
pub(crate) fn jj_squash_bookmark(
    repo_path: &Path,
    bookmark: &str,
    jj_path: &str,
) -> crate::error::Result<usize> {
    let revset = format!("trunk()..{}", bookmark);
    let output = super::jj_cmd(jj_path)
        .args([
            "log",
            "--no-graph",
            "-r",
            &revset,
            "-T",
            r#"change_id ++ "\n""#,
        ])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::JjCommand(format!(
            "jj log for squash failed: {}",
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let change_ids: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();

    if change_ids.len() <= 1 {
        return Ok(0);
    }

    // change_ids are newest-first; squash each into its parent, skipping the
    // oldest (which is the target that absorbs everything).
    let mut squashed = 0;
    for cid in &change_ids[..change_ids.len() - 1] {
        let sq_output = super::jj_cmd(jj_path)
            .args(["squash", "-r", cid.trim()])
            .current_dir(repo_path)
            .output()?;

        if !sq_output.status.success() {
            let stderr = String::from_utf8_lossy(&sq_output.stderr);
            return Err(DirigentError::JjCommand(format!(
                "jj squash -r {} failed: {}",
                cid.trim(),
                stderr.trim()
            )));
        }
        squashed += 1;
    }

    // Move the bookmark to the surviving commit (now the only one in the range).
    let surviving = super::jj_cmd(jj_path)
        .args(["log", "--no-graph", "-r", &revset, "-T", r#"change_id"#])
        .current_dir(repo_path)
        .output()?;
    if surviving.status.success() {
        let rev = String::from_utf8_lossy(&surviving.stdout);
        let rev = rev.trim();
        if !rev.is_empty() {
            let _ = super::jj_cmd(jj_path)
                .args(["bookmark", "set", bookmark, "-r", rev])
                .current_dir(repo_path)
                .output();
        }
    }

    Ok(squashed)
}

/// Create a new bookmark at the current working-copy commit's parent (`@-`).
pub(crate) fn jj_create_bookmark(
    repo_path: &Path,
    name: &str,
    jj_path: &str,
) -> crate::error::Result<()> {
    let output = super::jj_cmd(jj_path)
        .args(["bookmark", "create", name, "-r", "@-"])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::JjCommand(format!(
            "jj bookmark create failed: {}",
            stderr.trim()
        )));
    }
    Ok(())
}

/// Undo the last jj operation via `jj op restore @-`.
pub(crate) fn jj_undo(repo_path: &Path, jj_path: &str) -> crate::error::Result<String> {
    let output = super::jj_cmd(jj_path)
        .args(["op", "restore", "@-"])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::JjCommand(format!(
            "jj op restore failed: {}",
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let summary = stdout.lines().next().unwrap_or("ok").trim();
    Ok(format!("Undo: {}", summary))
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
            .args(["restore", "--from", "@-", "--", path])
            .current_dir(repo_path)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DirigentError::JjCommand(format!(
                "jj restore failed for {}: {}",
                path,
                stderr.trim()
            )));
        }
    }
    Ok(())
}
