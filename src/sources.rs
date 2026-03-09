use std::path::Path;
use std::process::Command;

/// An item fetched from an external source, to be converted to a Cue.
#[derive(Debug, Clone)]
pub struct SourceItem {
    pub external_id: String,
    pub text: String,
    pub source_label: String,
}

/// Fetch items from a GitHub Issues source using the `gh` CLI.
pub fn fetch_github_issues(
    project_root: &Path,
    label_filter: Option<&str>,
    state: Option<&str>,
    source_label: &str,
) -> Vec<SourceItem> {
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

    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("Failed to run gh: {}", e);
            return Vec::new();
        }
    };

    if !output.status.success() {
        eprintln!(
            "gh issue list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return Vec::new();
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let issues: Vec<serde_json::Value> = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Failed to parse gh output: {}", e);
            return Vec::new();
        }
    };

    issues
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
        .collect()
}

/// Fetch items from a custom command source.
/// The command should output JSON: either an array of objects or one object per line.
/// Each object should have "id" and "text" fields.
pub fn fetch_custom_command(
    project_root: &Path,
    command: &str,
    source_label: &str,
) -> Vec<SourceItem> {
    let output = match Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(project_root)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("Failed to run custom source command: {}", e);
            return Vec::new();
        }
    };

    if !output.status.success() {
        eprintln!(
            "Custom source command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return Vec::new();
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    parse_source_json(&json_str, source_label)
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
