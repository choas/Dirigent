use std::path::Path;
use std::process::Command;

use crate::error::DirigentError;

/// An item fetched from an external source, to be converted to a Cue.
#[derive(Debug, Clone)]
pub(crate) struct SourceItem {
    pub external_id: String,
    pub text: String,
    pub source_label: String,
}

/// Fetch items from a GitHub Issues source using the `gh` CLI.
pub(crate) fn fetch_github_issues(
    project_root: &Path,
    label_filter: Option<&str>,
    state: Option<&str>,
    source_label: &str,
) -> crate::error::Result<Vec<SourceItem>> {
    let mut cmd = Command::new("gh");
    cmd.arg("issue")
        .arg("list")
        .arg("--json")
        .arg("number,title,body,url")
        .arg("--limit")
        .arg("50");

    cmd.arg("--state").arg(state.unwrap_or("open"));

    if let Some(label) = label_filter {
        cmd.arg("--label").arg(label);
    }

    cmd.current_dir(project_root);

    let child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let timeout = std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS);
    let output = output_with_timeout(child, timeout)?;

    if !output.status.success() {
        return Err(DirigentError::Source(format!(
            "gh issue list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let issues: Vec<serde_json::Value> = serde_json::from_str(&json_str)?;

    Ok(issues
        .iter()
        .filter_map(|issue| {
            let number = issue.get("number")?.as_i64()?;
            let title = issue.get("title")?.as_str()?;
            let url = issue.get("url")?.as_str()?;
            let body = issue.get("body").and_then(|b| b.as_str()).unwrap_or("");

            let text = if body.is_empty() {
                format!("[#{}] {}", number, title)
            } else {
                format!("[#{}] {}\n\n{}", number, title, body)
            };

            Some(SourceItem {
                external_id: url.to_string(),
                text,
                source_label: source_label.to_string(),
            })
        })
        .collect())
}

/// Fetch messages from a Slack channel using the Slack Web API.
/// Requires a bot token (`xoxb-...`) and a channel ID.
pub(crate) fn fetch_slack_messages(
    token: &str,
    channel: &str,
    source_label: &str,
) -> crate::error::Result<Vec<SourceItem>> {
    if token.is_empty() {
        return Err(DirigentError::Source(
            "Slack bot token is empty".to_string(),
        ));
    }
    if channel.is_empty() {
        return Err(DirigentError::Source("Slack channel is empty".to_string()));
    }

    let child = Command::new("curl")
        .arg("-s")
        .arg("-H")
        .arg(format!("Authorization: Bearer {}", token))
        .arg(format!(
            "https://slack.com/api/conversations.history?channel={}&limit=50",
            channel,
        ))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let timeout = std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS);
    let output = output_with_timeout(child, timeout)?;

    if !output.status.success() {
        return Err(DirigentError::Source(format!(
            "curl failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let resp: serde_json::Value = serde_json::from_str(&json_str)?;

    if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let err = resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(DirigentError::Source(format!("Slack API error: {}", err)));
    }

    let messages = resp
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(messages
        .iter()
        .filter_map(|msg| {
            let text = msg.get("text")?.as_str()?;
            if text.trim().is_empty() {
                return None;
            }
            let ts = msg.get("ts")?.as_str()?;
            let user = msg
                .get("user")
                .and_then(|u| u.as_str())
                .unwrap_or("unknown");
            Some(SourceItem {
                external_id: format!("{}/{}", channel, ts),
                text: format!("[{}] {}", user, text),
                source_label: source_label.to_string(),
            })
        })
        .collect())
}

/// Fetch issues from a SonarQube instance using its Web API.
/// Reads the token from the project-root `.env` file if `token` is empty.
pub(crate) fn fetch_sonarqube_issues(
    project_root: &Path,
    host_url: &str,
    project_key: &str,
    token: &str,
    source_label: &str,
) -> crate::error::Result<Vec<SourceItem>> {
    if host_url.is_empty() {
        return Err(DirigentError::Source(
            "SonarQube host URL is empty".to_string(),
        ));
    }
    if project_key.is_empty() {
        return Err(DirigentError::Source(
            "SonarQube project key is empty".to_string(),
        ));
    }

    // Resolve token: use explicit token, fall back to .env SONAR_TOKEN
    let resolved_token = if token.is_empty() {
        load_env_var(project_root, "SONAR_TOKEN").unwrap_or_default()
    } else {
        token.to_string()
    };
    if resolved_token.is_empty() {
        return Err(DirigentError::Source(
            "SonarQube token is empty (set in source config or SONAR_TOKEN in .env)".to_string(),
        ));
    }

    let url = format!(
        "{}/api/issues/search?componentKeys={}&resolved=false&ps=100",
        host_url.trim_end_matches('/'),
        project_key,
    );

    let child = Command::new("curl")
        .arg("-s")
        .arg("-u")
        .arg(format!("{}:", resolved_token))
        .arg(&url)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let timeout = std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS);
    let output = output_with_timeout(child, timeout)?;

    if !output.status.success() {
        return Err(DirigentError::Source(format!(
            "SonarQube API request failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let resp: serde_json::Value = serde_json::from_str(&json_str)?;

    let issues = resp
        .get("issues")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(issues
        .iter()
        .filter_map(|issue| {
            let key = issue.get("key")?.as_str()?;
            let message = issue.get("message")?.as_str()?;
            let severity = issue
                .get("severity")
                .and_then(|s| s.as_str())
                .unwrap_or("INFO");
            let component = issue
                .get("component")
                .and_then(|c| c.as_str())
                .unwrap_or("");
            let line = issue.get("line").and_then(|l| l.as_u64()).unwrap_or(0);
            let rule = issue.get("rule").and_then(|r| r.as_str()).unwrap_or("");

            let text = if component.is_empty() {
                format!("[{}] {}", severity, message)
            } else if line > 0 {
                format!(
                    "[{}] {} ({}:{}, rule: {})",
                    severity, message, component, line, rule
                )
            } else {
                format!("[{}] {} ({}, rule: {})", severity, message, component, rule)
            };

            Some(SourceItem {
                external_id: key.to_string(),
                text,
                source_label: source_label.to_string(),
            })
        })
        .collect())
}

/// Load a variable from the `.env` file in the project root.
/// Returns `None` if the file doesn't exist or the key is not found.
fn load_env_var(project_root: &Path, key: &str) -> Option<String> {
    let env_path = project_root.join(".env");
    let content = std::fs::read_to_string(env_path).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix(key) {
            if let Some(value) = rest.strip_prefix('=') {
                // Strip surrounding quotes if present
                let value = value.trim();
                let value = value
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                    .unwrap_or(value);
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Maximum length for a custom source command string.
const MAX_COMMAND_LENGTH: usize = 4096;

/// Timeout for subprocess execution (seconds).
const SUBPROCESS_TIMEOUT_SECS: u64 = 60;

/// Shell metacharacters that could be used for injection.
const SHELL_METACHARACTERS: &[char] = &['`', '$', '!', ';', '&', '|', '<', '>', '(', ')'];

/// Validate a custom command string for safety.
/// Rejects null bytes, control characters (except common whitespace),
/// shell metacharacters, and excessively long commands.
fn validate_command(command: &str) -> Result<(), String> {
    if command.is_empty() {
        return Err("empty command".to_string());
    }
    if command.len() > MAX_COMMAND_LENGTH {
        return Err(format!(
            "command exceeds maximum length ({} > {})",
            command.len(),
            MAX_COMMAND_LENGTH
        ));
    }
    if command.contains('\0') {
        return Err("command contains null byte".to_string());
    }
    // Reject control characters other than tab/newline/carriage-return
    if let Some(pos) = command
        .chars()
        .position(|c| c.is_control() && c != '\t' && c != '\n' && c != '\r')
    {
        return Err(format!(
            "command contains control character at position {}",
            pos
        ));
    }
    // Reject shell metacharacters to prevent injection
    for &meta in SHELL_METACHARACTERS {
        if command.contains(meta) {
            return Err(format!("command contains shell metacharacter '{}'", meta));
        }
    }
    Ok(())
}

/// Run a command with a timeout. Returns the output or an IO error on timeout.
///
/// Reads stdout and stderr on separate threads to avoid deadlocking when the
/// child produces more output than the OS pipe buffer can hold (~64 KB on macOS).
fn output_with_timeout(
    mut child: std::process::Child,
    timeout: std::time::Duration,
) -> std::io::Result<std::process::Output> {
    use std::io::Read;

    // Take ownership of the pipe handles so we can read them on background threads.
    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();

    let stdout_thread = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        if let Some(mut out) = stdout_handle {
            out.read_to_end(&mut buf)?;
        }
        Ok(buf)
    });
    let stderr_thread = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        if let Some(mut err) = stderr_handle {
            err.read_to_end(&mut buf)?;
        }
        Ok(buf)
    });

    // Poll for process exit with a timeout.
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "subprocess timed out",
                ));
            }
            None => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    };

    let stdout = stdout_thread.join().unwrap_or_else(|_| Ok(Vec::new()))?;
    let stderr = stderr_thread.join().unwrap_or_else(|_| Ok(Vec::new()))?;

    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Parse JSON output from `gh api --paginate`.
/// When paginating, `gh` may concatenate multiple JSON arrays: `[...][...]`.
/// This function handles both a single valid array and concatenated arrays.
fn parse_paginated_json(raw: &str) -> Vec<serde_json::Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    // Fast path: valid single JSON array
    if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(trimmed) {
        return arr;
    }
    // Slow path: concatenated arrays — split on `][` and parse each chunk
    let mut items = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, ch) in trimmed.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    if let Ok(arr) =
                        serde_json::from_str::<Vec<serde_json::Value>>(&trimmed[start..=i])
                    {
                        items.extend(arr);
                    }
                    start = i + 1;
                }
            }
            _ => {}
        }
    }
    items
}

/// Fetch items from a custom command source.
/// The command should output JSON: either an array of objects or one object per line.
/// Each object should have "id" and "text" fields.
pub(crate) fn fetch_custom_command(
    project_root: &Path,
    command: &str,
    source_label: &str,
) -> crate::error::Result<Vec<SourceItem>> {
    if let Err(e) = validate_command(command) {
        return Err(DirigentError::Source(format!(
            "refusing to run custom source command: {}",
            e
        )));
    }

    let child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(project_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let timeout = std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS);
    let output = output_with_timeout(child, timeout)?;

    if !output.status.success() {
        return Err(DirigentError::Source(format!(
            "custom source command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    Ok(parse_source_json(&json_str, source_label))
}

/// Parse JSON output from a source command.
/// Supports JSON array or newline-delimited JSON objects.
/// Each object must have "id" and "text" fields.
fn parse_source_json(json_str: &str, source_label: &str) -> Vec<SourceItem> {
    // Try parsing as array first
    if let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(json_str) {
        return items
            .iter()
            .filter_map(|obj| parse_source_object(obj, source_label))
            .collect();
    }

    // Try newline-delimited JSON
    json_str
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let obj: serde_json::Value = serde_json::from_str(line).ok()?;
            parse_source_object(&obj, source_label)
        })
        .collect()
}

fn parse_source_object(obj: &serde_json::Value, source_label: &str) -> Option<SourceItem> {
    let id = obj.get("id")?.as_str()?;
    let text = obj.get("text")?.as_str()?;
    Some(SourceItem {
        external_id: id.to_string(),
        text: text.to_string(),
        source_label: source_label.to_string(),
    })
}

/// A finding extracted from a PR review comment.
#[derive(Debug, Clone)]
pub(crate) struct PrFinding {
    /// The file path the comment refers to (empty for general comments).
    pub file_path: String,
    /// The line number referenced (0 if not file-specific).
    pub line_number: usize,
    /// The finding text (reviewer comment body).
    pub text: String,
    /// A unique reference for deduplication (e.g. comment ID).
    pub external_id: String,
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

/// Check if a comment is a confirmation/addressed reply rather than a new finding.
/// CodeRabbit appends "✅ Confirmed as addressed" to the *original* comment body,
/// so we must search the entire text, not just the beginning.
fn is_confirmation_comment(body: &str) -> bool {
    let trimmed = body.trim();
    // Strip HTML comments to get visible text
    let without_html = trimmed
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.starts_with("<!--") && !t.ends_with("-->") && !t.is_empty()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let check = without_html.trim();
    // Check anywhere in the text — CodeRabbit edits the original comment to
    // append the confirmation marker at the bottom.
    check.contains("✅ Confirmed as addressed")
        || check.contains("Automated reply from [Dirigent]")
        || check.contains("<review_comment_addressed>")
        // Pure confirmation comments (standalone)
        || check.starts_with("Fixed in commit")
}

/// Check if a comment is an auto-generated summary (e.g. CodeRabbit walkthrough)
/// rather than an actionable finding.
fn is_auto_summary_comment(body: &str) -> bool {
    body.contains("<!-- walkthrough_start -->")
        || body.contains("auto-generated comment: summarize")
        || body.contains("auto-generated comment: release notes")
}

/// Extract the first "Prompt for AI Agents" block from a CodeRabbit comment.
fn extract_agent_prompt(body: &str) -> Option<String> {
    extract_all_agent_prompts(body).into_iter().next()
}

/// Check if a marker occurrence is part of the combined "all review comments" block.
fn is_combined_prompt_block(body: &str, abs_pos: usize) -> bool {
    let mut context_start = abs_pos.saturating_sub(60);
    // Ensure we land on a valid UTF-8 char boundary (emojis are multi-byte)
    while context_start > 0 && !body.is_char_boundary(context_start) {
        context_start -= 1;
    }
    body[context_start..abs_pos].contains("all review comments")
}

/// Extract the code-fenced prompt text that follows a marker position.
fn extract_code_block_after(text: &str) -> Option<String> {
    let code_start = text.find("```")?;
    let code_content = &text[code_start + 3..];
    // Skip the language identifier line if present
    let code_content = code_content
        .find('\n')
        .map_or(code_content, |nl| &code_content[nl + 1..]);
    let code_end = code_content.find("```")?;
    let prompt = code_content[..code_end].trim().to_string();
    if prompt.is_empty() {
        None
    } else {
        Some(prompt)
    }
}

/// Extract ALL individual "Prompt for AI Agents" blocks from a body.
/// Skips the combined "Prompt for all review comments" block.
fn extract_all_agent_prompts(body: &str) -> Vec<String> {
    let mut prompts = Vec::new();
    let marker = "Prompt for AI Agents";

    let mut search_from = 0;
    while let Some(rel_pos) = body[search_from..].find(marker) {
        let abs_pos = search_from + rel_pos;
        search_from = abs_pos + marker.len();

        if is_combined_prompt_block(body, abs_pos) {
            continue;
        }

        let after_marker = &body[abs_pos + marker.len()..];
        if let Some(prompt) = extract_code_block_after(after_marker) {
            prompts.push(prompt);
        }
    }

    prompts
}

/// Check whether a trimmed line is HTML markup that should be skipped.
fn is_skippable_markup(trimmed: &str) -> bool {
    trimmed.starts_with("<!--")
        || trimmed.starts_with("<sub")
        || trimmed.starts_with("</sub")
        || trimmed.starts_with("<blockquote")
        || trimmed.starts_with("</blockquote")
        || trimmed.starts_with("![")
}

/// Check whether a trimmed line is a severity/category label to skip.
fn is_severity_label(trimmed: &str) -> bool {
    trimmed.starts_with("_\u{26a0}") || trimmed.starts_with("_\u{1f41b}")
}

/// Truncate a string to at most `max_len` bytes on a valid UTF-8 boundary,
/// appending "..." if truncated.
fn truncate_with_ellipsis(s: &mut String, max_len: usize) {
    if s.len() <= max_len {
        return;
    }
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s.push_str("...");
}

/// Check whether a trimmed line opens a `<details>` or `<summary>` block.
fn is_details_open(trimmed: &str) -> bool {
    trimmed.starts_with("<details") || trimmed.starts_with("<summary")
}

/// Check whether a trimmed line should be ignored (markup, labels, or blank).
fn is_ignorable_line(trimmed: &str) -> bool {
    is_skippable_markup(trimmed) || is_severity_label(trimmed) || trimmed.is_empty()
}

/// Extract a clean finding text from a review comment body.
/// Strips HTML tags, diff blocks, and suggestion blocks to get the core message.
fn extract_finding_text(body: &str) -> String {
    let mut result = String::new();
    let mut in_details = false;
    let mut in_code_block = false;

    for line in body.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }

        if is_details_open(trimmed) {
            in_details = true;
            continue;
        }
        if trimmed == "</details>" {
            in_details = false;
            continue;
        }
        if in_details {
            continue;
        }

        if is_ignorable_line(trimmed) {
            continue;
        }

        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(trimmed);
    }

    truncate_with_ellipsis(&mut result, 2000);
    result
}

/// Parse a PR source_ref to extract (pr_number, comment_type, comment_id).
/// Formats: "pr<N>:comment:<ID>", "pr<N>:issue_comment:<ID>",
///          "pr<N>:review:<ID>" or "pr<N>:review:<ID>_<sub>"
pub(crate) fn parse_pr_source_ref(source_ref: &str) -> Option<(u32, &str, u64)> {
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

/// Reply to a PR inline review comment via `gh api`.
pub(crate) fn reply_to_pr_review_comment(
    project_root: &Path,
    pr_number: u32,
    comment_id: u64,
    body: &str,
) -> crate::error::Result<()> {
    let mut cmd = Command::new("gh");
    cmd.arg("api")
        .arg("--method")
        .arg("POST")
        .arg(format!(
            "repos/{{owner}}/{{repo}}/pulls/{}/comments/{}/replies",
            pr_number, comment_id
        ))
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
            "Failed to reply to PR comment: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

/// Post a new comment on a PR (issue-level) via `gh api`.
pub(crate) fn comment_on_pr(
    project_root: &Path,
    pr_number: u32,
    body: &str,
) -> crate::error::Result<()> {
    let mut cmd = Command::new("gh");
    cmd.arg("api")
        .arg("--method")
        .arg("POST")
        .arg(format!(
            "repos/{{owner}}/{{repo}}/issues/{}/comments",
            pr_number
        ))
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
            "Failed to comment on PR: {}",
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
            reply_to_pr_review_comment(project_root, pr_number, comment_id, &body)?;
        }
        "issue_comment" | "review" => {
            // Can't reply directly to issue/review comments; post a new comment mentioning it
            comment_on_pr(project_root, pr_number, &body)?;
        }
        _ => return Ok(false),
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- validate_command --

    #[test]
    fn validate_command_accepts_simple() {
        assert!(validate_command("echo hello").is_ok());
    }

    #[test]
    fn validate_command_rejects_empty() {
        assert!(validate_command("").is_err());
    }

    #[test]
    fn validate_command_rejects_null_byte() {
        assert!(validate_command("echo\0hello").is_err());
    }

    #[test]
    fn validate_command_rejects_too_long() {
        let long = "a".repeat(MAX_COMMAND_LENGTH + 1);
        assert!(validate_command(&long).is_err());
    }

    #[test]
    fn validate_command_rejects_control_chars() {
        assert!(validate_command("echo \x01 hi").is_err());
    }

    #[test]
    fn validate_command_rejects_shell_metacharacters() {
        for &meta in SHELL_METACHARACTERS {
            let cmd = format!("echo {}foo", meta);
            assert!(validate_command(&cmd).is_err(), "should reject '{}'", meta);
        }
    }

    #[test]
    fn validate_command_allows_safe_characters() {
        assert!(validate_command("python3 script.py --flag=value 'arg' \"arg2\"").is_ok());
        assert!(validate_command("curl https://example.com/api").is_ok());
    }

    // -- parse_source_json --

    #[test]
    fn parse_json_array() {
        let json = r#"[{"id":"1","text":"first"},{"id":"2","text":"second"}]"#;
        let items = parse_source_json(json, "test");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].external_id, "1");
        assert_eq!(items[0].text, "first");
        assert_eq!(items[0].source_label, "test");
    }

    #[test]
    fn parse_ndjson() {
        let json = "{\"id\":\"a\",\"text\":\"alpha\"}\n{\"id\":\"b\",\"text\":\"beta\"}\n";
        let items = parse_source_json(json, "src");
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].external_id, "b");
    }

    #[test]
    fn parse_empty_json() {
        let items = parse_source_json("[]", "test");
        assert!(items.is_empty());
    }

    #[test]
    fn parse_missing_fields_skipped() {
        let json = r#"[{"id":"1"},{"id":"2","text":"ok"}]"#;
        let items = parse_source_json(json, "test");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].external_id, "2");
    }

    // -- parse_source_object --

    #[test]
    fn parse_source_object_valid() {
        let obj: serde_json::Value = serde_json::from_str(r#"{"id":"x","text":"hello"}"#).unwrap();
        let item = parse_source_object(&obj, "lbl").unwrap();
        assert_eq!(item.external_id, "x");
        assert_eq!(item.text, "hello");
        assert_eq!(item.source_label, "lbl");
    }

    #[test]
    fn parse_source_object_missing_id() {
        let obj: serde_json::Value = serde_json::from_str(r#"{"text":"hello"}"#).unwrap();
        assert!(parse_source_object(&obj, "lbl").is_none());
    }

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

    // -- is_confirmation_comment --

    #[test]
    fn confirmation_comment_with_checkmark() {
        let body = "Some finding text\n\n✅ Confirmed as addressed by @user";
        assert!(is_confirmation_comment(body));
    }

    #[test]
    fn confirmation_comment_in_html_stripped() {
        // Confirmation marker as visible text (not in HTML comment) should be detected
        let body = "Finding text\n<!-- comment -->\n✅ Confirmed as addressed\n<!-- end -->";
        assert!(is_confirmation_comment(body));
    }

    #[test]
    fn non_confirmation_comment() {
        let body = "**Bug found:** This function panics on empty input.";
        assert!(!is_confirmation_comment(body));
    }

    // -- is_auto_summary_comment --

    #[test]
    fn auto_summary_walkthrough() {
        let body = "<!-- walkthrough_start -->\n## Walkthrough\nSome changes...";
        assert!(is_auto_summary_comment(body));
    }

    #[test]
    fn auto_summary_not_review() {
        // Review status comment is NOT an auto-summary (it contains actual findings)
        let body = "<!-- This is an auto-generated comment by CodeRabbit for review status -->";
        assert!(!is_auto_summary_comment(body));
    }

    // -- extract_all_agent_prompts --

    #[test]
    fn extract_single_agent_prompt() {
        let body = r#"Some finding text

<details>
<summary>🤖 Prompt for AI Agents</summary>

```
Fix the bug in src/main.rs at line 42.
```

</details>"#;
        let prompts = extract_all_agent_prompts(body);
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains("Fix the bug"));
    }

    #[test]
    fn extract_multiple_agent_prompts_skips_combined() {
        let body = r#"<details>
<summary>🤖 Prompt for AI Agents</summary>

```
First finding.
```

</details>

<details>
<summary>🤖 Prompt for AI Agents</summary>

```
Second finding.
```

</details>

<details>
<summary>🤖 Prompt for all review comments with AI agents</summary>

```
Combined prompt (should be skipped).
```

</details>"#;
        let prompts = extract_all_agent_prompts(body);
        assert_eq!(prompts.len(), 2);
        assert!(prompts[0].contains("First finding"));
        assert!(prompts[1].contains("Second finding"));
    }

    #[test]
    fn extract_agent_prompt_with_emoji_context() {
        // Emojis near the marker shouldn't cause panics
        let body = "🧹🔧🐛 Some context\n\n<summary>🤖 Prompt for AI Agents</summary>\n\n```\nFix it.\n```";
        let prompts = extract_all_agent_prompts(body);
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains("Fix it"));
    }

    // -- extract_finding_text --

    #[test]
    fn extract_finding_text_strips_code_blocks() {
        let body = "**Bug:** Something is wrong.\n\n```rust\nlet x = 1;\n```\n\nPlease fix.";
        let text = extract_finding_text(body);
        assert!(text.contains("Bug:"));
        assert!(text.contains("Please fix"));
        assert!(!text.contains("let x"));
    }

    #[test]
    fn extract_finding_text_strips_details() {
        let body =
            "Finding.\n<details>\n<summary>Details</summary>\nHidden content\n</details>\nVisible.";
        let text = extract_finding_text(body);
        assert!(text.contains("Finding"));
        assert!(text.contains("Visible"));
        assert!(!text.contains("Hidden"));
    }
}
