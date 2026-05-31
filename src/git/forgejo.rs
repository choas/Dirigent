//! Shared Forgejo (Codeberg) REST API helpers.
//!
//! Codeberg runs Forgejo, which the `gh` CLI cannot talk to. These primitives
//! detect a Codeberg remote, parse its owner/repo, and build authenticated
//! requests against the Forgejo `/api/v1` surface. They are shared between PR
//! creation (`git::pr`) and PR-finding import / feedback (`sources`).

use std::path::Path;
use std::time::Duration;

use git2::Repository;

use crate::error::DirigentError;

/// A git hosting remote parsed into its host, owner and repository name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteInfo {
    pub host: String,
    pub owner: String,
    pub repo: String,
}

impl RemoteInfo {
    /// Whether this remote is hosted on Codeberg (Forgejo).
    pub(crate) fn is_codeberg(&self) -> bool {
        self.host.eq_ignore_ascii_case("codeberg.org")
    }

    /// Base URL of the Forgejo REST API for this repository, e.g.
    /// `https://codeberg.org/api/v1/repos/owner/repo`.
    pub(crate) fn api_base(&self) -> String {
        format!(
            "https://{}/api/v1/repos/{}/{}",
            self.host, self.owner, self.repo
        )
    }
}

/// Human-readable hint shown when a Codeberg token is required but missing.
pub(crate) const TOKEN_HELP: &str = "Codeberg requires an access token. Create one at \
     Codeberg > Settings > Applications and set it in the CODEBERG_TOKEN (or FORGEJO_TOKEN) \
     environment variable.";

/// Read the URL of the `origin` remote (falling back to the first configured remote).
fn origin_remote_url(repo: &Repository) -> Option<String> {
    if let Ok(remote) = repo.find_remote("origin") {
        if let Some(url) = remote.url() {
            return Some(url.to_string());
        }
    }
    let remotes = repo.remotes().ok()?;
    let name = remotes.iter().flatten().next()?;
    let remote = repo.find_remote(name).ok()?;
    remote.url().map(|u| u.to_string())
}

/// Parse a git remote URL into host/owner/repo.
///
/// Handles the common forms:
///   - `https://host/owner/repo.git`
///   - `ssh://git@host/owner/repo.git`
///   - `git@host:owner/repo.git`
pub(crate) fn parse_remote_url(url: &str) -> Option<RemoteInfo> {
    let url = url.trim();

    // Reduce every form to `host/owner/repo` (path may still carry a leading slash).
    let rest = if let Some(idx) = url.find("://") {
        // scheme://[user@]host/owner/repo
        let after = &url[idx + 3..];
        after.rsplit('@').next().unwrap_or(after).to_string()
    } else if let Some(at) = url.find('@') {
        // user@host:owner/repo  ->  host/owner/repo
        url[at + 1..].replacen(':', "/", 1)
    } else {
        url.to_string()
    };

    let rest = rest.strip_suffix(".git").unwrap_or(&rest);
    let mut segments = rest.split('/').filter(|s| !s.is_empty());
    let host = segments.next()?.to_string();
    let owner = segments.next()?.to_string();
    let repo = segments.next()?.to_string();
    if host.is_empty() || owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(RemoteInfo { host, owner, repo })
}

/// Return the parsed `origin` remote for `repo_path`, if any.
pub(crate) fn origin_remote(repo_path: &Path) -> Option<RemoteInfo> {
    let repo = Repository::discover(repo_path).ok()?;
    origin_remote_url(&repo)
        .as_deref()
        .and_then(parse_remote_url)
}

/// Return the remote for `repo_path` only when it is hosted on Codeberg.
///
/// This is the single entry point callers use to decide whether to route a PR
/// operation through the Forgejo API instead of `gh`.
pub(crate) fn codeberg_remote(repo_path: &Path) -> Option<RemoteInfo> {
    let remote = origin_remote(repo_path)?;
    remote.is_codeberg().then_some(remote)
}

/// Read a Codeberg/Forgejo access token.
///
/// Resolution order, first non-empty hit wins: `.Dirigent/.env`, then `.env` in
/// the project root, then the process environment. This mirrors how other
/// source integrations resolve their tokens, so a token placed in
/// `.Dirigent/.env` works for PR operations too — not only one exported into
/// Dirigent's own environment.
pub(crate) fn token(project_root: &Path) -> Option<String> {
    for key in ["CODEBERG_TOKEN", "FORGEJO_TOKEN"] {
        let from_file = crate::claude::load_env_var_with_dirigent_fallback(project_root, key);
        let resolved = from_file.or_else(|| std::env::var(key).ok());
        if let Some(v) = resolved.filter(|t| !t.trim().is_empty()) {
            return Some(v);
        }
    }
    None
}

/// Build a blocking HTTP client with the given timeout.
pub(crate) fn client(timeout_secs: u64) -> crate::error::Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| DirigentError::GitCommand(format!("HTTP client error: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_remote() {
        let r = parse_remote_url("https://codeberg.org/lars/dirigent.git").unwrap();
        assert_eq!(r.host, "codeberg.org");
        assert_eq!(r.owner, "lars");
        assert_eq!(r.repo, "dirigent");
        assert!(r.is_codeberg());
    }

    #[test]
    fn parses_scp_style_remote() {
        let r = parse_remote_url("git@codeberg.org:lars/dirigent.git").unwrap();
        assert_eq!(r.host, "codeberg.org");
        assert_eq!(r.owner, "lars");
        assert_eq!(r.repo, "dirigent");
    }

    #[test]
    fn parses_ssh_scheme_remote() {
        let r = parse_remote_url("ssh://git@codeberg.org/lars/dirigent.git").unwrap();
        assert_eq!(r.host, "codeberg.org");
        assert_eq!(r.owner, "lars");
        assert_eq!(r.repo, "dirigent");
    }

    #[test]
    fn parses_remote_without_git_suffix() {
        let r = parse_remote_url("https://codeberg.org/lars/dirigent").unwrap();
        assert_eq!(r.repo, "dirigent");
    }

    #[test]
    fn github_is_not_codeberg() {
        let r = parse_remote_url("git@github.com:lars/dirigent.git").unwrap();
        assert!(!r.is_codeberg());
    }

    #[test]
    fn rejects_incomplete_url() {
        assert!(parse_remote_url("https://codeberg.org/lars").is_none());
    }

    #[test]
    fn api_base_points_at_forgejo_v1() {
        let r = parse_remote_url("https://codeberg.org/lars/dirigent.git").unwrap();
        assert_eq!(
            r.api_base(),
            "https://codeberg.org/api/v1/repos/lars/dirigent"
        );
    }
}
