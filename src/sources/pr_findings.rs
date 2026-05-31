use std::path::Path;
use std::process::Command;

use crate::error::DirigentError;
use crate::git::forgejo::{self, RemoteInfo};

use super::custom::{output_with_timeout, parse_paginated_json, SUBPROCESS_TIMEOUT_SECS};
use super::finding_text::extract_finding_text;
use super::pr_comments::{
    extract_agent_prompt, extract_all_agent_prompts, is_auto_summary_comment,
    is_confirmation_comment,
};
use super::types::PrFinding;

/// Strip the trailing `[Hint: use \`gh pr view ...\`]` suffix that older versions
/// appended to finding text.  Used to normalise the comparison so that removing
/// the hint does not cause every existing cue to be detected as "text changed".
pub(crate) fn strip_pr_context_hint(text: &str) -> &str {
    let t = text.trim_end();
    if let Some(pos) = t.rfind("\n\n[Hint: use `gh pr view") {
        t[..pos].trim_end()
    } else {
        t
    }
}

/// Run a `gh api` command with pagination and return parsed JSON values.
fn gh_api_paginated(
    project_root: &Path,
    endpoint: &str,
) -> crate::error::Result<Vec<serde_json::Value>> {
    let mut cmd = Command::new("gh");
    cmd.arg("api")
        .arg(endpoint)
        .arg("--paginate")
        .current_dir(project_root);

    let child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let timeout = std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS);
    let output = output_with_timeout(child, timeout)?;

    if !output.status.success() {
        return Err(DirigentError::Source(format!(
            "gh api {} failed: {}",
            endpoint,
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    parse_paginated_json(&json_str)
}

/// Determine whether a comment body should be skipped (empty, confirmation, or summary).
fn should_skip_comment(body: &str) -> bool {
    body.trim().is_empty() || is_confirmation_comment(body) || is_auto_summary_comment(body)
}

/// Extract finding text from a comment body, preferring agent prompts.
fn finding_text_from_body(body: &str) -> Option<String> {
    let text = extract_agent_prompt(body).unwrap_or_else(|| extract_finding_text(body));
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Process inline review comments into findings.
fn process_inline_comments(comments: &[serde_json::Value], pr_number: u32) -> Vec<PrFinding> {
    let mut findings = Vec::new();
    for comment in comments {
        if comment.get("in_reply_to_id").is_some_and(|v| !v.is_null()) {
            continue;
        }
        let body = comment.get("body").and_then(|b| b.as_str()).unwrap_or("");
        if should_skip_comment(body) {
            continue;
        }
        let raw_path = comment.get("path").and_then(|p| p.as_str()).unwrap_or("");
        // Reject paths with traversal components or absolute paths from the API.
        let path = if std::path::Path::new(raw_path).components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        }) {
            ""
        } else {
            raw_path
        };
        let line = comment
            .get("line")
            .or_else(|| comment.get("original_line"))
            .and_then(|l| l.as_u64())
            .unwrap_or(0) as usize;
        // Clamp to 1 when file_path is present: the app uses 1-based line numbers,
        // and 0 would create an invalid navigation target.
        let line = if !path.is_empty() && line == 0 {
            1
        } else {
            line
        };
        let comment_id = comment.get("id").and_then(|id| id.as_u64()).unwrap_or(0);
        if let Some(finding_text) = finding_text_from_body(body) {
            findings.push(PrFinding {
                file_path: path.to_string(),
                line_number: line,
                text: finding_text,
                external_id: format!("pr{}:comment:{}", pr_number, comment_id),
            });
        }
    }
    findings
}

/// Create a non-file-specific finding.
fn global_finding(text: String, external_id: String) -> PrFinding {
    PrFinding {
        file_path: String::new(),
        line_number: 0,
        text,
        external_id,
    }
}

/// Process a list of comment/review JSON objects into findings.
/// `id_prefix` is used to construct the external_id (e.g. "issue_comment" or "review").
/// When `extract_prompts` is true, multi-prompt extraction is attempted on review bodies.
fn process_body_comments(
    comments: &[serde_json::Value],
    pr_number: u32,
    id_prefix: &str,
    extract_prompts: bool,
) -> Vec<PrFinding> {
    let mut findings = Vec::new();
    for comment in comments {
        let body = comment.get("body").and_then(|b| b.as_str()).unwrap_or("");
        if should_skip_comment(body) {
            continue;
        }
        let comment_id = comment.get("id").and_then(|id| id.as_u64()).unwrap_or(0);

        if extract_prompts {
            let prompts = extract_all_agent_prompts(body);
            if !prompts.is_empty() {
                for (i, prompt) in prompts.iter().enumerate() {
                    findings.push(global_finding(
                        prompt.clone(),
                        format!("pr{}:{}:{}_{}", pr_number, id_prefix, comment_id, i),
                    ));
                }
                continue;
            }
        }

        if let Some(finding_text) = finding_text_from_body(body) {
            findings.push(global_finding(
                finding_text,
                format!("pr{}:{}:{}", pr_number, id_prefix, comment_id),
            ));
        }
    }
    findings
}

/// GET a paginated Forgejo (Codeberg) list endpoint and return all JSON items.
///
/// `url` is the endpoint without paging params; this walks `?page=N` until a
/// short page signals the end. A token is sent when available so private repos
/// and rate limits work, but public repos read fine without one.
fn forgejo_get_all(
    client: &reqwest::blocking::Client,
    token: Option<&str>,
    url: &str,
) -> crate::error::Result<Vec<serde_json::Value>> {
    const LIMIT: usize = 50;
    let mut all = Vec::new();
    for page in 1..=100u32 {
        let sep = if url.contains('?') { '&' } else { '?' };
        let paged = format!("{}{}page={}&limit={}", url, sep, page, LIMIT);
        let mut req = client.get(&paged).header("Accept", "application/json");
        if let Some(t) = token {
            req = req.header("Authorization", format!("token {}", t));
        }
        let resp = req
            .send()
            .map_err(|e| DirigentError::Source(format!("Codeberg API request failed: {}", e)))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        if !status.is_success() {
            return Err(DirigentError::Source(format!(
                "Codeberg API GET {} failed (HTTP {}): {}",
                url,
                status.as_u16(),
                text.trim()
            )));
        }
        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| DirigentError::Source(format!("invalid Codeberg API response: {}", e)))?;
        let batch = json.as_array().cloned().unwrap_or_default();
        let n = batch.len();
        all.extend(batch);
        if n < LIMIT {
            break;
        }
    }
    Ok(all)
}

/// Fetch PR review findings from Codeberg via the Forgejo REST API.
///
/// Forgejo exposes inline comments per-review (rather than a single flat
/// `pulls/{n}/comments` list like GitHub), so this walks each review's comments
/// in addition to the review bodies and issue-level comments.
fn fetch_pr_findings_codeberg(
    project_root: &Path,
    remote: &RemoteInfo,
    pr_number: u32,
) -> crate::error::Result<Vec<PrFinding>> {
    let client = forgejo::client(SUBPROCESS_TIMEOUT_SECS)?;
    let token = forgejo::token(project_root);
    let token = token.as_deref();
    let base = remote.api_base();
    let mut findings = Vec::new();

    // Issue-level comments (general PR discussion, e.g. bot summaries).
    if let Ok(issue_comments) = forgejo_get_all(
        &client,
        token,
        &format!("{}/issues/{}/comments", base, pr_number),
    ) {
        findings.extend(process_body_comments(
            &issue_comments,
            pr_number,
            "issue_comment",
            false,
        ));
    }

    // PR reviews, plus the inline code comments attached to each review.
    if let Ok(reviews) = forgejo_get_all(
        &client,
        token,
        &format!("{}/pulls/{}/reviews", base, pr_number),
    ) {
        findings.extend(process_body_comments(&reviews, pr_number, "review", true));
        for review in &reviews {
            let Some(review_id) = review.get("id").and_then(|v| v.as_u64()) else {
                continue;
            };
            if let Ok(comments) = forgejo_get_all(
                &client,
                token,
                &format!(
                    "{}/pulls/{}/reviews/{}/comments",
                    base, pr_number, review_id
                ),
            ) {
                findings.extend(process_inline_comments(&comments, pr_number));
            }
        }
    }

    Ok(findings)
}

/// Fetch PR review comments and parse actionable findings.
///
/// GitHub remotes use the `gh` CLI; Codeberg (Forgejo) remotes use the Forgejo
/// REST API directly. Returns findings from inline review comments (e.g. CodeRabbit).
pub(crate) fn fetch_pr_findings(
    project_root: &Path,
    pr_number: u32,
) -> crate::error::Result<Vec<PrFinding>> {
    // Route Codeberg (Forgejo) remotes through the Forgejo API; `gh` is GitHub-only.
    if let Some(remote) = forgejo::codeberg_remote(project_root) {
        return fetch_pr_findings_codeberg(project_root, &remote, pr_number);
    }

    let mut findings = Vec::new();

    // Fetch inline review comments (code-level comments, e.g. from CodeRabbit)
    let comments = gh_api_paginated(
        project_root,
        &format!("repos/{{owner}}/{{repo}}/pulls/{}/comments", pr_number),
    )?;
    findings.extend(process_inline_comments(&comments, pr_number));

    // Also fetch issue-level comments (general PR comments, e.g. CodeRabbit summary)
    if let Ok(issue_comments) = gh_api_paginated(
        project_root,
        &format!("repos/{{owner}}/{{repo}}/issues/{}/comments", pr_number),
    ) {
        findings.extend(process_body_comments(
            &issue_comments,
            pr_number,
            "issue_comment",
            false,
        ));
    }

    // Also fetch PR reviews (e.g. CodeRabbit re-reviews with nitpick findings in the body)
    if let Ok(reviews) = gh_api_paginated(
        project_root,
        &format!("repos/{{owner}}/{{repo}}/pulls/{}/reviews", pr_number),
    ) {
        findings.extend(process_body_comments(&reviews, pr_number, "review", true));
    }

    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_pr_context_hint_removes_trailing_hint() {
        let with_hint = "Fix the bug\n\n[Hint: use `gh pr view 7 --comments` to read the full PR discussion for additional context.]";
        assert_eq!(strip_pr_context_hint(with_hint), "Fix the bug");
    }

    #[test]
    fn strip_pr_context_hint_preserves_text_without_hint() {
        assert_eq!(strip_pr_context_hint("Fix the bug"), "Fix the bug");
        assert_eq!(strip_pr_context_hint(""), "");
    }

    #[test]
    fn strip_pr_context_hint_normalises_old_vs_new_findings() {
        // Simulates the scenario: old cue in DB has the hint suffix,
        // new finding from API does not.  After stripping, both should match.
        let old_db_text = "Review comment body\n\n[Hint: use `gh pr view 5 --comments` to read the full PR discussion for additional context.]";
        let new_finding_text = "Review comment body";
        assert_eq!(
            strip_pr_context_hint(old_db_text),
            strip_pr_context_hint(new_finding_text),
        );
    }
}
