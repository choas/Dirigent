use std::path::Path;
use std::process::Command;

use crate::error::DirigentError;

use super::custom::{output_with_timeout, SUBPROCESS_TIMEOUT_SECS};

/// Parse a PR source_ref to extract (pr_number, comment_type, comment_id).
/// Formats: "pr<N>:comment:<ID>", "pr<N>:issue_comment:<ID>",
///          "pr<N>:review:<ID>" or "pr<N>:review:<ID>_<sub>"
fn parse_pr_source_ref(source_ref: &str) -> Option<(u32, &str, u64)> {
    let parts: Vec<&str> = source_ref.splitn(3, ':').collect();
    if parts.len() != 3 {
        return None;
    }
    let pr_num = parts[0].strip_prefix("pr")?.parse().ok()?;
    let comment_type = parts[1]; // "comment", "issue_comment", or "review"
                                 // Strip the "_<sub>" suffix for review findings (e.g. "123_0" → "123")
    let id_str = parts[2].split('_').next().unwrap_or(parts[2]);
    let comment_id = id_str.parse().ok()?;
    Some((pr_num, comment_type, comment_id))
}

/// Post to a GitHub API endpoint via `gh api --method POST`.
fn gh_api_post(
    project_root: &Path,
    endpoint: &str,
    body: &str,
    error_context: &str,
) -> crate::error::Result<()> {
    let mut cmd = Command::new("gh");
    cmd.arg("api")
        .arg("--method")
        .arg("POST")
        .arg(endpoint)
        .arg("-f")
        .arg(format!("body={}", body))
        .current_dir(project_root);

    let child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let timeout = std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS);
    let output = output_with_timeout(child, timeout)?;

    if !output.status.success() {
        return Err(DirigentError::Source(format!(
            "{}: {}",
            error_context,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

/// Notify a PR comment that a finding has been addressed.
/// Returns Ok(true) if a reply was posted, Ok(false) if the source_ref was not a PR ref.
pub(crate) fn notify_pr_finding_fixed(
    project_root: &Path,
    source_ref: &str,
    commit_hash: &str,
) -> crate::error::Result<bool> {
    let (pr_number, comment_type, comment_id) = match parse_pr_source_ref(source_ref) {
        Some(parsed) => parsed,
        None => return Ok(false),
    };

    let body = format!(
        "Fixed in commit {}.\n\n*Automated reply from [Dirigent](https://github.com/choas/Dirigent)*",
        commit_hash
    );

    match comment_type {
        "comment" => {
            let endpoint = format!(
                "repos/{{owner}}/{{repo}}/pulls/{}/comments/{}/replies",
                pr_number, comment_id
            );
            gh_api_post(
                project_root,
                &endpoint,
                &body,
                "Failed to reply to PR comment",
            )?;
        }
        "issue_comment" | "review" => {
            // Can't reply directly to issue/review comments; post a new comment mentioning it
            let endpoint = format!("repos/{{owner}}/{{repo}}/issues/{}/comments", pr_number);
            gh_api_post(project_root, &endpoint, &body, "Failed to comment on PR")?;
        }
        _ => return Ok(false),
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_pr_source_ref --

    #[test]
    fn parse_pr_source_ref_review_comment() {
        let (pr, kind, id) = parse_pr_source_ref("pr3:comment:12345").unwrap();
        assert_eq!(pr, 3);
        assert_eq!(kind, "comment");
        assert_eq!(id, 12345);
    }

    #[test]
    fn parse_pr_source_ref_issue_comment() {
        let (pr, kind, id) = parse_pr_source_ref("pr42:issue_comment:999").unwrap();
        assert_eq!(pr, 42);
        assert_eq!(kind, "issue_comment");
        assert_eq!(id, 999);
    }

    #[test]
    fn parse_pr_source_ref_invalid() {
        assert!(parse_pr_source_ref("not_a_pr_ref").is_none());
        assert!(parse_pr_source_ref("pr:comment:1").is_none());
        assert!(parse_pr_source_ref("").is_none());
    }

    #[test]
    fn parse_pr_source_ref_review() {
        let (pr, kind, id) = parse_pr_source_ref("pr1:review:3986437510").unwrap();
        assert_eq!(pr, 1);
        assert_eq!(kind, "review");
        assert_eq!(id, 3986437510);
    }

    #[test]
    fn parse_pr_source_ref_review_with_sub_index() {
        let (pr, kind, id) = parse_pr_source_ref("pr1:review:3986437510_2").unwrap();
        assert_eq!(pr, 1);
        assert_eq!(kind, "review");
        assert_eq!(id, 3986437510);
    }
}
