use std::path::Path;
use std::process::Output;

use crate::error::DirigentError;

fn is_stale_working_copy(output: &Output) -> bool {
    if output.status.success() {
        return false;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    stderr.contains("working copy is stale")
}

/// Detect the rejection `jj git push` emits when a commit being pushed — or any
/// of its ancestors — still contains a conflict. The tip bookmark can look
/// perfectly resolved while an earlier commit in its history is conflicted, so
/// the bare jj message ("Won't push commit <id> since it has conflicts") leaves
/// the user staring at a clean tip with no idea where the conflict lives.
fn is_conflicted_commit_rejection(stderr: &str) -> bool {
    stderr.contains("since it has conflicts")
        || (stderr.contains("Won't push commit") && stderr.contains("conflicts"))
}

/// Build a friendly error for the conflicted-ancestor push rejection that points
/// the user at the exact commits to resolve.
fn conflicted_ancestor_message(repo_path: &Path, jj_path: &str, stderr: &str) -> String {
    let bookmark = bookmarks_for_rev(repo_path, "@-", jj_path)
        .into_iter()
        .next()
        .unwrap_or_else(|| "<bookmark>".to_string());
    format!(
        "jj git push refused: a commit in the history still has conflicts.\n\n\
         The tip bookmark may look resolved, but jj won't push while any ancestor \
         commit is conflicted. List the conflicted commits to resolve with:\n\n  \
         jj log -r 'ancestors({bookmark}) & conflicts()'\n\n\
         jj reported:\n{}",
        stderr.trim()
    )
}

fn update_stale_working_copy(repo_path: &Path, jj_path: &str) -> crate::error::Result<()> {
    let output = super::jj_cmd(jj_path)
        .args(["workspace", "update-stale"])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::JjCommand(format!(
            "jj workspace update-stale failed: {}",
            stderr.trim()
        )));
    }
    Ok(())
}

/// Push the current bookmarks to the remote via `jj git push`.
pub(crate) fn jj_push(repo_path: &Path, jj_path: &str) -> crate::error::Result<String> {
    let output = super::jj_cmd(jj_path)
        .args(["git", "push"])
        .current_dir(repo_path)
        .output()?;

    if is_stale_working_copy(&output) {
        update_stale_working_copy(repo_path, jj_path)?;
        let output = super::jj_cmd(jj_path)
            .args(["git", "push"])
            .current_dir(repo_path)
            .output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if is_conflicted_commit_rejection(&stderr) {
                return Err(DirigentError::JjCommand(conflicted_ancestor_message(
                    repo_path, jj_path, &stderr,
                )));
            }
            return Err(DirigentError::JjCommand(format!(
                "jj git push failed: {}",
                stderr.trim()
            )));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let summary = stdout.lines().next().unwrap_or("ok").trim();
        return Ok(format!("Pushed ({})", summary));
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if is_conflicted_commit_rejection(&stderr) {
            return Err(DirigentError::JjCommand(conflicted_ancestor_message(
                repo_path, jj_path, &stderr,
            )));
        }
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

    if is_stale_working_copy(&output) {
        update_stale_working_copy(repo_path, jj_path)?;
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
        return Ok(format!("Fetched ({})", summary));
    }

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
        .args([
            "log",
            "-r",
            rev,
            "--no-graph",
            "--color",
            "never",
            "-T",
            "bookmarks",
        ])
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
///
/// When `bookmark_to_advance` is `Some(name)`, only that specific bookmark is
/// advanced — other bookmarks sharing the same parent commit are left in place.
/// This prevents unrelated bookmarks (e.g. "main") from being dragged forward
/// when the user is working on a feature bookmark.
pub(crate) fn jj_commit_all(
    repo_path: &Path,
    commit_message: &str,
    jj_path: &str,
    advance_bookmarks: bool,
    bookmark_to_advance: Option<&str>,
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
            "--color",
            "never",
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
    // When a specific bookmark is targeted, only advance that one —
    // this prevents dragging unrelated bookmarks (e.g. "main") forward
    // when they happen to share the same parent commit.
    for bm in &parent_bookmarks {
        if let Some(target) = bookmark_to_advance {
            if bm != target {
                continue;
            }
        }
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
    bookmark_to_advance: Option<&str>,
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
        return jj_commit_all(
            repo_path,
            commit_message,
            jj_path,
            true,
            bookmark_to_advance,
        );
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

    let result = jj_commit_all(
        repo_path,
        commit_message,
        jj_path,
        true,
        bookmark_to_advance,
    );

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
            "--color",
            "never",
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
        .args([
            "log",
            "--no-graph",
            "--color",
            "never",
            "-r",
            &revset,
            "-T",
            r#"change_id"#,
        ])
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

/// Abandon one or more revisions by change id.
pub(crate) fn jj_abandon(
    repo_path: &Path,
    change_ids: &[String],
    jj_path: &str,
) -> crate::error::Result<usize> {
    if change_ids.is_empty() {
        return Ok(0);
    }

    let revset = change_ids.join(" | ");
    let output = super::jj_cmd(jj_path)
        .args(["abandon", "-r", &revset])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::JjCommand(format!(
            "jj abandon failed: {}",
            stderr.trim()
        )));
    }

    Ok(change_ids.len())
}

/// Merge a bookmark into the current bookmark.
///
/// Uses `jj new current source` to create a merge commit with two parents,
/// then advances the current bookmark to the merge commit.
pub(crate) fn jj_merge_bookmark(
    repo_path: &Path,
    source_bookmark: &str,
    jj_path: &str,
    destination_bookmark: Option<&str>,
) -> crate::error::Result<String> {
    // Use the explicitly provided destination bookmark if available,
    // otherwise fall back to querying @- (which may be ambiguous).
    let current_bm = destination_bookmark
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| {
            bookmarks_for_rev(repo_path, "@-", jj_path)
                .first()
                .cloned()
                .unwrap_or_default()
        });

    // Create a merge commit: `jj new @- <source_bookmark>`
    // This creates a new working-copy change whose parents are both the
    // current bookmark and the source bookmark.
    let output = super::jj_cmd(jj_path)
        .args(["new", "@-", source_bookmark])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::JjCommand(format!(
            "jj new (merge) failed: {}",
            stderr.trim()
        )));
    }

    // Describe the merge commit.
    let msg = format!("merge '{}' into '{}'", source_bookmark, current_bm);
    let desc_output = super::jj_cmd(jj_path)
        .args(["describe", "-m", &msg])
        .current_dir(repo_path)
        .output()?;

    if !desc_output.status.success() {
        let stderr = String::from_utf8_lossy(&desc_output.stderr);
        return Err(DirigentError::JjCommand(format!(
            "jj describe (merge) failed: {}",
            stderr.trim()
        )));
    }

    // Commit the merge so it becomes @- and a fresh empty @ is created.
    let commit_output = super::jj_cmd(jj_path)
        .args(["commit", "-m", &msg])
        .current_dir(repo_path)
        .output()?;

    if !commit_output.status.success() {
        let stderr = String::from_utf8_lossy(&commit_output.stderr);
        return Err(DirigentError::JjCommand(format!(
            "jj commit (merge) failed: {}",
            stderr.trim()
        )));
    }

    // Advance the current bookmark to the merge commit (@-).
    if !current_bm.is_empty() {
        let _ = super::jj_cmd(jj_path)
            .args(["bookmark", "set", &current_bm, "-r", "@-"])
            .current_dir(repo_path)
            .output();
    }

    Ok(format!(
        "Merged '{}' into '{}'",
        source_bookmark, current_bm
    ))
}
