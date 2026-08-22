//! SonarQube issues, security hotspots and duplication reports.

use std::sync::atomic::AtomicBool;

use super::super::external::{http_client, url_encode, warn_once};
use super::super::types::SourceItem;
use crate::error::DirigentError;

/// Check whether a `DirigentError` is an "Insufficient privileges" response from SonarQube.
fn is_insufficient_privileges(e: &DirigentError) -> bool {
    match e {
        DirigentError::Source(msg) => msg.contains("Insufficient privileges"),
        _ => false,
    }
}
/// Try an optional SonarQube fetch; on failure, warn once with a privilege hint.
fn try_sonar_fetch(
    items: &mut Vec<SourceItem>,
    result: crate::error::Result<Vec<SourceItem>>,
    warned: &AtomicBool,
    category: &str,
    privilege_hint: &str,
) {
    match result {
        Ok(new) => items.extend(new),
        Err(e) => {
            let hint = if is_insufficient_privileges(&e) {
                privilege_hint
            } else {
                ""
            };
            warn_once(warned, &format!("SonarQube {category} skipped: {e}{hint}"));
        }
    }
}

/// Perform a GET request to SonarQube and return the parsed JSON.
fn sonar_get(
    client: &reqwest::blocking::Client,
    url: &str,
    token: &str,
) -> crate::error::Result<serde_json::Value> {
    let resp: serde_json::Value = client
        .get(url)
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
        if !msgs.is_empty() {
            return Err(DirigentError::Source(format!(
                "SonarQube API error: {}",
                msgs.join("; ")
            )));
        }
    }
    Ok(resp)
}
/// Validate that the required SonarQube parameters are non-empty.
fn validate_sonar_params(
    host_url: &str,
    project_key: &str,
    token: &str,
) -> crate::error::Result<()> {
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
    Ok(())
}
/// Fetch issues, security hotspots, and duplications from a SonarQube instance.
/// The caller is expected to resolve the token (e.g. via `resolve_source_token`).
pub(crate) fn fetch_sonarqube_issues(
    host_url: &str,
    project_key: &str,
    token: &str,
    source_label: &str,
) -> crate::error::Result<Vec<SourceItem>> {
    validate_sonar_params(host_url, project_key, token)?;

    let base = host_url.trim_end_matches('/');
    let client = http_client()?;
    let mut items = Vec::new();

    // ── 1. Standard issues (/api/issues/search) ──
    let issues_url = format!(
        "{}/api/issues/search?componentKeys={}&resolved=false&ps=100",
        base,
        url_encode(project_key),
    );
    let resp = sonar_get(&client, &issues_url, token)?;

    let issues = resp
        .get("issues")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    items.extend(
        issues
            .iter()
            .filter_map(|issue| parse_sonar_issue(issue, source_label)),
    );

    // ── 2. Security Hotspots (/api/hotspots/search) ──
    // Non-fatal: some tokens lack hotspot permissions.
    static HOTSPOT_WARNED: AtomicBool = AtomicBool::new(false);
    try_sonar_fetch(
        &mut items,
        fetch_sonar_hotspots(&client, base, project_key, token, source_label),
        &HOTSPOT_WARNED,
        "hotspots",
        "\n  → Your token's user needs 'Browse' and 'Administer \
         Security Hotspots' on this project. Ask a SonarQube admin \
         to grant these under the project's Permissions page, or \
         generate a new token with a user that already has them.",
    );

    // ── 3. Duplications (/api/measures/component) ──
    // Non-fatal: some tokens lack measures permissions.
    static DUP_WARNED: AtomicBool = AtomicBool::new(false);
    try_sonar_fetch(
        &mut items,
        fetch_sonar_duplications(&client, base, project_key, token, source_label),
        &DUP_WARNED,
        "duplications",
        "\n  → Your token's user needs 'Browse' permission on this \
         project. Ask a SonarQube admin to grant it, or generate a \
         new token with a user that already has access.",
    );

    Ok(items)
}
/// Fetch security hotspots from SonarQube.
fn fetch_sonar_hotspots(
    client: &reqwest::blocking::Client,
    base: &str,
    project_key: &str,
    token: &str,
    source_label: &str,
) -> crate::error::Result<Vec<SourceItem>> {
    let url = format!(
        "{}/api/hotspots/search?projectKey={}&ps=100&status=TO_REVIEW",
        base,
        url_encode(project_key),
    );
    let resp = sonar_get(client, &url, token)?;
    let hotspots = resp
        .get("hotspots")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(hotspots
        .iter()
        .filter_map(|hs| parse_sonar_hotspot(hs, source_label))
        .collect())
}
/// Fetch duplication measures from SonarQube.
fn fetch_sonar_duplications(
    client: &reqwest::blocking::Client,
    base: &str,
    project_key: &str,
    token: &str,
    source_label: &str,
) -> crate::error::Result<Vec<SourceItem>> {
    let url = format!(
        "{}/api/measures/component?component={}&metricKeys=duplicated_lines_density,duplicated_blocks,duplicated_lines,duplicated_files",
        base,
        url_encode(project_key),
    );
    let resp = sonar_get(client, &url, token)?;
    let measures = resp
        .pointer("/component/measures")
        .and_then(|v| v.as_array());
    let Some(measures) = measures else {
        return Ok(Vec::new());
    };
    // Check duplicated_lines_density — skip all duplication items if below 3.0%
    // (the SonarQube "Required" gate threshold).
    let density: f64 = measures
        .iter()
        .filter(|m| m.get("metric").and_then(|v| v.as_str()) == Some("duplicated_lines_density"))
        .filter_map(|m| m.get("value").and_then(|v| v.as_str()))
        .filter_map(|v| v.parse().ok())
        .next()
        .unwrap_or(0.0);
    if density < 3.0 {
        return Ok(Vec::new());
    }

    // Skip summary measures (e.g. "Duplicated blocks: 6") — they lack detail
    // for Dirigent to act on.  Only fetch per-file duplication details.
    let mut items: Vec<SourceItem> = Vec::new();

    // Fetch per-file duplication details via component_tree.
    static DUP_FILES_WARNED: AtomicBool = AtomicBool::new(false);
    try_sonar_fetch(
        &mut items,
        fetch_sonar_duplicated_files(client, base, project_key, token, source_label),
        &DUP_FILES_WARNED,
        "duplicated files detail",
        "\n  → Your token's user needs 'Browse' permission on this \
         project. Ask a SonarQube admin to grant it, or generate a \
         new token with a user that already has access.",
    );

    Ok(items)
}
/// Fetch individual files with duplications via `/api/measures/component_tree`.
fn fetch_sonar_duplicated_files(
    client: &reqwest::blocking::Client,
    base: &str,
    project_key: &str,
    token: &str,
    source_label: &str,
) -> crate::error::Result<Vec<SourceItem>> {
    let url = format!(
        "{}/api/measures/component_tree?component={}&metricKeys=duplicated_blocks,duplicated_lines\
         &qualifiers=FIL&metricSort=duplicated_blocks&metricSortFilter=withMeasuresOnly\
         &s=metric&asc=false&ps=100",
        base,
        url_encode(project_key),
    );
    let resp = sonar_get(client, &url, token)?;
    let components = resp.get("components").and_then(|v| v.as_array());
    let Some(components) = components else {
        return Ok(Vec::new());
    };
    let mut items = Vec::new();
    for comp in components {
        let key = comp.get("key").and_then(|v| v.as_str()).unwrap_or("");
        let file_path = key.split(':').next_back().unwrap_or(key);
        let measures = comp.get("measures").and_then(|v| v.as_array());
        let Some(measures) = measures else { continue };
        let mut blocks = 0u64;
        let mut lines = 0u64;
        for m in measures {
            let metric = m.get("metric").and_then(|v| v.as_str()).unwrap_or("");
            let val: u64 = m
                .get("value")
                .and_then(|v| v.as_str())
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            match metric {
                "duplicated_blocks" => blocks = val,
                "duplicated_lines" => lines = val,
                _ => {}
            }
        }
        if blocks == 0 {
            continue;
        }
        items.push(
            SourceItem::new(
                format!("sonar-dup-file-{}-{}", project_key, file_path),
                format!(
                    "[DUPLICATION] {} ({} blocks, {} lines)",
                    file_path, blocks, lines,
                ),
                source_label,
            )
            .with_location(file_path, 0),
        );
    }
    Ok(items)
}
/// Extract the file path from a SonarQube component string.
/// SonarQube components are typically `project-key:src/file.rs`; this strips the
/// project key prefix and returns just the relative path portion.
///
/// Rejects paths that contain `..` segments or are absolute, to prevent path
/// traversal attacks via malicious SonarQube data.
fn sonar_component_to_path(component: &str) -> String {
    if component.is_empty() {
        return String::new();
    }
    let path = component.split(':').next_back().unwrap_or(component);
    // Reject absolute paths and path traversal attempts.
    if std::path::Path::new(path).components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        log::warn!(
            "[sonar] rejecting component path with traversal: {:?}",
            path
        );
        return String::new();
    }
    path.to_string()
}
/// Format a SonarQube finding location suffix like `(file.rs:10, rule: S123)`.
/// Returns an empty string when `component` is empty.
fn sonar_location_suffix(component: &str, line: u64, detail_label: &str, detail: &str) -> String {
    if component.is_empty() {
        String::new()
    } else if line > 0 {
        format!(" ({}:{}, {}: {})", component, line, detail_label, detail)
    } else {
        format!(" ({}, {}: {})", component, detail_label, detail)
    }
}
/// Parse a standard SonarQube issue into a `SourceItem`.
fn parse_sonar_issue(issue: &serde_json::Value, source_label: &str) -> Option<SourceItem> {
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

    let loc = sonar_location_suffix(component, line, "rule", rule);
    let text = format!("[{}] {}{}", severity, message, loc);
    let file_path = sonar_component_to_path(component);
    Some(SourceItem::new(key, text, source_label).with_location(&file_path, line as usize))
}
/// Parse a SonarQube Security Hotspot into a `SourceItem`.
fn parse_sonar_hotspot(hs: &serde_json::Value, source_label: &str) -> Option<SourceItem> {
    let key = hs.get("key")?.as_str()?;
    let message = hs.get("message")?.as_str()?;
    let vulnerability = hs
        .get("vulnerabilityProbability")
        .and_then(|v| v.as_str())
        .unwrap_or("UNKNOWN");
    let component = hs.get("component").and_then(|c| c.as_str()).unwrap_or("");
    let line = hs.get("line").and_then(|l| l.as_u64()).unwrap_or(0);
    let category = hs
        .get("securityCategory")
        .and_then(|c| c.as_str())
        .unwrap_or("");

    let loc = sonar_location_suffix(component, line, "category", category);
    let text = format!("[HOTSPOT/{}] {}{}", vulnerability, message, loc);
    let file_path = sonar_component_to_path(component);
    Some(SourceItem::new(key, text, source_label).with_location(&file_path, line as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape captured from a real `GET /api/issues/search` response.
    fn issue_fixture() -> serde_json::Value {
        serde_json::json!({
            "key": "AY1234abcd",
            "rule": "rust:S1135",
            "severity": "MAJOR",
            "component": "my-project:src/app/mod.rs",
            "project": "my-project",
            "line": 214,
            "message": "Complete the task associated to this TODO comment.",
            "status": "OPEN",
            "type": "CODE_SMELL"
        })
    }

    #[test]
    fn parses_issue_with_location_and_severity() {
        let item = parse_sonar_issue(&issue_fixture(), "Sonar").expect("issue parses");
        assert_eq!(item.external_id, "AY1234abcd");
        assert_eq!(item.file_path, "src/app/mod.rs");
        assert_eq!(item.line_number, 214);
        assert!(item.text.starts_with("[MAJOR] "), "got: {}", item.text);
        assert!(item.text.contains("rule: rust:S1135"), "got: {}", item.text);
    }

    /// A project-level issue has no line; the suffix must drop the `:0`.
    #[test]
    fn parses_issue_without_line() {
        let mut issue = issue_fixture();
        issue.as_object_mut().expect("object").remove("line");
        let item = parse_sonar_issue(&issue, "Sonar").expect("issue parses");
        assert_eq!(item.line_number, 0);
        assert!(
            !item.text.contains(":0,"),
            "line-less issue must not render a zero line: {}",
            item.text
        );
    }

    /// Missing required fields yield `None` rather than a half-built item.
    #[test]
    fn issue_without_key_or_message_is_skipped() {
        let mut no_key = issue_fixture();
        no_key.as_object_mut().expect("object").remove("key");
        assert!(parse_sonar_issue(&no_key, "Sonar").is_none());

        let mut no_message = issue_fixture();
        no_message
            .as_object_mut()
            .expect("object")
            .remove("message");
        assert!(parse_sonar_issue(&no_message, "Sonar").is_none());
    }

    /// Missing optional fields fall back rather than dropping the finding.
    #[test]
    fn issue_defaults_severity_when_absent() {
        let mut issue = issue_fixture();
        issue.as_object_mut().expect("object").remove("severity");
        let item = parse_sonar_issue(&issue, "Sonar").expect("issue parses");
        assert!(item.text.starts_with("[INFO] "), "got: {}", item.text);
    }

    #[test]
    fn parses_security_hotspot() {
        let hotspot = serde_json::json!({
            "key": "AZ9876wxyz",
            "component": "my-project:src/lsp/client.rs",
            "project": "my-project",
            "securityCategory": "dos",
            "vulnerabilityProbability": "HIGH",
            "line": 536,
            "message": "Make sure not using resource integrity feature is safe here.",
            "status": "TO_REVIEW"
        });
        let item = parse_sonar_hotspot(&hotspot, "Sonar").expect("hotspot parses");
        assert_eq!(item.external_id, "AZ9876wxyz");
        assert_eq!(item.file_path, "src/lsp/client.rs");
        assert_eq!(item.line_number, 536);
        assert!(
            item.text.starts_with("[HOTSPOT/HIGH] "),
            "got: {}",
            item.text
        );
        assert!(item.text.contains("category: dos"), "got: {}", item.text);
    }

    /// The component key is `<projectKey>:<path>`; only the path is kept, and
    /// anything that escapes the repository is rejected outright.
    #[test]
    fn component_to_path_strips_project_and_rejects_traversal() {
        assert_eq!(
            sonar_component_to_path("proj:src/main.rs"),
            "src/main.rs".to_string()
        );
        assert_eq!(sonar_component_to_path(""), String::new());
        assert_eq!(
            sonar_component_to_path("proj:../../etc/passwd"),
            String::new()
        );
        assert_eq!(sonar_component_to_path("proj:/etc/passwd"), String::new());
    }

    #[test]
    fn location_suffix_is_empty_without_a_component() {
        assert_eq!(sonar_location_suffix("", 12, "rule", "S1"), String::new());
        assert_eq!(
            sonar_location_suffix("c.rs", 0, "rule", "S1"),
            " (c.rs, rule: S1)".to_string()
        );
    }

    #[test]
    fn insufficient_privileges_is_recognised() {
        assert!(is_insufficient_privileges(&DirigentError::Source(
            "SonarQube API error: Insufficient privileges".to_string()
        )));
        assert!(!is_insufficient_privileges(&DirigentError::Source(
            "SonarQube request failed: connection refused".to_string()
        )));
    }
}
