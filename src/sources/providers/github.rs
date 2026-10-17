//! GitHub Issues, fetched through the `gh` CLI.

use std::path::Path;
use std::process::Command;

use super::super::custom::output_piped_command;
use super::super::external::title_with_optional_body;
use super::super::types::SourceItem;
use crate::error::DirigentError;

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
    let output = output_piped_command(cmd)?;

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
        .filter_map(|issue| github_issue_to_item(issue, source_label))
        .collect())
}

/// Map one `gh issue list --json number,title,body,url` entry to a `SourceItem`.
fn github_issue_to_item(issue: &serde_json::Value, source_label: &str) -> Option<SourceItem> {
    let number = issue.get("number")?.as_i64()?;
    let title = issue.get("title")?.as_str()?;
    let url = issue.get("url")?.as_str()?;
    let body = issue.get("body").and_then(|b| b.as_str()).unwrap_or("");

    let summary = format!("[#{}] {}", number, title);
    let text = title_with_optional_body(&summary, body);

    Some(SourceItem::new(url, text, source_label))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue_fixture() -> serde_json::Value {
        serde_json::json!({
            "number": 412,
            "title": "Crash when opening an empty repo",
            "body": "Steps:\n1. dirigent /tmp/empty\n2. boom",
            "url": "https://github.com/acme/dirigent/issues/412"
        })
    }

    #[test]
    fn maps_an_issue_with_its_number_prefixed() {
        let item = github_issue_to_item(&issue_fixture(), "GitHub").expect("issue parses");
        assert_eq!(
            item.external_id,
            "https://github.com/acme/dirigent/issues/412"
        );
        assert!(
            item.text
                .starts_with("[#412] Crash when opening an empty repo\n\n"),
            "got: {}",
            item.text
        );
    }

    #[test]
    fn issue_without_a_body_is_just_its_title() {
        let mut issue = issue_fixture();
        issue["body"] = serde_json::json!("");
        let item = github_issue_to_item(&issue, "GitHub").expect("issue parses");
        assert_eq!(item.text, "[#412] Crash when opening an empty repo");
    }

    #[test]
    fn issue_missing_a_required_field_is_skipped() {
        for missing in ["number", "title", "url"] {
            let mut issue = issue_fixture();
            issue.as_object_mut().expect("object").remove(missing);
            assert!(
                github_issue_to_item(&issue, "GitHub").is_none(),
                "issue without {missing} must be skipped"
            );
        }
    }
}
