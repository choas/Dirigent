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
fn output_with_timeout(
    mut child: std::process::Child,
    timeout: std::time::Duration,
) -> std::io::Result<std::process::Output> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait()? {
            Some(_) => return child.wait_with_output(),
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
    }
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
}
