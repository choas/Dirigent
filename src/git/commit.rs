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

    // Reset the index to HEAD so pre-existing staged changes aren't included.
    {
        let repo = Repository::discover(repo_path)?;
        let head_commit = repo
            .head()?
            .peel_to_commit()
            .map_err(|e| DirigentError::GitCommand(format!("cannot peel HEAD to commit: {e}")))?;
        let head_tree = head_commit.tree()?;
        let mut idx = repo.index()?;
        idx.read_tree(&head_tree)?;
        idx.write()?;
    }

    // Stage the working-tree state of the affected files.
    stage_files(repo_path, &file_paths)?;

    let repo = Repository::discover(repo_path)?;
    commit_staged(
        &repo,
        commit_message,
        "nothing to commit — diff already applied",
    )
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

    // If the text already starts with a known conventional type prefix, use it.
    for &ct in CONVENTIONAL_TYPES {
        if first_word == ct
            || first_word.starts_with(&format!("{ct}:"))
            || first_word.starts_with(&format!("{ct}("))
        {
            return ct;
        }
    }

    // Keyword-based heuristics (order matters — more specific first).
    if lower.contains("fix") || lower.contains("bug") || lower.contains("crash") {
        "fix"
    } else if lower.contains("refactor")
        || lower.contains("rename")
        || lower.contains("move ")
        || lower.contains("extract")
    {
        "refactor"
    } else if lower.contains("readme")
        || lower.contains("documentation")
        || lower.starts_with("doc")
    {
        "docs"
    } else if lower.contains("test") {
        "test"
    } else if lower.contains("format") || lower.contains("whitespace") || lower.contains("lint") {
        "style"
    } else if lower.contains("performance") || lower.contains("optimiz") || lower.contains("speed")
    {
        "perf"
    } else if lower.contains("dependenc") || lower.contains("upgrade") || lower.contains("cargo") {
        "build"
    } else if lower.contains(" ci") || lower.contains("pipeline") || lower.contains("workflow") {
        "ci"
    } else if lower.contains("cleanup") || lower.contains("chore") {
        "chore"
    } else if lower.contains("revert") {
        "revert"
    } else {
        "feat"
    }
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
        return &text[i..];
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

/// Generate a conventional commit message from cue text.
///
/// Format: `type: description\n\n[body]\n\nDirigent: https://github.com/choas/Dirigent`
pub(crate) fn generate_commit_message(cue_text: &str) -> String {
    let commit_type = detect_commit_type(cue_text);
    let raw_desc = strip_type_prefix(cue_text);
    let description = format_description(raw_desc);

    // Build subject line within 72-char limit.
    let prefix = format!("{}: ", commit_type);
    let max_desc = 72 - prefix.len();
    let subject_desc = if description.len() > max_desc {
        format!(
            "{}...",
            crate::app::truncate_str(&description, max_desc - 3)
        )
    } else {
        description.clone()
    };

    let subject = format!("{}{}", prefix, subject_desc);

    let msg = if cue_text.len() > 68 {
        format!("{}\n\n{}\n\n{}", subject, cue_text, DIRIGENT_FOOTER)
    } else {
        format!("{}\n\n{}", subject, DIRIGENT_FOOTER)
    };

    // Validate with commitlint-rs — log any issues for diagnostics.
    let violations = lint_commit_message(&msg);
    if !violations.is_empty() {
        eprintln!(
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
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "git push failed: {}",
            stderr.trim()
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
        let msg = generate_commit_message("Fix typo");
        assert!(msg.starts_with("fix: fix typo"));
        assert!(msg.ends_with(DIRIGENT_FOOTER));
        // Validate with commitlint-rs
        assert!(lint_commit_message(&msg).is_empty());
    }

    #[test]
    fn generate_commit_message_long_truncates() {
        let long_text = format!("Add {}", "feature ".repeat(20));
        let msg = generate_commit_message(&long_text);
        let subject = msg.lines().next().unwrap();
        assert!(subject.len() <= 72, "subject too long: {}", subject.len());
        assert!(subject.contains("..."));
        assert!(msg.contains(&long_text)); // full text in body
        assert!(msg.ends_with(DIRIGENT_FOOTER));
        assert!(lint_commit_message(&msg).is_empty());
    }

    #[test]
    fn generate_commit_message_short_no_body() {
        let msg = generate_commit_message("add dark mode");
        // Short message should NOT have the cue text repeated as body
        let parts: Vec<&str> = msg.split("\n\n").collect();
        assert_eq!(parts.len(), 2); // subject + footer only
        assert!(lint_commit_message(&msg).is_empty());
    }

    #[test]
    fn generate_commit_message_preserves_existing_prefix() {
        let msg = generate_commit_message("docs: update readme");
        assert!(msg.starts_with("docs: update readme"));
        assert!(lint_commit_message(&msg).is_empty());
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
