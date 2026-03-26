use std::path::Path;
use std::process::Command;

use crate::error::DirigentError;

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
    Ok(parse_paginated_json(&json_str))
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
        let path = comment.get("path").and_then(|p| p.as_str()).unwrap_or("");
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

/// Process issue-level comments into findings.
fn process_issue_comments(comments: &[serde_json::Value], pr_number: u32) -> Vec<PrFinding> {
    let mut findings = Vec::new();
    for comment in comments {
        let body = comment.get("body").and_then(|b| b.as_str()).unwrap_or("");
        if should_skip_comment(body) {
            continue;
        }
        let comment_id = comment.get("id").and_then(|id| id.as_u64()).unwrap_or(0);
        if let Some(finding_text) = finding_text_from_body(body) {
            findings.push(PrFinding {
                file_path: String::new(),
                line_number: 0,
                text: finding_text,
                external_id: format!("pr{}:issue_comment:{}", pr_number, comment_id),
            });
        }
    }
    findings
}

/// Process PR review bodies into findings.
fn process_reviews(reviews: &[serde_json::Value], pr_number: u32) -> Vec<PrFinding> {
    let mut findings = Vec::new();
    for review in reviews {
        let body = review.get("body").and_then(|b| b.as_str()).unwrap_or("");
        if should_skip_comment(body) {
            continue;
        }
        let review_id = review.get("id").and_then(|id| id.as_u64()).unwrap_or(0);
        let prompts = extract_all_agent_prompts(body);
        if prompts.is_empty() {
            if let Some(finding_text) = finding_text_from_body(body) {
                findings.push(PrFinding {
                    file_path: String::new(),
                    line_number: 0,
                    text: finding_text,
                    external_id: format!("pr{}:review:{}", pr_number, review_id),
                });
            }
        } else {
            for (i, prompt) in prompts.iter().enumerate() {
                findings.push(PrFinding {
                    file_path: String::new(),
                    line_number: 0,
                    text: prompt.clone(),
                    external_id: format!("pr{}:review:{}_{}", pr_number, review_id, i),
                });
            }
        }
    }
    findings
}

/// Fetch PR review comments using `gh` CLI and parse actionable findings.
/// Returns findings from inline review comments (e.g. CodeRabbit).
pub(crate) fn fetch_pr_findings(
    project_root: &Path,
    pr_number: u32,
) -> crate::error::Result<Vec<PrFinding>> {
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
        findings.extend(process_issue_comments(&issue_comments, pr_number));
    }

    // Also fetch PR reviews (e.g. CodeRabbit re-reviews with nitpick findings in the body)
    if let Ok(reviews) = gh_api_paginated(
        project_root,
        &format!("repos/{{owner}}/{{repo}}/pulls/{}/reviews", pr_number),
    ) {
        findings.extend(process_reviews(&reviews, pr_number));
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
