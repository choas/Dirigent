use std::path::{Path, PathBuf};

use git2::Repository;

use crate::error::DirigentError;

/// Create a GitHub pull request for the current branch using `gh pr create`.
/// Returns the PR URL on success.
pub(crate) fn create_pull_request(
    repo_path: &Path,
    title: &str,
    body: &str,
    base: &str,
    draft: bool,
) -> crate::error::Result<String> {
    use std::process::Command;

    let mut cmd = Command::new("gh");
    cmd.arg("pr")
        .arg("create")
        .arg("--title")
        .arg(title)
        .arg("--body")
        .arg(body)
        .arg("--base")
        .arg(base);
    if draft {
        cmd.arg("--draft");
    }

    let output = cmd.current_dir(repo_path).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "gh pr create failed: {}",
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.trim().to_string())
}

/// Get the default branch name for the repository (e.g. "main" or "master").
/// Falls back to "main" if detection fails.
pub(crate) fn get_default_branch(repo_path: &Path) -> String {
    use std::process::Command;

    // Try gh api first (most reliable for GitHub repos)
    if let Ok(output) = Command::new("gh")
        .args([
            "repo",
            "view",
            "--json",
            "defaultBranchRef",
            "-q",
            ".defaultBranchRef.name",
        ])
        .current_dir(repo_path)
        .output()
    {
        if output.status.success() {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !branch.is_empty() {
                return branch;
            }
        }
    }

    // Fallback: check refs/remotes/origin/HEAD
    let repo = match Repository::discover(repo_path) {
        Ok(r) => r,
        Err(_) => return "main".to_string(),
    };
    if let Ok(reference) = repo.find_reference("refs/remotes/origin/HEAD") {
        if let Some(target) = reference.symbolic_target() {
            if let Some(branch) = target.strip_prefix("refs/remotes/origin/") {
                return branch.to_string();
            }
        }
    }

    "main".to_string()
}

/// Build a PR body from the commits on the current branch that are ahead of the base branch.
pub(crate) fn build_pr_body(repo_path: &Path, base: &str) -> String {
    use std::process::Command;

    let output = Command::new("git")
        .args(["log", "--oneline", &format!("origin/{}..HEAD", base)])
        .current_dir(repo_path)
        .output();

    let commits = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => String::new(),
    };

    if commits.is_empty() {
        return String::new();
    }

    let bullet_list: String = commits
        .lines()
        .map(|line| format!("- {}", line))
        .collect::<Vec<_>>()
        .join("\n");

    format!("## Changes\n\n{}", bullet_list)
}

/// Returns the path to the main (non-linked) worktree / main repo.
/// The first entry in `git worktree list` is always the main worktree.
pub(crate) fn main_worktree_path(repo_path: &Path) -> crate::error::Result<PathBuf> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        return Err(DirigentError::GitCommand(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("worktree ") {
            return Ok(PathBuf::from(rest));
        }
    }

    Err(DirigentError::GitCommand("no main worktree found".into()))
}
