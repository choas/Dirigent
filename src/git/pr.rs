use std::path::{Path, PathBuf};

use git2::Repository;

use super::forgejo::{self, RemoteInfo};
use crate::error::DirigentError;

/// Create a pull request for the current branch.
///
/// GitHub remotes use the `gh` CLI (with a REST fallback). Codeberg remotes use
/// the Forgejo REST API directly, since `gh` cannot talk to Codeberg.
/// Returns the PR URL on success.
pub(crate) fn create_pull_request(
    repo_path: &Path,
    title: &str,
    body: &str,
    base: &str,
    draft: bool,
) -> crate::error::Result<String> {
    use std::process::Command;

    let repo = Repository::discover(repo_path)?;
    let head_ref = repo
        .head()
        .map_err(|e| DirigentError::GitCommand(format!("cannot determine HEAD: {}", e)))?;
    let branch_name = head_ref
        .shorthand()
        .ok_or_else(|| DirigentError::GitCommand("HEAD is not on a branch".into()))?
        .to_string();

    // Route Codeberg (Forgejo) remotes through the Forgejo API; `gh` is GitHub-only.
    if let Some(remote) = forgejo::codeberg_remote(repo_path) {
        return create_pull_request_codeberg(
            repo_path,
            &remote,
            title,
            body,
            base,
            &branch_name,
            draft,
        );
    }

    // Run `gh` from the main worktree directory so it can resolve the remote.
    let gh_dir = main_worktree_path(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());

    let mut cmd = Command::new("gh");
    cmd.arg("pr")
        .arg("create")
        .arg("--title")
        .arg(title)
        .arg("--body")
        .arg(body)
        .arg("--base")
        .arg(base)
        .arg("--head")
        .arg(&branch_name);
    if draft {
        cmd.arg("--draft");
    }

    let output = cmd.current_dir(&gh_dir).output()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Ok(stdout.trim().to_string());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    // The GraphQL createPullRequest mutation races with GitHub indexing a
    // newly-pushed branch, producing "Head sha can't be blank" / "No commits
    // between ..." errors. Fall back to the REST API which is more reliable.
    if stderr.contains("createPullRequest")
        || stderr.contains("Head sha can't be blank")
        || stderr.contains("No commits between")
    {
        return create_pull_request_rest(&gh_dir, title, body, base, &branch_name, draft);
    }

    Err(DirigentError::GitCommand(format!(
        "gh pr create failed: {}",
        stderr
    )))
}

/// Fallback: create a PR via the GitHub REST API (`POST /repos/{owner}/{repo}/pulls`).
fn create_pull_request_rest(
    gh_dir: &Path,
    title: &str,
    body: &str,
    base: &str,
    head: &str,
    draft: bool,
) -> crate::error::Result<String> {
    use std::process::Command;

    // Give GitHub a moment to index the newly-pushed branch.
    std::thread::sleep(std::time::Duration::from_secs(2));

    let slug_output = Command::new("gh")
        .args([
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "-q",
            ".nameWithOwner",
        ])
        .current_dir(gh_dir)
        .output()?;
    if !slug_output.status.success() {
        return Err(DirigentError::GitCommand(
            "cannot determine repo slug for REST API fallback".into(),
        ));
    }
    let slug = String::from_utf8_lossy(&slug_output.stdout)
        .trim()
        .to_string();
    if slug.is_empty() {
        return Err(DirigentError::GitCommand(
            "empty repo slug for REST API fallback".into(),
        ));
    }

    let mut cmd = Command::new("gh");
    cmd.arg("api")
        .arg("-X")
        .arg("POST")
        .arg(format!("repos/{}/pulls", slug))
        .arg("-f")
        .arg(format!("title={}", title))
        .arg("-f")
        .arg(format!("head={}", head))
        .arg("-f")
        .arg(format!("base={}", base))
        .arg("-f")
        .arg(format!("body={}", body))
        .arg("--jq")
        .arg(".html_url");
    if draft {
        cmd.arg("-F").arg("draft=true");
    }

    let output = cmd.current_dir(gh_dir).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DirigentError::GitCommand(format!(
            "gh pr create failed (REST fallback also failed): {}",
            stderr.trim()
        )));
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        return Err(DirigentError::GitCommand(
            "REST PR creation returned empty URL".into(),
        ));
    }
    Ok(url)
}

/// Create a pull request on Codeberg via the Forgejo REST API.
///
/// Requires a Codeberg access token in the `CODEBERG_TOKEN` (or `FORGEJO_TOKEN`)
/// environment variable. Forgejo has no draft flag on creation, so draft PRs are
/// marked with the conventional `WIP:` title prefix.
fn create_pull_request_codeberg(
    repo_path: &Path,
    remote: &RemoteInfo,
    title: &str,
    body: &str,
    base: &str,
    head: &str,
    draft: bool,
) -> crate::error::Result<String> {
    let token = forgejo::token(repo_path)
        .ok_or_else(|| DirigentError::GitCommand(forgejo::TOKEN_HELP.into()))?;

    let title = if draft {
        format!("WIP: {}", title)
    } else {
        title.to_string()
    };

    let api = format!("{}/pulls", remote.api_base());

    let client = forgejo::client(30)?;

    let resp = client
        .post(&api)
        .header("Authorization", format!("token {}", token))
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "title": title,
            "head": head,
            "base": base,
            "body": body,
        }))
        .send()
        .map_err(|e| DirigentError::GitCommand(format!("Codeberg API request failed: {}", e)))?;

    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        return Err(DirigentError::GitCommand(format!(
            "Codeberg PR creation failed (HTTP {}): {}",
            status.as_u16(),
            text.trim()
        )));
    }

    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| DirigentError::GitCommand(format!("invalid Codeberg API response: {}", e)))?;
    let url = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if url.is_empty() {
        return Err(DirigentError::GitCommand(
            "Codeberg PR created but no URL returned".into(),
        ));
    }
    Ok(url)
}

/// Query the Forgejo API for a Codeberg repo's default branch. Returns `None` on
/// any failure so callers can fall back to the generic detection path.
fn codeberg_default_branch(repo_path: &Path, remote: &RemoteInfo) -> Option<String> {
    let api = remote.api_base();
    let client = forgejo::client(15).ok()?;
    let mut req = client.get(&api).header("Accept", "application/json");
    if let Some(token) = forgejo::token(repo_path) {
        req = req.header("Authorization", format!("token {}", token));
    }
    let json: serde_json::Value = req.send().ok()?.json().ok()?;
    json.get("default_branch")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Get the default branch name for the repository (e.g. "main" or "master").
/// Falls back to "main" if detection fails.
pub(crate) fn get_default_branch(repo_path: &Path) -> String {
    use std::process::Command;

    // Codeberg (Forgejo) repos: ask the Forgejo API; `gh` cannot reach them.
    if let Some(remote) = forgejo::codeberg_remote(repo_path) {
        if let Some(branch) = codeberg_default_branch(repo_path, &remote) {
            return branch;
        }
    }

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
