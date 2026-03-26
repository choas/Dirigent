use std::path::Path;
use std::process::Command;

use crate::error::DirigentError;

use super::custom::{output_with_timeout, SUBPROCESS_TIMEOUT_SECS};
use super::types::SourceItem;

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

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS))
        .build()
        .map_err(|e| DirigentError::Source(format!("HTTP client error: {e}")))?;

    let resp: serde_json::Value = client
        .get("https://slack.com/api/conversations.history")
        .query(&[("channel", channel), ("limit", "50")])
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .map_err(|e| DirigentError::Source(format!("Slack request failed: {e}")))?
        .json()
        .map_err(|e| DirigentError::Source(format!("Slack response parse error: {e}")))?;

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
/// The caller is expected to resolve the token (e.g. via `resolve_source_token`).
pub(crate) fn fetch_sonarqube_issues(
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
    if token.is_empty() {
        return Err(DirigentError::Source(
            "SonarQube token is empty (set in source config or SONAR_TOKEN in .env)".to_string(),
        ));
    }

    let url = format!(
        "{}/api/issues/search?componentKeys={}&resolved=false&ps=100",
        host_url.trim_end_matches('/'),
        project_key,
    );

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS))
        .build()
        .map_err(|e| DirigentError::Source(format!("HTTP client error: {e}")))?;

    let resp: serde_json::Value = client
        .get(&url)
        .basic_auth(token, Option::<&str>::None)
        .send()
        .map_err(|e| DirigentError::Source(format!("SonarQube request failed: {e}")))?
        .json()
        .map_err(|e| DirigentError::Source(format!("SonarQube response parse error: {e}")))?;

    if let Some(errors) = resp.get("errors").and_then(|v| v.as_array()) {
        let msgs: Vec<String> = errors
            .iter()
            .filter_map(|e| e.get("msg").and_then(|m| m.as_str()).map(String::from))
            .collect();
        return Err(DirigentError::Source(format!(
            "SonarQube API error: {}",
            if msgs.is_empty() {
                "unknown error".to_string()
            } else {
                msgs.join("; ")
            }
        )));
    }

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
pub(crate) fn load_env_var(project_root: &Path, key: &str) -> Option<String> {
    let env_path = project_root.join(".env");
    let content = std::fs::read_to_string(env_path).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let prefix = format!("{}=", key);
        if let Some(value) = line.strip_prefix(&prefix) {
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
    None
}
