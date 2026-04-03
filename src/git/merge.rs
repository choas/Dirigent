use std::path::Path;

use git2::{Repository, StatusOptions};

use crate::error::DirigentError;

/// What kind of merge/rebase operation is in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum MergeOperation {
    Merge,
    Rebase,
}

/// Detect whether a merge or rebase is currently in progress.
pub(crate) fn detect_merge_operation(repo_path: &Path) -> Option<MergeOperation> {
    let repo = Repository::discover(repo_path).ok()?;
    let git_dir = repo.path(); // .git directory

    // Rebase in progress?
    if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
        return Some(MergeOperation::Rebase);
    }
    // Merge in progress?
    if git_dir.join("MERGE_HEAD").exists() {
        return Some(MergeOperation::Merge);
    }
    None
}

/// Returns relative paths of files with merge conflicts (unmerged entries in the index).
pub(crate) fn get_conflicted_files(repo_path: &Path) -> Vec<String> {
    let repo = match Repository::discover(repo_path) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut opts = StatusOptions::new();
    opts.include_untracked(false);
    let statuses = match repo.statuses(Some(&mut opts)) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mut files = Vec::new();
    for entry in statuses.iter() {
        if entry.status().is_conflicted() {
            if let Some(p) = entry.path() {
                files.push(p.to_string());
            }
        }
    }
    files
}

/// Run a git command and return its stdout on success, or a `GitCommand` error
/// with the given `label` and stderr on failure.
fn run_git(
    repo_path: &Path,
    cmd: &mut std::process::Command,
    label: &str,
) -> crate::error::Result<String> {
    let output = cmd.current_dir(repo_path).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "{} failed: {}",
            label,
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Stage resolved files (git add).
pub(crate) fn stage_files(repo_path: &Path, files: &[String]) -> crate::error::Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let mut cmd = std::process::Command::new("git");
    cmd.arg("add").arg("--");
    for f in files {
        cmd.arg(f);
    }
    run_git(repo_path, &mut cmd, "git add")?;
    Ok(())
}

/// Abort an in-progress merge.
pub(crate) fn merge_abort(repo_path: &Path) -> crate::error::Result<()> {
    run_git(
        repo_path,
        std::process::Command::new("git").args(["merge", "--abort"]),
        "git merge --abort",
    )?;
    Ok(())
}

/// Abort an in-progress rebase.
pub(crate) fn rebase_abort(repo_path: &Path) -> crate::error::Result<()> {
    run_git(
        repo_path,
        std::process::Command::new("git").args(["rebase", "--abort"]),
        "git rebase --abort",
    )?;
    Ok(())
}

/// Complete a merge after all conflicts are resolved (creates a merge commit).
pub(crate) fn merge_continue(repo_path: &Path) -> crate::error::Result<String> {
    // git commit with no message flag — git will use the auto-generated merge message
    let stdout = run_git(
        repo_path,
        std::process::Command::new("git").args(["commit", "--no-edit"]),
        "git commit",
    )?;
    Ok(stdout
        .lines()
        .next()
        .unwrap_or("merge complete")
        .trim()
        .to_string())
}

/// Continue a rebase after all conflicts in the current step are resolved.
pub(crate) fn rebase_continue(repo_path: &Path) -> crate::error::Result<String> {
    let stdout = run_git(
        repo_path,
        std::process::Command::new("git")
            .args(["rebase", "--continue"])
            .env("GIT_EDITOR", "true"), // skip editor for commit message
        "git rebase --continue",
    )?;
    Ok(stdout
        .lines()
        .last()
        .unwrap_or("rebase complete")
        .trim()
        .to_string())
}
