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

    let output = cmd.output()?;

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
            let body = issue
                .get("body")
                .and_then(|b| b.as_str())
                .unwrap_or("");

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

/// Maximum length for a custom source command string.
const MAX_COMMAND_LENGTH: usize = 4096;

/// Validate a custom command string for safety.
/// Rejects null bytes, control characters (except common whitespace),
/// and excessively long commands.
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
    if let Some(pos) = command.chars().position(|c| {
        c.is_control() && c != '\t' && c != '\n' && c != '\r'
    }) {
        return Err(format!(
            "command contains control character at position {}",
            pos
        ));
    }
    Ok(())
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

    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(project_root)
        .output()?;

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
