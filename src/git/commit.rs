use std::path::Path;

use git2::{BranchType, Repository, Signature};

use crate::error::DirigentError;

use super::diff::parse_diff_paths;
use super::merge::stage_files;

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

pub(crate) fn generate_commit_message(cue_text: &str) -> String {
    let summary = if cue_text.len() > 68 {
        format!("{}...", crate::app::truncate_str(cue_text, 65))
    } else {
        cue_text.to_string()
    };
    if cue_text.len() > 68 {
        format!("Dirigent: {}\n\n{}", summary, cue_text)
    } else {
        format!("Dirigent: {}", summary)
    }
}

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
/// to its remote tracking branch (`origin/<branch>`).
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

    // Determine the remote tracking ref to reset to
    let remote_ref = format!("origin/{}", current_branch);
    repo.refname_to_id(&format!("refs/remotes/{}", remote_ref))
        .map_err(|_| {
            DirigentError::GitCommand(format!(
                "no remote tracking branch '{}' — cannot move commits",
                remote_ref
            ))
        })?;

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

    #[test]
    fn generate_commit_message_short() {
        let msg = generate_commit_message("Fix typo");
        assert_eq!(msg, "Dirigent: Fix typo");
    }

    #[test]
    fn generate_commit_message_long_truncates() {
        let long_text = "A".repeat(100);
        let msg = generate_commit_message(&long_text);
        // Should have truncated summary with "..." and full body
        assert!(msg.starts_with("Dirigent: "));
        assert!(msg.contains("..."));
        assert!(msg.contains(&long_text));
    }

    #[test]
    fn generate_commit_message_boundary_68() {
        let exactly_68 = "B".repeat(68);
        let msg = generate_commit_message(&exactly_68);
        assert_eq!(msg, format!("Dirigent: {}", exactly_68));
        assert!(!msg.contains("..."));
    }
}
