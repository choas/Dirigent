use std::path::{Path, PathBuf};
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

fn run_jj_checked<const N: usize>(
    repo_path: &Path,
    jj_path: &str,
    args: [&str; N],
    failure_context: &str,
) -> crate::error::Result<Output> {
    let output = super::jj_cmd(jj_path)
        .args(args)
        .current_dir(repo_path)
        .output()?;

    if output.status.success() {
        Ok(output)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(DirigentError::JjCommand(format!(
            "{} failed: {}",
            failure_context,
            stderr.trim()
        )))
    }
}

fn summarize_jj_git_output(prefix: &str, output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let summary = stdout.lines().next().unwrap_or("ok").trim();
    format!("{} ({})", prefix, summary)
}

fn jj_git_command_error(
    repo_path: &Path,
    jj_path: &str,
    action: &str,
    stderr: &str,
    explain_conflicts: bool,
) -> DirigentError {
    if explain_conflicts && is_conflicted_commit_rejection(stderr) {
        return DirigentError::JjCommand(conflicted_ancestor_message(repo_path, jj_path, stderr));
    }
    DirigentError::JjCommand(format!("jj git {} failed: {}", action, stderr.trim()))
}

fn run_jj_git_command(
    repo_path: &Path,
    jj_path: &str,
    action: &str,
    success_prefix: &str,
    explain_conflicts: bool,
) -> crate::error::Result<String> {
    let mut output = super::jj_cmd(jj_path)
        .args(["git", action])
        .current_dir(repo_path)
        .output()?;

    if is_stale_working_copy(&output) {
        update_stale_working_copy(repo_path, jj_path)?;
        output = super::jj_cmd(jj_path)
            .args(["git", action])
            .current_dir(repo_path)
            .output()?;
    }

    if output.status.success() {
        return Ok(summarize_jj_git_output(success_prefix, &output));
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(jj_git_command_error(
        repo_path,
        jj_path,
        action,
        &stderr,
        explain_conflicts,
    ))
}

/// Push the current bookmarks to the remote via `jj git push`.
pub(crate) fn jj_push(repo_path: &Path, jj_path: &str) -> crate::error::Result<String> {
    run_jj_git_command(repo_path, jj_path, "push", "Pushed", true)
}

/// Fetch from remote via `jj git fetch`.
pub(crate) fn jj_pull(repo_path: &Path, jj_path: &str) -> crate::error::Result<String> {
    run_jj_git_command(repo_path, jj_path, "fetch", "Fetched", false)
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
        if let Some(target) = bookmark_to_advance
            && bm != target
        {
            continue;
        }
        let _ = super::jj_cmd(jj_path)
            .args(["bookmark", "set", bm, "-r", "@-"])
            .current_dir(repo_path)
            .output();
    }

    Ok(change_id)
}

/// Directory name prefix, under `.Dirigent/`, for the crash-recovery snapshot
/// taken by [`jj_commit_diff`].
const SNAPSHOT_DIR_PREFIX: &str = "pending-restore-";

/// Name of the manifest written alongside a snapshot.
const SNAPSHOT_MANIFEST: &str = "MANIFEST.txt";

/// On-disk snapshot of the working-copy files that a partial jj commit has to
/// temporarily revert.
struct WorkingCopySnapshot {
    dir: PathBuf,
    /// (repo-relative path, path inside `dir`) — absent path means the file was
    /// already deleted in the working copy and must be deleted again on restore.
    entries: Vec<(String, Option<PathBuf>)>,
}

impl WorkingCopySnapshot {
    /// Copy every listed file into `.Dirigent/pending-restore-<stamp>/`.
    ///
    /// This happens **before** `jj restore` touches anything, so the snapshot
    /// survives a crash, a kill, or a panic in the window between the restore
    /// and the write-back. Previously the only copy lived in this process's
    /// memory: dying in that window permanently discarded the user's uncommitted
    /// edits to every file outside the commit.
    fn capture(repo_path: &Path, files: &[String]) -> std::io::Result<Self> {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%f");
        let dir = repo_path
            .join(".Dirigent")
            .join(format!("{SNAPSHOT_DIR_PREFIX}{stamp}"));
        std::fs::create_dir_all(&dir)?;

        let mut entries = Vec::with_capacity(files.len());
        let mut manifest = String::from(
            "Dirigent snapshot of uncommitted working-copy files, taken before a\n\
             partial `jj` commit reverted them.\n\n\
             If Dirigent exited before restoring them, copy each file listed below\n\
             back to the matching path in the repository, then delete this directory.\n\n",
        );
        for rel in files {
            let source = repo_path.join(rel);
            if source.exists() {
                let stored = dir.join(rel);
                if let Some(parent) = stored.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&source, &stored)?;
                manifest.push_str(&format!("{rel}\n"));
                entries.push((rel.clone(), Some(stored)));
            } else {
                manifest.push_str(&format!("{rel}  (deleted in working copy)\n"));
                entries.push((rel.clone(), None));
            }
        }
        crate::util::fs::atomic_write(&dir.join(SNAPSHOT_MANIFEST), manifest.as_bytes())?;
        Ok(Self { dir, entries })
    }

    /// Put the snapshotted files back. Returns the paths that could not be
    /// restored — the caller must surface those rather than swallow them.
    fn restore(&self) -> Vec<String> {
        let mut failed = Vec::new();
        for (rel, stored) in &self.entries {
            let target = self.repo_target(rel);
            let outcome = match stored {
                Some(stored) => target
                    .parent()
                    .map(std::fs::create_dir_all)
                    .unwrap_or(Ok(()))
                    .and_then(|()| std::fs::copy(stored, &target).map(|_| ())),
                None => match std::fs::remove_file(&target) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    other => other,
                },
            };
            if let Err(e) = outcome {
                log::error!("[jj] failed to restore {rel} after partial commit: {e}");
                failed.push(rel.clone());
            }
        }
        failed
    }

    /// `<repo>/<rel>`, derived from the snapshot directory's location.
    fn repo_target(&self, rel: &str) -> PathBuf {
        self.repo_root().join(rel)
    }

    fn repo_root(&self) -> PathBuf {
        // dir == <repo>/.Dirigent/pending-restore-<stamp>
        self.dir
            .parent()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Remove the snapshot. Only called once every file was restored — a
    /// surviving directory is the recovery trail.
    fn discard(self) {
        if let Err(e) = std::fs::remove_dir_all(&self.dir) {
            log::warn!(
                "[jj] could not remove snapshot dir {}: {e}",
                self.dir.display()
            );
        }
    }
}

/// Commit only the files referenced in `diff_text`.
///
/// In jj every edit is already part of the working-copy commit (`@`).
/// To commit a subset we temporarily revert the *other* dirty files to
/// their parent state, commit, then write the originals back so they
/// reappear in the new `@`.
///
/// Every step of that dance is now failure-visible: the snapshot is written to
/// disk first (crash-recoverable), a failed `jj restore` aborts before anything
/// is committed, and a failed write-back is reported with the snapshot
/// directory left in place instead of being silently dropped.
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

    // Snapshot to disk BEFORE anything is reverted.
    let snapshot = WorkingCopySnapshot::capture(repo_path, &other_files).map_err(|e| {
        DirigentError::JjCommand(format!(
            "could not snapshot uncommitted files before the partial commit: {e}. \
             Nothing was changed."
        ))
    })?;

    // Revert non-diff files to parent state so they aren't part of the commit.
    let mut restore_args: Vec<&str> = vec!["restore", "--from", "@-", "--"];
    for f in &other_files {
        restore_args.push(f);
    }
    let revert = super::jj_cmd(jj_path)
        .args(&restore_args)
        .current_dir(repo_path)
        .output();
    match revert {
        Ok(output) if output.status.success() => {}
        outcome => {
            // Nothing has been committed yet. Put back whatever the partial
            // restore may have touched, then abort.
            let detail = match outcome {
                Ok(output) => String::from_utf8_lossy(&output.stderr).trim().to_string(),
                Err(e) => e.to_string(),
            };
            let failed = snapshot.restore();
            if failed.is_empty() {
                snapshot.discard();
                return Err(DirigentError::JjCommand(format!(
                    "could not set aside the other modified files, so nothing was \
                     committed: {detail}"
                )));
            }
            return Err(DirigentError::JjCommand(format!(
                "could not set aside the other modified files ({detail}), and {} of them \
                 could not be put back. Copies are in {}",
                failed.len(),
                snapshot.dir.display()
            )));
        }
    }

    let result = jj_commit_all(
        repo_path,
        commit_message,
        jj_path,
        true,
        bookmark_to_advance,
    );

    // Put the non-diff files back so they remain as pending changes in the new @.
    let failed = snapshot.restore();
    if !failed.is_empty() {
        // The commit itself may have succeeded, but the user's other edits are
        // only in the snapshot directory now — say so instead of returning Ok.
        return Err(DirigentError::JjCommand(format!(
            "committed, but {} uncommitted file(s) could not be restored: {}. \
             Copies are preserved in {}",
            failed.len(),
            failed.join(", "),
            snapshot.dir.display()
        )));
    }
    snapshot.discard();

    result
}

/// Delete a bookmark.
pub(crate) fn jj_delete_bookmark(
    repo_path: &Path,
    name: &str,
    jj_path: &str,
) -> crate::error::Result<()> {
    run_jj_checked(
        repo_path,
        jj_path,
        ["bookmark", "delete", name],
        "jj bookmark delete",
    )?;
    Ok(())
}

/// Run `jj log` for `revset` and return the local bookmark names it resolves to.
///
/// Shared by [`jj_merged_bookmarks`] and [`jj_trunk_bookmarks`]: both ask jj for
/// the `local_bookmarks` of a revset and parse the whitespace-separated output
/// (stripping the `*` that marks bookmarks with unpushed changes). Returns an
/// empty vec if jj fails or resolves nothing.
fn bookmarks_matching_revset(repo_path: &Path, jj_path: &str, revset: &str) -> Vec<String> {
    let output = super::jj_cmd(jj_path)
        .args([
            "log",
            "--no-graph",
            "--color",
            "never",
            "-r",
            revset,
            "-T",
            r#"local_bookmarks ++ "\n""#,
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

/// List local bookmarks that are fully merged into `trunk()`.
///
/// A bookmark counts as merged when its target commit is an ancestor of (or
/// equal to) `trunk()` — i.e. it has no commits of its own ahead of trunk and
/// is therefore safe to delete. The revset includes the trunk bookmark itself
/// (e.g. `main`/`master`), so callers should filter out protected names.
pub(crate) fn jj_merged_bookmarks(repo_path: &Path, jj_path: &str) -> Vec<String> {
    bookmarks_matching_revset(repo_path, jj_path, "bookmarks() & ::trunk()")
}

/// Resolve the name(s) of the repository's trunk bookmark.
///
/// `trunk()` is jj's configured default branch (commonly `main`/`master`, but a
/// repo can override `revset-aliases."trunk()"` to anything, e.g. `develop`).
/// Callers use this to protect the actual trunk instead of hard-coding
/// `main`/`master`. Returns an empty vec if jj cannot resolve `trunk()` (it then
/// falls back to `root()`, which carries no bookmarks).
///
/// Unlike [`bookmarks_matching_revset`], this templates `bookmarks` (local *and*
/// remote) at the `trunk()` commit rather than `local_bookmarks` of
/// `bookmarks() & trunk()`. `trunk()` usually resolves to the *remote* target
/// (e.g. `develop@origin`), so once the local bookmark advances ahead of the
/// remote there is no local bookmark sitting on that commit and a
/// `local_bookmarks`-only query comes back empty — silently failing to protect
/// the trunk. The `@remote` suffix is stripped so the local and remote forms
/// collapse to the bare bookmark name.
pub(crate) fn jj_trunk_bookmarks(repo_path: &Path, jj_path: &str) -> Vec<String> {
    let output = super::jj_cmd(jj_path)
        .args([
            "log",
            "--no-graph",
            "--color",
            "never",
            "-r",
            "trunk()",
            "-T",
            r#"bookmarks ++ "\n""#,
        ])
        .current_dir(repo_path)
        .output();
    match output {
        Ok(o) if o.status.success() => {
            let raw = String::from_utf8_lossy(&o.stdout);
            let mut names: Vec<String> = Vec::new();
            for token in raw.split_whitespace() {
                // Drop the `@remote` suffix (e.g. `develop@origin`) and any
                // conflict/sync markers jj appends (`*`, `?`).
                let name = token
                    .split('@')
                    .next()
                    .unwrap_or(token)
                    .trim_end_matches(['*', '?']);
                if !name.is_empty() && !names.iter().any(|n| n == name) {
                    names.push(name.to_string());
                }
            }
            names
        }
        _ => Vec::new(),
    }
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
    run_jj_checked(
        repo_path,
        jj_path,
        ["bookmark", "create", name, "-r", "@-"],
        "jj bookmark create",
    )?;
    Ok(())
}

/// Undo the last jj operation via `jj op restore @-`.
pub(crate) fn jj_undo(repo_path: &Path, jj_path: &str) -> crate::error::Result<String> {
    let output = run_jj_checked(repo_path, jj_path, ["op", "restore", "@-"], "jj op restore")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, contents: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, contents).expect("write");
    }

    /// The snapshot must be on disk before anything is reverted — that is the
    /// whole point: a crash in the revert/write-back window used to discard the
    /// user's uncommitted edits permanently.
    #[test]
    fn snapshot_is_written_to_disk_before_restore() {
        let repo = tempfile::tempdir().expect("tempdir");
        write(repo.path(), "src/keep.rs", "original contents");
        write(repo.path(), "nested/dir/other.txt", "other");

        let snapshot = WorkingCopySnapshot::capture(
            repo.path(),
            &[
                "src/keep.rs".to_string(),
                "nested/dir/other.txt".to_string(),
            ],
        )
        .expect("capture");

        assert!(snapshot.dir.starts_with(repo.path().join(".Dirigent")));
        assert_eq!(
            std::fs::read_to_string(snapshot.dir.join("src/keep.rs")).expect("stored copy"),
            "original contents"
        );
        assert!(
            snapshot.dir.join(SNAPSHOT_MANIFEST).exists(),
            "a recovery manifest must explain what the directory is"
        );
    }

    #[test]
    fn restore_puts_back_contents_and_reports_no_failures() {
        let repo = tempfile::tempdir().expect("tempdir");
        write(repo.path(), "a.txt", "mine");

        let snapshot =
            WorkingCopySnapshot::capture(repo.path(), &["a.txt".to_string()]).expect("capture");
        // Simulate `jj restore` reverting the file to its parent state.
        write(repo.path(), "a.txt", "parent version");

        let failed = snapshot.restore();

        assert!(failed.is_empty(), "unexpected failures: {failed:?}");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("a.txt")).expect("restored"),
            "mine"
        );
    }

    /// A file the user deleted in the working copy must stay deleted after the
    /// partial commit, not reappear from the parent revision.
    #[test]
    fn restore_reapplies_working_copy_deletions() {
        let repo = tempfile::tempdir().expect("tempdir");

        let snapshot =
            WorkingCopySnapshot::capture(repo.path(), &["gone.txt".to_string()]).expect("capture");
        // `jj restore` brings the file back from the parent commit.
        write(repo.path(), "gone.txt", "resurrected by jj restore");

        let failed = snapshot.restore();

        assert!(failed.is_empty(), "unexpected failures: {failed:?}");
        assert!(!repo.path().join("gone.txt").exists());
    }

    #[test]
    fn discard_removes_the_snapshot_directory() {
        let repo = tempfile::tempdir().expect("tempdir");
        write(repo.path(), "a.txt", "x");
        let snapshot =
            WorkingCopySnapshot::capture(repo.path(), &["a.txt".to_string()]).expect("capture");
        let dir = snapshot.dir.clone();
        snapshot.discard();
        assert!(!dir.exists());
    }
}
