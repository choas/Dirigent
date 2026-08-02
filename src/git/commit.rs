use std::collections::HashSet;
use std::path::Path;

use commitlint_rs::message::Message as LintMessage;
use commitlint_rs::rule::Rules as LintRules;
use git2::{BranchType, Repository, Signature};

use crate::error::DirigentError;

use super::diff::parse_diff_paths;
use super::merge::stage_files;

/// Footer appended to every Dirigent-generated commit message.
pub(crate) const DIRIGENT_FOOTER: &str = "Dirigent: https://github.com/choas/Dirigent";

/// Strategy for resolving diverged branches during pull.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PullStrategy {
    /// Only fast-forward (default, safest).
    FfOnly,
    /// Merge with a merge commit.
    Merge,
    /// Rebase local commits on top of remote.
    Rebase,
}

/// Commit whatever is currently staged in the repository index.
/// Returns the full commit OID as a string.
///
/// Handles: signature creation, parent resolution, nothing-to-commit
/// detection, and post-commit index reset.
fn commit_staged(
    repo: &Repository,
    commit_message: &str,
    nothing_msg: &str,
) -> crate::error::Result<String> {
    let mut index = repo.index()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let sig = repo.signature().unwrap_or_else(|_| {
        Signature::now("Dirigent", "Dirigent@local")
            .expect("hardcoded signature arguments are valid")
    });

    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());

    if let Some(ref parent_commit) = parent {
        if parent_commit.tree_id() == tree_id {
            return Err(DirigentError::GitCommand(nothing_msg.into()));
        }
    }

    let parents: Vec<&git2::Commit> = parent.iter().collect();
    let oid = repo.commit(Some("HEAD"), &sig, &sig, commit_message, &tree, &parents)?;

    // Reset the index back to the newly created commit so it doesn't stay staged.
    // Use the returned OID rather than repo.head() to avoid stale refdb cache.
    let new_commit = repo.find_commit(oid)?;
    repo.reset(new_commit.as_object(), git2::ResetType::Mixed, None)?;

    Ok(format!("{}", oid))
}

fn reset_index_to_head(repo_path: &Path) -> crate::error::Result<()> {
    let repo = Repository::discover(repo_path)?;
    let mut idx = repo.index()?;
    match repo.head() {
        Ok(head) => {
            let head_commit = head.peel_to_commit().map_err(|e| {
                DirigentError::GitCommand(format!("cannot peel HEAD to commit: {e}"))
            })?;
            let head_tree = head_commit.tree()?;
            idx.read_tree(&head_tree)?;
        }
        // Fresh repository with no commits yet (unborn branch): HEAD points at a
        // ref that doesn't exist, so there is no tree to reset to. Starting from
        // an empty index lets the first commit be created normally.
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => {
            idx.clear()?;
        }
        Err(e) => return Err(e.into()),
    }
    idx.write()?;
    Ok(())
}

/// Bits of `IndexEntry::flags` holding the merge stage; stage 0 is a normal entry.
const INDEX_ENTRY_STAGE_MASK: u16 = 0x3000;

/// Snapshot of the staged delta (HEAD vs index) taken before a commit flow
/// resets the index. `entries` holds the full index entries of staged
/// additions/modifications (their blobs already live in the object db, so the
/// entries stay restorable even if the working-tree file changes meanwhile);
/// `deletions` holds paths staged for deletion.
struct StagedSnapshot {
    entries: Vec<git2::IndexEntry>,
    deletions: Vec<Vec<u8>>,
}

fn snapshot_staged(repo: &Repository) -> crate::error::Result<StagedSnapshot> {
    let index = repo.index()?;
    let head_tree = match repo.head() {
        Ok(head) => Some(head.peel_to_commit()?.tree()?),
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => None,
        Err(e) => return Err(e.into()),
    };
    let diff = repo.diff_tree_to_index(head_tree.as_ref(), Some(&index), None)?;
    let mut changed: HashSet<Vec<u8>> = HashSet::new();
    let mut deletions = Vec::new();
    for delta in diff.deltas() {
        match delta.status() {
            git2::Delta::Deleted => {
                if let Some(p) = delta.old_file().path_bytes() {
                    deletions.push(p.to_vec());
                }
            }
            // Conflicted entries can't be restored via a stage-0 add; the
            // commit itself would fail on a conflicted index anyway.
            git2::Delta::Conflicted => {}
            _ => {
                if let Some(p) = delta.new_file().path_bytes() {
                    changed.insert(p.to_vec());
                }
            }
        }
    }
    let entries = index
        .iter()
        .filter(|e| e.flags & INDEX_ENTRY_STAGE_MASK == 0 && changed.contains(&e.path))
        .collect();
    Ok(StagedSnapshot { entries, deletions })
}

/// Re-apply a [`StagedSnapshot`] to the on-disk index, skipping paths in
/// `skip` (the paths a just-created commit consumed — their staged state was
/// either included in the commit or superseded by it).
fn restore_staged(
    repo: &Repository,
    snapshot: &StagedSnapshot,
    skip: &HashSet<Vec<u8>>,
) -> crate::error::Result<()> {
    if snapshot.entries.is_empty() && snapshot.deletions.is_empty() {
        return Ok(());
    }
    let mut index = repo.index()?;
    for entry in &snapshot.entries {
        if skip.contains(&entry.path) {
            continue;
        }
        index.add(entry)?;
    }
    for path in &snapshot.deletions {
        if skip.contains(path) {
            continue;
        }
        let Ok(path_str) = std::str::from_utf8(path) else {
            continue;
        };
        match index.remove_path(Path::new(path_str)) {
            Ok(()) => {}
            Err(e) if e.code() == git2::ErrorCode::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    index.write()?;
    Ok(())
}

/// Paths touched by the given commit (diff against its first parent, or the
/// empty tree for a root commit), as repo-relative byte paths.
fn committed_path_set(repo: &Repository, oid_str: &str) -> crate::error::Result<HashSet<Vec<u8>>> {
    let commit = repo.find_commit(git2::Oid::from_str(oid_str)?)?;
    let new_tree = commit.tree()?;
    let old_tree = match commit.parent(0) {
        Ok(parent) => Some(parent.tree()?),
        Err(_) => None,
    };
    let diff = repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)?;
    let mut paths = HashSet::new();
    for delta in diff.deltas() {
        for file in [delta.old_file(), delta.new_file()] {
            if let Some(p) = file.path_bytes() {
                paths.insert(p.to_vec());
            }
        }
    }
    Ok(paths)
}

/// Finish a commit flow that reset the index: on success, restore snapshotted
/// staged entries the commit didn't consume; on failure, roll the index back
/// to HEAD and restore the snapshot in full. Restore problems are logged
/// rather than propagated — they must not turn a created commit into an error
/// or mask the original failure.
fn finish_commit_restoring_staged(
    repo_path: &Path,
    snapshot: &StagedSnapshot,
    result: crate::error::Result<String>,
) -> crate::error::Result<String> {
    match result {
        Ok(oid) => {
            let restore = (|| {
                let repo = Repository::discover(repo_path)?;
                let committed = committed_path_set(&repo, &oid)?;
                restore_staged(&repo, snapshot, &committed)
            })();
            if let Err(e) = restore {
                log::warn!("commit succeeded but restoring staged entries failed: {e}");
            }
            Ok(oid)
        }
        Err(e) => {
            let rollback = (|| {
                reset_index_to_head(repo_path)?;
                let repo = Repository::discover(repo_path)?;
                restore_staged(&repo, snapshot, &HashSet::new())
            })();
            if let Err(re) = rollback {
                log::warn!("rollback after failed commit could not restore staged entries: {re}");
            }
            Err(e)
        }
    }
}

/// Commit the working-tree state of files touched by `diff_text`.
/// This stages the actual files the user sees (including any post-run formatting),
/// so the committed state matches the working tree and files appear clean afterwards.
pub(crate) fn commit_diff(
    repo_path: &Path,
    diff_text: &str,
    commit_message: &str,
) -> crate::error::Result<String> {
    let file_paths = parse_diff_paths(repo_path, diff_text);

    if file_paths.is_empty() {
        return Err(DirigentError::GitCommand(
            "no files to commit — diff contains no file paths".into(),
        ));
    }

    // Snapshot pre-existing staged state, then reset the index to HEAD so it
    // isn't included in this commit's tree. Entries for paths the commit
    // doesn't touch are restored afterwards — a user-curated index (hunk
    // staging UI, external `git add`) must survive committing an unrelated
    // cue, and a staged blob may be the only remaining copy of that content.
    let snapshot = snapshot_staged(&Repository::discover(repo_path)?)?;
    reset_index_to_head(repo_path)?;

    // Stage the working-tree state of the affected files, then commit.
    let result = stage_files(repo_path, &file_paths).and_then(|_| {
        let repo = Repository::discover(repo_path)?;
        commit_staged(
            &repo,
            commit_message,
            "nothing to commit — diff already applied",
        )
    });

    finish_commit_restoring_staged(repo_path, &snapshot, result)
}

/// Commit a partial selection: stage `whole_files` entirely and apply each
/// single-hunk patch in `hunk_patches` to the index, then commit the staged
/// index. The working tree is left untouched, so any un-selected hunks of a
/// shared file remain dirty afterwards.
pub(crate) fn commit_partial(
    repo_path: &Path,
    whole_files: &[String],
    hunk_patches: &[String],
    commit_message: &str,
) -> crate::error::Result<String> {
    if whole_files.is_empty() && hunk_patches.is_empty() {
        return Err(DirigentError::GitCommand(
            "nothing selected to commit".into(),
        ));
    }

    // Snapshot pre-existing staged state, then reset the index to HEAD so it
    // isn't included and the hunk patches (built against HEAD) apply cleanly.
    // Entries for paths this commit doesn't touch are restored afterwards
    // rather than discarded (see `commit_diff`).
    let snapshot = snapshot_staged(&Repository::discover(repo_path)?)?;
    reset_index_to_head(repo_path)?;

    // Stage whole files, apply each owned hunk to the index, then commit. Any
    // failure rolls the index back via `finish_commit_restoring_staged` —
    // without this a rejected hunk would leave the earlier stagings behind,
    // silently widening the next commit.
    let result = stage_files(repo_path, whole_files)
        .and_then(|_| {
            hunk_patches
                .iter()
                .try_for_each(|patch| super::staging::stage_hunk(repo_path, patch))
        })
        .and_then(|_| {
            let repo = Repository::discover(repo_path)?;
            commit_staged(&repo, commit_message, "nothing to commit — selection empty")
        });

    finish_commit_restoring_staged(repo_path, &snapshot, result)
}

pub(crate) fn revert_files(repo_path: &Path, file_paths: &[String]) -> crate::error::Result<()> {
    use std::process::Command;

    if file_paths.is_empty() {
        return Ok(());
    }

    let mut cmd = Command::new("git");
    cmd.arg("checkout").arg("--");
    for f in file_paths {
        cmd.arg(f);
    }
    let output = cmd.current_dir(repo_path).output()?;

    if !output.status.success() {
        return Err(DirigentError::GitCommand(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(())
}

/// Stage all changes (tracked + untracked) and commit with the given message.
pub(crate) fn commit_all(repo_path: &Path, commit_message: &str) -> crate::error::Result<String> {
    use std::process::Command;

    // Stage all changes including untracked files
    let output = Command::new("git")
        .args(["add", "-A"])
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "git add -A failed: {}",
            stderr
        )));
    }

    let repo = Repository::discover(repo_path)?;
    commit_staged(
        &repo,
        commit_message,
        "nothing to commit — no uncommitted changes",
    )
}

/// Detect the conventional commit type from cue text using keyword heuristics.
pub(crate) fn detect_commit_type(text: &str) -> &'static str {
    let lower = text.to_lowercase();
    let first_word = lower.split_whitespace().next().unwrap_or("");
    conventional_type_prefix(first_word)
        .or_else(|| keyword_commit_type(&lower))
        .unwrap_or("feat")
}

fn conventional_type_prefix(first_word: &str) -> Option<&'static str> {
    CONVENTIONAL_TYPES.iter().copied().find(|ct| {
        first_word == *ct
            || first_word.starts_with(&format!("{ct}:"))
            || first_word.starts_with(&format!("{ct}("))
    })
}

fn keyword_commit_type(lower: &str) -> Option<&'static str> {
    const KEYWORDS: &[(&str, &[&str])] = &[
        ("fix", &["fix", "bug", "crash"]),
        ("refactor", &["refactor", "rename", "move ", "extract"]),
        ("docs", &["readme", "documentation"]),
        ("test", &["test"]),
        ("style", &["format", "whitespace", "lint"]),
        ("perf", &["performance", "optimiz", "speed"]),
        ("build", &["dependenc", "upgrade", "cargo"]),
        ("ci", &[" ci", "pipeline", "workflow"]),
        ("chore", &["cleanup", "chore"]),
        ("revert", &["revert"]),
    ];
    KEYWORDS
        .iter()
        .find(|(commit_type, terms)| {
            terms.iter().any(|term| lower.contains(term))
                || (*commit_type == "docs" && lower.starts_with("doc"))
        })
        .map(|(commit_type, _)| *commit_type)
}

/// Strip an existing conventional commit type prefix (e.g. "feat: " or "fix(scope): ").
fn strip_type_prefix(text: &str) -> &str {
    let bytes = text.as_bytes();
    let mut i = 0;
    // skip word chars (the type)
    while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    // optional scope in parens
    if i < bytes.len() && bytes[i] == b'(' {
        if let Some(close) = text[i..].find(')') {
            i += close + 1;
        }
    }
    // optional '!'
    if i < bytes.len() && bytes[i] == b'!' {
        i += 1;
    }
    // colon + optional space
    if i < bytes.len() && bytes[i] == b':' {
        i += 1;
        if i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        if text.is_char_boundary(i) {
            return &text[i..];
        }
    }
    text
}

/// Format description for conventional commit: lowercase first char, drop trailing period.
fn format_description(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut chars = trimmed.chars();
    let first = chars.next().unwrap();
    let rest: String = chars.collect();
    let desc = format!("{}{}", first.to_lowercase(), rest);
    desc.trim_end_matches('.').to_string()
}

fn commit_subject(commit_type: &str, description: &str) -> String {
    let prefix = format!("{}: ", commit_type);
    let max_desc = 72 - prefix.len();
    let subject_desc = if description.len() > max_desc {
        format!("{}...", crate::app::truncate_str(description, max_desc - 3))
    } else {
        description.to_string()
    };
    format!("{}{}", prefix, subject_desc)
}

/// Validate a raw commit message using commitlint-rs rules.
/// Returns a list of violation messages (empty = valid).
pub(crate) fn lint_commit_message(raw: &str) -> Vec<String> {
    let msg = LintMessage::new(raw.to_string());
    let rules = LintRules::default();
    rules
        .validate(&msg)
        .into_iter()
        .map(|v| v.message)
        .collect()
}

/// Upper bound for the CLI message we include in the commit body. This is a
/// sanity guard against a corrupted PTY capture bloating the commit, not a
/// quality filter: a detailed multi-sentence message is perfectly valid, so a
/// message longer than this is truncated rather than discarded.
const MAX_LOG_MESSAGE_LEN: usize = 2000;

/// Generate a conventional commit message from the CLI-generated commit
/// message and/or cue text.
///
/// When `cli_message` is provided (extracted from `<commit-message>` tags),
/// it drives both the subject line and message body.  The cue text is
/// appended so the original user intent is preserved in the commit.
///
/// Format: `type: title\n\n[cli body]\n\nCue: cue_text\n\nDirigent: …`
///
/// When `include_footer` is false the trailing `Dirigent: <url>` line is
/// omitted (controlled by the "Add Dirigent footer to commits" setting).
pub(crate) fn generate_commit_message(
    cue_text: &str,
    cli_message: Option<&str>,
    include_footer: bool,
) -> String {
    let cli_body = cli_message.map(|m| m.trim()).filter(|t| !t.is_empty());

    let (subject, body_parts) = if let Some(cli) = cli_body {
        let first_line = cli.lines().next().unwrap_or(cli);
        let commit_type = detect_commit_type(first_line);
        let raw_desc = strip_type_prefix(first_line);
        let description = format_description(raw_desc);
        let subject = commit_subject(commit_type, &description);

        let mut parts = Vec::new();
        // Include the CLI message in the body. Only an over-long message
        // (likely a garbled capture) is truncated — a normal detailed
        // message is kept in full rather than thrown away.
        let body = if cli.len() > MAX_LOG_MESSAGE_LEN {
            format!("{}…", crate::app::truncate_str(cli, MAX_LOG_MESSAGE_LEN))
        } else {
            cli.to_string()
        };
        parts.push(body);
        parts.push(format!("Cue: {}", cue_text));

        (subject, parts)
    } else {
        let commit_type = detect_commit_type(cue_text);
        let raw_desc = strip_type_prefix(cue_text);
        let description = format_description(raw_desc);
        let subject = commit_subject(commit_type, &description);

        let mut parts = Vec::new();
        if cue_text.len() > 68 {
            parts.push(cue_text.to_string());
        }

        (subject, parts)
    };

    let mut sections = vec![subject.clone()];
    for p in &body_parts {
        sections.push(p.clone());
    }
    if include_footer {
        sections.push(DIRIGENT_FOOTER.to_string());
    }
    let msg = sections.join("\n\n");

    // Validate with commitlint-rs — log any issues for diagnostics.
    let violations = lint_commit_message(&msg);
    if !violations.is_empty() {
        log::warn!(
            "[commitlint] generated message has {} violation(s): {}",
            violations.len(),
            violations.join("; ")
        );
    }

    msg
}

const CONVENTIONAL_TYPES: &[&str] = &[
    "feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore", "revert",
];

/// Push the current branch to its remote (typically `origin`).
/// When there is no remote tracking branch (e.g. a new worktree branch), pushes with
/// `-u origin <branch>` to set up tracking.
/// Returns the remote name and branch that was pushed (e.g. "origin/main").
pub(crate) fn git_push(repo_path: &Path) -> crate::error::Result<String> {
    use std::process::Command;

    // Determine current branch
    let repo = Repository::discover(repo_path)?;
    let head = repo
        .head()
        .map_err(|e| DirigentError::GitCommand(format!("cannot determine HEAD: {}", e)))?;
    let branch_name = head
        .shorthand()
        .ok_or_else(|| DirigentError::GitCommand("HEAD is not on a branch".into()))?
        .to_string();

    // Check if the local branch already has an upstream configured
    let has_upstream = repo
        .find_branch(&branch_name, BranchType::Local)
        .ok()
        .and_then(|b| b.upstream().ok())
        .is_some();

    let output = if has_upstream {
        Command::new("git")
            .args(["push", "--porcelain", "--follow-tags"])
            .current_dir(repo_path)
            .output()?
    } else {
        // No upstream — determine the default remote and push with -u to set up tracking
        let remotes = repo.remotes()?;
        let remote_name = remotes.iter().flatten().next().ok_or_else(|| {
            DirigentError::GitCommand("no remotes configured for repository".to_string())
        })?;
        Command::new("git")
            .args([
                "push",
                "-u",
                remote_name,
                &branch_name,
                "--porcelain",
                "--follow-tags",
            ])
            .current_dir(repo_path)
            .output()?
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = match (stderr.is_empty(), stdout.is_empty()) {
            (false, false) => format!("{}\n{}", stderr, stdout),
            (false, true) => stderr,
            (true, false) => stdout,
            (true, true) => "unknown error (no output)".to_string(),
        };
        return Err(DirigentError::GitCommand(format!(
            "git push failed:\n{}",
            detail
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(format!(
        "Pushed {} ({})",
        branch_name,
        stdout.lines().next().unwrap_or("ok").trim()
    ))
}

/// Pull the current branch from its remote (typically `origin`).
/// Returns a summary string describing the result.
pub(crate) fn git_pull(repo_path: &Path, strategy: PullStrategy) -> crate::error::Result<String> {
    use std::process::Command;

    // Determine current branch
    let repo = Repository::discover(repo_path)?;
    let head = repo
        .head()
        .map_err(|e| DirigentError::GitCommand(format!("cannot determine HEAD: {}", e)))?;
    let branch_name = head
        .shorthand()
        .ok_or_else(|| DirigentError::GitCommand("HEAD is not on a branch".into()))?
        .to_string();

    let args: Vec<&str> = match strategy {
        PullStrategy::FfOnly => vec!["pull", "--ff-only"],
        PullStrategy::Merge => vec![
            "-c",
            "pull.ff=false",
            "-c",
            "pull.rebase=false",
            "pull",
            "--no-ff",
        ],
        PullStrategy::Rebase => vec!["-c", "pull.rebase=true", "pull", "--rebase"],
    };

    let output = Command::new("git")
        .args(&args)
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "git pull failed: {}",
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let summary = stdout.lines().next().unwrap_or("ok").trim();
    let strategy_label = match strategy {
        PullStrategy::FfOnly => "",
        PullStrategy::Merge => ", merge",
        PullStrategy::Rebase => ", rebase",
    };
    Ok(format!(
        "Pulled {}{} ({})",
        branch_name, strategy_label, summary
    ))
}

/// Create a new branch at the current HEAD, then reset the current branch back
/// to its configured upstream tracking branch.
///
/// This effectively "moves" all local-only commits from the current branch to
/// the new branch, leaving the current branch in sync with the remote.
///
/// Returns `Ok(new_branch_name)` on success.
pub(crate) fn move_to_new_branch(
    repo_path: &Path,
    new_branch_name: &str,
) -> crate::error::Result<String> {
    use std::process::Command;

    let repo = Repository::discover(repo_path)?;

    // Determine the current branch name
    let head = repo
        .head()
        .map_err(|e| DirigentError::GitCommand(format!("cannot determine HEAD: {}", e)))?;
    let current_branch = head
        .shorthand()
        .ok_or_else(|| DirigentError::GitCommand("HEAD is not on a branch".into()))?
        .to_string();

    // Determine the remote tracking ref to reset to via the configured upstream
    let local_branch = repo
        .find_branch(&current_branch, BranchType::Local)
        .map_err(|e| {
            DirigentError::GitCommand(format!(
                "cannot find local branch '{}': {}",
                current_branch, e
            ))
        })?;
    let upstream = local_branch.upstream().map_err(|_| {
        DirigentError::GitCommand(format!(
            "no upstream configured for '{}' — cannot move commits",
            current_branch
        ))
    })?;
    let remote_ref = upstream
        .get()
        .shorthand()
        .ok_or_else(|| DirigentError::GitCommand("upstream ref has no shorthand name".into()))?
        .to_string();

    // Refuse to proceed if the working tree has uncommitted changes,
    // because `git reset --hard` below would destroy them.
    let dirty = super::status::get_dirty_files(repo_path);
    if !dirty.is_empty() {
        return Err(DirigentError::GitCommand(
            "cannot move commits: working tree has uncommitted changes — commit or stash first"
                .into(),
        ));
    }

    // Create the new branch at current HEAD
    let output = Command::new("git")
        .args(["branch", "--", new_branch_name])
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        return Err(DirigentError::GitCommand(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    // Reset current branch back to the remote
    let output = Command::new("git")
        .args(["reset", "--hard", &remote_ref])
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        return Err(DirigentError::GitCommand(format!(
            "git reset failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(new_branch_name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- detect_commit_type ---

    #[test]
    fn detect_type_fix_keyword() {
        assert_eq!(detect_commit_type("Fix typo in readme"), "fix");
        assert_eq!(detect_commit_type("resolve a bug in login"), "fix");
    }

    #[test]
    fn detect_type_explicit_prefix() {
        assert_eq!(detect_commit_type("refactor: extract helper"), "refactor");
        assert_eq!(detect_commit_type("docs(readme): update"), "docs");
    }

    #[test]
    fn detect_type_defaults_to_feat() {
        assert_eq!(detect_commit_type("add dark mode toggle"), "feat");
    }

    #[test]
    fn detect_type_various_keywords() {
        assert_eq!(detect_commit_type("rename the function"), "refactor");
        assert_eq!(detect_commit_type("add unit test for parser"), "test");
        assert_eq!(detect_commit_type("optimize database queries"), "perf");
        assert_eq!(detect_commit_type("update dependencies"), "build");
    }

    // --- strip_type_prefix ---

    #[test]
    fn strip_prefix_present() {
        assert_eq!(strip_type_prefix("feat: add button"), "add button");
        assert_eq!(strip_type_prefix("fix(ui): align text"), "align text");
        assert_eq!(
            strip_type_prefix("feat!: breaking change"),
            "breaking change"
        );
    }

    #[test]
    fn strip_prefix_absent() {
        assert_eq!(strip_type_prefix("add button"), "add button");
        assert_eq!(strip_type_prefix("Fix the login bug"), "Fix the login bug");
    }

    // --- format_description ---

    #[test]
    fn format_desc_lowercase_and_strip_period() {
        assert_eq!(format_description("Add button."), "add button");
        assert_eq!(format_description("fix typo"), "fix typo");
        assert_eq!(format_description("  Trim me  "), "trim me");
    }

    // --- generate_commit_message ---

    #[test]
    fn generate_commit_message_short() {
        let msg = generate_commit_message("Fix typo", None, true);
        assert!(msg.starts_with("fix: fix typo"));
        assert!(msg.ends_with(DIRIGENT_FOOTER));
        assert!(lint_commit_message(&msg).is_empty());
    }

    #[test]
    fn generate_commit_message_omits_footer_when_disabled() {
        let msg = generate_commit_message("Fix typo", None, false);
        assert!(msg.starts_with("fix: fix typo"));
        assert!(!msg.contains(DIRIGENT_FOOTER));
        assert!(lint_commit_message(&msg).is_empty());
    }

    #[test]
    fn generate_commit_message_long_truncates() {
        let long_text = format!("Add {}", "feature ".repeat(20));
        let msg = generate_commit_message(&long_text, None, true);
        let subject = msg.lines().next().unwrap();
        assert!(subject.len() <= 72, "subject too long: {}", subject.len());
        assert!(subject.contains("..."));
        assert!(msg.contains(&long_text)); // full text in body
        assert!(msg.ends_with(DIRIGENT_FOOTER));
        assert!(lint_commit_message(&msg).is_empty());
    }

    #[test]
    fn generate_commit_message_short_no_body() {
        let msg = generate_commit_message("add dark mode", None, true);
        let parts: Vec<&str> = msg.split("\n\n").collect();
        assert_eq!(parts.len(), 2); // subject + footer only
        assert!(lint_commit_message(&msg).is_empty());
    }

    #[test]
    fn generate_commit_message_preserves_existing_prefix() {
        let msg = generate_commit_message("docs: update readme", None, true);
        assert!(msg.starts_with("docs: update readme"));
        assert!(lint_commit_message(&msg).is_empty());
    }

    #[test]
    fn generate_commit_message_with_cli_message() {
        let msg =
            generate_commit_message("Fix typo", Some("Fixed a typo in the README file"), true);
        // Subject derived from CLI message, not cue text
        assert!(msg.starts_with("fix: fixed a typo in the README file"));
        // Full CLI message preserved in body
        assert!(msg.contains("Fixed a typo in the README file"));
        assert!(msg.contains("Cue: Fix typo"));
        assert!(msg.ends_with(DIRIGENT_FOOTER));
        assert!(lint_commit_message(&msg).is_empty());
    }

    #[test]
    fn generate_commit_message_cli_multiline() {
        let cli = "Added new button component\n\nThis adds a reusable button with variants.";
        let msg = generate_commit_message("add a button", Some(cli), true);
        assert!(msg.starts_with("feat: added new button component"));
        // Full CLI message (title + body) preserved in body section
        assert!(msg
            .contains("Added new button component\n\nThis adds a reusable button with variants."));
        assert!(msg.contains("Cue: add a button"));
        assert!(msg.ends_with(DIRIGENT_FOOTER));
    }

    #[test]
    fn generate_commit_message_keeps_detailed_cli() {
        // A long but legitimate message (e.g. 765 chars from a real run) must be
        // used, not discarded in favour of the raw cue text.
        let detailed = format!("Add a feature.\n\n{}", "Detail sentence. ".repeat(40));
        let detailed = detailed.trim();
        assert!(detailed.len() > 500 && detailed.len() <= MAX_LOG_MESSAGE_LEN);
        let msg = generate_commit_message("add a feature", Some(detailed), true);
        assert!(msg.starts_with("feat: add a feature"));
        assert!(msg.contains(detailed)); // full message preserved
        assert!(msg.contains("Cue: add a feature"));
    }

    #[test]
    fn generate_commit_message_truncates_oversized_cli() {
        // Only an absurdly long (likely garbled) capture is truncated — and even
        // then the message is still used rather than dropped to the cue text.
        let huge_cli = format!(
            "Add a feature.\n\n{}",
            "x".repeat(MAX_LOG_MESSAGE_LEN + 500)
        );
        let msg = generate_commit_message("Fix typo", Some(&huge_cli), true);
        assert!(msg.starts_with("feat: add a feature"));
        assert!(!msg.contains(&huge_cli)); // truncated
        assert!(msg.contains("…"));
        assert!(msg.contains("Cue: Fix typo"));
    }

    // --- lint_commit_message ---

    #[test]
    fn lint_valid_message() {
        let msg = format!("feat: add dark mode\n\n{}", DIRIGENT_FOOTER);
        assert!(lint_commit_message(&msg).is_empty());
    }

    #[test]
    fn lint_missing_type() {
        // A message without conventional type prefix triggers violations
        let violations = lint_commit_message("just a plain message");
        assert!(!violations.is_empty());
    }
}
