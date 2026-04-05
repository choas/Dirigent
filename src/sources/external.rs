use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::DirigentError;

/// Print a warning to stderr at most once per flag.
fn warn_once(flag: &AtomicBool, msg: &str) {
    if !flag.swap(true, Ordering::Relaxed) {
        eprintln!("{msg}");
    }
}

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

            Some(SourceItem::new(url, text, source_label))
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

    let client = http_client()?;

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
            Some(SourceItem::new(
                format!("{}/{}", channel, ts),
                format!("[{}] {}", user, text),
                source_label,
            ))
        })
        .collect())
}

/// Check that an HTTP response indicates success; return a descriptive error otherwise.
fn check_response(
    resp: reqwest::blocking::Response,
    api_name: &str,
) -> crate::error::Result<reqwest::blocking::Response> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status();
    let body = resp.text().unwrap_or_default();
    Err(DirigentError::Source(format!(
        "{api_name} error ({status}): {body}"
    )))
}

/// Build an HTTP client with the standard subprocess timeout.
fn http_client() -> crate::error::Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS))
        .build()
        .map_err(|e| DirigentError::Source(format!("HTTP client error: {e}")))
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
        base, project_key,
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
        base, project_key,
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
        base, project_key,
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
        base, project_key,
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
fn sonar_component_to_path(component: &str) -> String {
    if component.is_empty() {
        return String::new();
    }
    component
        .split(':')
        .next_back()
        .unwrap_or(component)
        .to_string()
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

/// Fetch cards from a Trello board using the Trello REST API.
/// Requires an API key and a token (generated at trello.com/power-ups/admin).
pub(crate) fn fetch_trello_cards(
    api_key: &str,
    token: &str,
    board_id: &str,
    list_filter: Option<&str>,
    source_label: &str,
) -> crate::error::Result<Vec<SourceItem>> {
    if api_key.is_empty() {
        return Err(DirigentError::Source("Trello API key is empty".to_string()));
    }
    if token.is_empty() {
        return Err(DirigentError::Source("Trello token is empty".to_string()));
    }
    if board_id.is_empty() {
        return Err(DirigentError::Source(
            "Trello board ID is empty".to_string(),
        ));
    }

    let client = http_client()?;

    // Fetch cards with their list info so we can filter by list name.
    let url = format!("https://api.trello.com/1/boards/{}/cards", board_id,);

    let resp = client
        .get(&url)
        .query(&[
            ("key", api_key),
            ("token", token),
            ("fields", "name,desc,shortUrl,idList"),
            ("limit", "100"),
        ])
        .send()
        .map_err(|e| {
            DirigentError::Source(format!("Trello request failed: {}", e.without_url()))
        })?;
    let resp = check_response(resp, "Trello API")?;

    let parsed: serde_json::Value = resp
        .json()
        .map_err(|e| DirigentError::Source(format!("Trello response parse error: {e}")))?;

    // Trello may return a plain string error message instead of JSON.
    if let Some(err_str) = parsed.as_str() {
        return Err(DirigentError::Source(format!(
            "Trello API error: {err_str}"
        )));
    }

    // Trello may return an error object instead of the expected array.
    if let Some(err_msg) = parsed.get("error").and_then(|v| v.as_str()) {
        let detail = parsed.get("message").and_then(|v| v.as_str()).unwrap_or("");
        return Err(DirigentError::Source(format!(
            "Trello API error: {err_msg}: {detail}"
        )));
    }

    let cards: Vec<serde_json::Value> = serde_json::from_value(parsed)
        .map_err(|e| DirigentError::Source(format!("Trello response parse error: {e}")))?;

    let allowed_list_ids = match list_filter {
        Some(filter) => Some(resolve_trello_list_ids(
            &client, board_id, api_key, token, filter,
        )?),
        None => None,
    };

    Ok(cards
        .iter()
        .filter_map(|card| trello_card_to_item(card, allowed_list_ids.as_deref(), source_label))
        .collect())
}

/// Resolve Trello list names to IDs for filtering cards by list.
fn resolve_trello_list_ids(
    client: &reqwest::blocking::Client,
    board_id: &str,
    api_key: &str,
    token: &str,
    filter: &str,
) -> crate::error::Result<Vec<String>> {
    let lists_url = format!("https://api.trello.com/1/boards/{}/lists", board_id,);
    let lists_resp = client
        .get(&lists_url)
        .query(&[("key", api_key), ("token", token), ("fields", "name")])
        .send()
        .map_err(|e| {
            DirigentError::Source(format!("Trello lists request failed: {}", e.without_url()))
        })?;
    let lists_resp = check_response(lists_resp, "Trello lists API")?;

    let lists: Vec<serde_json::Value> = lists_resp
        .json()
        .map_err(|e| DirigentError::Source(format!("Trello lists parse error: {e}")))?;

    let filter_lower = filter.to_lowercase();
    Ok(lists
        .iter()
        .filter_map(|l| {
            let name = l.get("name")?.as_str()?;
            if name.to_lowercase().contains(&filter_lower) {
                Some(l.get("id")?.as_str()?.to_string())
            } else {
                None
            }
        })
        .collect())
}

/// Convert a Trello card JSON value to a `SourceItem`, applying an optional list filter.
fn trello_card_to_item(
    card: &serde_json::Value,
    allowed_list_ids: Option<&[String]>,
    source_label: &str,
) -> Option<SourceItem> {
    let name = card.get("name")?.as_str()?;
    let url = card.get("shortUrl")?.as_str()?;
    let desc = card.get("desc").and_then(|d| d.as_str()).unwrap_or("");

    if let Some(ids) = allowed_list_ids {
        let id_list = card.get("idList")?.as_str()?;
        if !ids.iter().any(|id| id == id_list) {
            return None;
        }
    }

    let text = if desc.is_empty() {
        name.to_string()
    } else {
        format!("{}\n\n{}", name, desc)
    };

    Some(SourceItem::new(url, text, source_label))
}

/// Fetch incomplete tasks from an Asana project using the Asana REST API.
/// Requires a personal access token.
pub(crate) fn fetch_asana_tasks(
    token: &str,
    project_gid: &str,
    source_label: &str,
) -> crate::error::Result<Vec<SourceItem>> {
    if token.is_empty() {
        return Err(DirigentError::Source("Asana token is empty".to_string()));
    }
    if project_gid.is_empty() {
        return Err(DirigentError::Source(
            "Asana project GID is empty".to_string(),
        ));
    }

    let client = http_client()?;

    let url = format!(
        "https://app.asana.com/api/1.0/projects/{}/tasks?opt_fields=name,notes,permalink_url,completed&limit=100",
        project_gid,
    );

    let resp_raw = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .map_err(|e| DirigentError::Source(format!("Asana request failed: {e}")))?;
    let resp_raw = check_response(resp_raw, "Asana API")?;

    let resp: serde_json::Value = resp_raw
        .json()
        .map_err(|e| DirigentError::Source(format!("Asana response parse error: {e}")))?;

    if let Some(errors) = resp.get("errors").and_then(|v| v.as_array()) {
        let msgs: Vec<String> = errors
            .iter()
            .filter_map(|e| e.get("message").and_then(|m| m.as_str()).map(String::from))
            .collect();
        return Err(DirigentError::Source(format!(
            "Asana API error: {}",
            if msgs.is_empty() {
                "unknown error".to_string()
            } else {
                msgs.join("; ")
            }
        )));
    }

    let tasks = resp
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(tasks
        .iter()
        .filter_map(|task| {
            // Skip completed tasks.
            if task.get("completed").and_then(|c| c.as_bool()) == Some(true) {
                return None;
            }

            let gid = task.get("gid")?.as_str()?;
            let name = task.get("name")?.as_str()?;
            if name.trim().is_empty() {
                return None;
            }
            let notes = task.get("notes").and_then(|n| n.as_str()).unwrap_or("");
            let permalink = task
                .get("permalink_url")
                .and_then(|u| u.as_str())
                .unwrap_or("");

            let text = if notes.is_empty() {
                name.to_string()
            } else {
                format!("{}\n\n{}", name, notes)
            };

            let external_id = if permalink.is_empty() {
                format!("asana:{}", gid)
            } else {
                permalink.to_string()
            };

            Some(SourceItem::new(external_id, text, source_label))
        })
        .collect())
}

/// Fetch all databases and pages visible to the Notion integration token.
///
/// Uses the Notion Search API (`POST /v1/search`) to list every object the
/// integration has been shared with.  Returns a vec of [`NotionObject`] with
/// both databases and pages, sorted databases-first.
pub(crate) fn fetch_notion_objects(
    token: &str,
) -> crate::error::Result<Vec<super::types::NotionObject>> {
    if token.is_empty() {
        return Err(DirigentError::Source(
            "Notion token is empty (set in source config or NOTION_TOKEN in .env)".to_string(),
        ));
    }

    let client = http_client()?;
    let raw = notion_paginated_post(
        &client,
        "https://api.notion.com/v1/search",
        token,
        &serde_json::json!({ "page_size": 100 }),
    )?;
    let mut all: Vec<super::types::NotionObject> =
        raw.iter().filter_map(parse_notion_search_result).collect();
    sort_notion_objects(&mut all);
    Ok(all)
}

fn sort_notion_objects(objects: &mut [super::types::NotionObject]) {
    objects.sort_by(|a, b| {
        let type_order = |t: &str| -> u8 {
            if t == "database" {
                0
            } else {
                1
            }
        };
        type_order(&a.object_type)
            .cmp(&type_order(&b.object_type))
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
}

/// POST to a paginated Notion endpoint, collecting all `results` arrays across pages.
fn notion_paginated_post(
    client: &reqwest::blocking::Client,
    url: &str,
    token: &str,
    base_body: &serde_json::Value,
) -> crate::error::Result<Vec<serde_json::Value>> {
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut body = base_body.clone();
        if let Some(ref c) = cursor {
            body["start_cursor"] = serde_json::json!(c);
        }
        let json = notion_post(client, url, token, &body)?;
        if let Some(results) = json.get("results").and_then(|v| v.as_array()) {
            all.extend(results.iter().cloned());
        }
        if !notion_has_more(&json, &mut cursor) {
            break;
        }
    }
    Ok(all)
}

fn notion_post(
    client: &reqwest::blocking::Client,
    url: &str,
    token: &str,
    body: &serde_json::Value,
) -> crate::error::Result<serde_json::Value> {
    let resp = client
        .post(url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Notion-Version", "2022-06-28")
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .map_err(|e| DirigentError::Source(format!("Notion request failed: {e}")))?;
    let resp = check_response(resp, "Notion API")?;

    resp.json()
        .map_err(|e| DirigentError::Source(format!("Notion response parse error: {e}")))
}

fn notion_get(
    client: &reqwest::blocking::Client,
    url: &str,
    token: &str,
) -> crate::error::Result<serde_json::Value> {
    let resp = client
        .get(url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Notion-Version", "2022-06-28")
        .send()
        .map_err(|e| DirigentError::Source(format!("Notion request failed: {e}")))?;
    let resp = check_response(resp, "Notion API")?;

    resp.json()
        .map_err(|e| DirigentError::Source(format!("Notion response parse error: {e}")))
}

fn notion_has_more(json: &serde_json::Value, cursor: &mut Option<String>) -> bool {
    let has_more = json
        .get("has_more")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !has_more {
        return false;
    }
    *cursor = json
        .get("next_cursor")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    cursor.is_some()
}

fn parse_notion_search_result(obj: &serde_json::Value) -> Option<super::types::NotionObject> {
    use super::types::NotionObject;

    let object_type = obj.get("object")?.as_str()?.to_string();
    let id = obj.get("id")?.as_str()?.to_string();
    if id.is_empty() {
        return None;
    }

    let title = match object_type.as_str() {
        "database" => extract_database_title(obj),
        "page" => extract_notion_page_title(obj),
        _ => return None,
    };

    let title = if title.is_empty() {
        let short_id = if id.len() > 8 { &id[..8] } else { &id };
        format!("Untitled ({}…)", short_id)
    } else {
        title
    };

    Some(NotionObject {
        id,
        title,
        object_type,
    })
}

fn extract_database_title(obj: &serde_json::Value) -> String {
    obj.get("title")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.get("plain_text").and_then(|p| p.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// Fetch tasks from a Notion database using the Notion API.
///
/// For **Todo List** databases: fetches pages where the checkbox property
/// (named by `done_property`) is *not* checked.
///
/// For **Kanban Board** databases: fetches pages whose Status property
/// matches `inbox_status` (e.g. "Not started").
///
/// The Notion API version used is `2022-06-28`.
pub(crate) fn fetch_notion_tasks(
    token: &str,
    database_id: &str,
    page_type: &crate::settings::NotionPageType,
    inbox_status: Option<&str>,
    done_property: &str,
    status_property: &str,
    source_label: &str,
) -> crate::error::Result<Vec<SourceItem>> {
    if token.is_empty() {
        return Err(DirigentError::Source(
            "Notion token is empty (set in source config or NOTION_TOKEN in .env)".to_string(),
        ));
    }
    if database_id.is_empty() {
        return Err(DirigentError::Source(
            "Notion database ID is empty".to_string(),
        ));
    }

    let client = http_client()?;
    let actual_id = extract_notion_page_id(database_id);
    let resolved = resolve_notion_id(&client, token, &actual_id)?;

    let resolved_id = match resolved {
        NotionIdKind::Database(id) => id,
        NotionIdKind::Page(id) => {
            return fetch_notion_single_page(&client, token, &id, source_label);
        }
    };

    let url = format!("https://api.notion.com/v1/databases/{}/query", resolved_id);
    let filter = build_notion_query_filter(page_type, inbox_status, done_property, status_property);
    let all_results = notion_paginated_post(&client, &url, token, &filter)?;

    Ok(all_results
        .iter()
        .filter_map(|page| notion_page_to_item(page, source_label))
        .collect())
}

fn build_notion_query_filter(
    page_type: &crate::settings::NotionPageType,
    inbox_status: Option<&str>,
    done_property: &str,
    status_property: &str,
) -> serde_json::Value {
    use crate::settings::NotionPageType;

    match page_type {
        NotionPageType::TodoList => {
            let prop = if done_property.is_empty() {
                "Done"
            } else {
                done_property
            };
            serde_json::json!({
                "filter": { "property": prop, "checkbox": { "equals": false } },
                "page_size": 100
            })
        }
        NotionPageType::KanbanBoard => {
            let status_prop = if status_property.is_empty() {
                "Status"
            } else {
                status_property
            };
            build_kanban_filter(status_prop, inbox_status, done_property)
        }
    }
}

fn build_kanban_filter(
    status_prop: &str,
    inbox_status: Option<&str>,
    done_property: &str,
) -> serde_json::Value {
    if let Some(status) = inbox_status.filter(|s| !s.is_empty()) {
        serde_json::json!({
            "filter": { "property": status_prop, "status": { "equals": status } },
            "page_size": 100
        })
    } else {
        let done_val = if done_property.is_empty() {
            "Done"
        } else {
            done_property
        };
        serde_json::json!({
            "filter": { "property": status_prop, "status": { "does_not_equal": done_val } },
            "page_size": 100
        })
    }
}

/// Extract a `SourceItem` from a Notion page object.
fn notion_page_to_item(page: &serde_json::Value, source_label: &str) -> Option<SourceItem> {
    let id = page.get("id")?.as_str()?;

    // Extract title from the first "title" type property.
    let properties = page.get("properties")?.as_object()?;
    let title = properties.values().find_map(|prop| {
        if prop.get("type")?.as_str()? != "title" {
            return None;
        }
        let title_arr = prop.get("title")?.as_array()?;
        let parts: Vec<&str> = title_arr
            .iter()
            .filter_map(|t| t.get("plain_text").and_then(|p| p.as_str()))
            .collect();
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(""))
        }
    })?;

    if title.trim().is_empty() {
        return None;
    }

    Some(SourceItem::new(id, title, source_label))
}

/// Mark a Notion page as done via the Notion API.
///
/// For **Todo List** pages: sets the checkbox property to `true`.
/// For **Kanban Board** pages: sets the Status property to the given value.
pub(crate) fn mark_notion_done(
    token: &str,
    page_id: &str,
    page_type: &crate::settings::NotionPageType,
    done_value: &str,
    status_property: &str,
) -> crate::error::Result<()> {
    use crate::settings::NotionPageType;

    if token.is_empty() {
        return Err(DirigentError::Source("Notion token is empty".to_string()));
    }
    if page_id.is_empty() {
        return Err(DirigentError::Source("Notion page ID is empty".to_string()));
    }

    let client = http_client()?;

    // The page_id from source_ref may be a URL like "https://www.notion.so/...{id}".
    // Extract the actual UUID if needed.
    let actual_id = extract_notion_page_id(page_id);

    let url = format!("https://api.notion.com/v1/pages/{}", actual_id);

    let done_val = if done_value.is_empty() {
        "Done"
    } else {
        done_value
    };

    let body = match page_type {
        NotionPageType::TodoList => {
            serde_json::json!({
                "properties": {
                    (done_val): { "checkbox": true }
                }
            })
        }
        NotionPageType::KanbanBoard => {
            let status_prop = if status_property.is_empty() {
                "Status"
            } else {
                status_property
            };
            serde_json::json!({
                "properties": {
                    (status_prop): {
                        "status": { "name": done_val }
                    }
                }
            })
        }
    };

    let resp = client
        .patch(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Notion-Version", "2022-06-28")
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .map_err(|e| DirigentError::Source(format!("Notion request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        // Parse the Notion error JSON for a readable message.
        let message = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("message")?.as_str().map(|s| s.to_string()));
        return Err(DirigentError::Source(match message {
            Some(msg) if status == reqwest::StatusCode::NOT_FOUND => {
                format!(
                    "Notion page not found — share the page (or its parent database) \
                     with your Notion integration. ({msg})"
                )
            }
            Some(msg) => format!("Notion API error ({status}): {msg}"),
            None => format!("Notion API error ({status}): {body}"),
        }));
    }

    Ok(())
}

/// The result of resolving a Notion ID: either a database or a standalone page.
enum NotionIdKind {
    Database(String),
    Page(String),
}

/// Given an ID that might be a database or a page, resolve what it is.
/// If the ID refers to a database directly, return `Database`.  If it
/// refers to a page that contains a child database, return that child
/// `Database`.  Otherwise return `Page` so the caller can fetch the
/// page directly.
fn resolve_notion_id(
    client: &reqwest::blocking::Client,
    token: &str,
    id: &str,
) -> crate::error::Result<NotionIdKind> {
    // Quick check: try to retrieve the database metadata.
    let db_resp = client
        .get(format!("https://api.notion.com/v1/databases/{}", id))
        .header("Authorization", format!("Bearer {}", token))
        .header("Notion-Version", "2022-06-28")
        .send()
        .map_err(|e| DirigentError::Source(format!("Notion request failed: {e}")))?;

    let db_status = db_resp.status();
    if db_status.is_success() {
        // It is a database — use it directly.
        return Ok(NotionIdKind::Database(id.to_string()));
    }

    // Parse the error body to check if the object simply isn't shared with the
    // integration (Notion returns "object_not_found" for both "does not exist"
    // and "not shared with integration").
    let db_body = db_resp.text().unwrap_or_default();

    // Try as a page: look for child databases inside it.
    let children_url = format!(
        "https://api.notion.com/v1/blocks/{}/children?page_size=100",
        id
    );
    let children_resp = client
        .get(&children_url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Notion-Version", "2022-06-28")
        .send()
        .map_err(|e| DirigentError::Source(format!("Notion request failed: {e}")))?;

    if !children_resp.status().is_success() {
        // Both the database and the page/block lookup failed.  The most common
        // cause is that the page/database hasn't been shared with the Notion
        // integration.  Surface that directly.
        return Err(DirigentError::Source(format!(
            "Could not access this Notion ID ({}). \
             Open the page in Notion, click ··· → Connections, and add your integration. \
             Notion API response: {}",
            db_status, db_body
        )));
    }

    let json: serde_json::Value = children_resp
        .json()
        .map_err(|e| DirigentError::Source(format!("Notion response parse error: {e}")))?;

    if let Some(child_db_id) = find_child_database(&json) {
        return Ok(NotionIdKind::Database(child_db_id));
    }

    // No child database found — treat as a standalone page.
    Ok(NotionIdKind::Page(id.to_string()))
}

/// Search the children blocks JSON for a `child_database` entry and return its ID.
fn find_child_database(json: &serde_json::Value) -> Option<String> {
    let results = json.get("results")?.as_array()?;
    results.iter().find_map(|block| {
        let block_type = block.get("type")?.as_str()?;
        if block_type != "child_database" {
            return None;
        }
        Some(block.get("id")?.as_str()?.to_string())
    })
}

/// Fetch a single Notion page by ID, including its block content.
/// The page body is split at `### ` (h3) headings so each section becomes its
/// own `SourceItem` / cue.  Content before the first h3 (plus the page title)
/// becomes one item; each h3 section becomes another.
fn fetch_notion_single_page(
    client: &reqwest::blocking::Client,
    token: &str,
    page_id: &str,
    source_label: &str,
) -> crate::error::Result<Vec<SourceItem>> {
    let url = format!("https://api.notion.com/v1/pages/{}", page_id);
    let page = notion_get(client, &url, token)?;

    let id = match page.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => return Ok(vec![]),
    };

    let title = extract_notion_page_title(&page);
    if title.trim().is_empty() {
        return Ok(vec![]);
    }

    let lines = fetch_notion_block_lines(client, token, page_id).unwrap_or_default();

    // Separate unchecked to-do lines from other content.
    // Checked to-dos are already filtered out by format_todo_block.
    let mut todo_texts = Vec::new();
    let mut other_lines = Vec::new();
    for line in &lines {
        if let Some(todo_text) = line.strip_prefix("- [ ] ") {
            todo_texts.push(todo_text.to_string());
        } else {
            other_lines.push(line.clone());
        }
    }

    if !todo_texts.is_empty() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut items: Vec<SourceItem> = todo_texts
            .iter()
            .enumerate()
            .map(|(idx, text)| {
                // Include the index so that duplicate todo texts on the same
                // page produce distinct IDs instead of colliding.
                let mut hasher = DefaultHasher::new();
                idx.hash(&mut hasher);
                text.hash(&mut hasher);
                let hash = hasher.finish();
                SourceItem::new(
                    format!("{}-todo-{:x}", id, hash),
                    format!("{}: {}", title, text),
                    source_label,
                )
            })
            .collect();
        // Also include non-to-do sections (headings, paragraphs, etc.)
        let sections = split_h3_sections(&other_lines);
        items.extend(sections_to_items(sections, &id, &title, source_label));
        return Ok(items);
    }

    let sections = split_h3_sections(&lines);
    let items = sections_to_items(sections, &id, &title, source_label);
    if !items.is_empty() {
        return Ok(items);
    }

    Ok(vec![build_fallback_item(id, title, &lines, source_label)])
}

fn build_fallback_item(
    id: String,
    title: String,
    lines: &[String],
    source_label: &str,
) -> SourceItem {
    let body = lines.join("\n").trim().to_string();
    let text = if body.is_empty() {
        title
    } else {
        format!("{}\n\n{}", title, body)
    };
    SourceItem::new(id, text, source_label)
}

fn split_h3_sections(lines: &[String]) -> Vec<(Option<String>, Vec<String>)> {
    let mut sections: Vec<(Option<String>, Vec<String>)> = vec![(None, Vec::new())];
    for line in lines {
        if line.starts_with("### ") {
            let heading = line.trim_start_matches("### ").to_string();
            sections.push((Some(heading), Vec::new()));
        } else if let Some(last) = sections.last_mut() {
            last.1.push(line.clone());
        }
    }
    sections
}

fn sections_to_items(
    sections: Vec<(Option<String>, Vec<String>)>,
    id: &str,
    title: &str,
    source_label: &str,
) -> Vec<SourceItem> {
    let mut items = Vec::new();
    for (idx, (heading, body_lines)) in sections.into_iter().enumerate() {
        let body = body_lines.join("\n").trim().to_string();
        let text = section_text(heading.as_deref(), &body, title);
        let Some(text) = text else { continue };
        items.push(SourceItem::new(
            format!("{}-{}", id, idx),
            text,
            source_label,
        ));
    }
    items
}

fn section_text(heading: Option<&str>, body: &str, title: &str) -> Option<String> {
    match heading {
        None => {
            if body.is_empty() {
                None
            } else {
                Some(format!("{}\n\n{}", title, body))
            }
        }
        Some(h) => {
            if body.is_empty() {
                Some(format!("### {}", h))
            } else {
                Some(format!("### {}\n{}", h, body))
            }
        }
    }
}

/// Build a Notion blocks-children URL with optional pagination cursor.
fn build_blocks_url(block_id: &str, cursor: Option<&str>) -> String {
    let mut url = format!(
        "https://api.notion.com/v1/blocks/{}/children?page_size=100",
        block_id
    );
    if let Some(c) = cursor {
        url.push_str(&format!("&start_cursor={}", c));
    }
    url
}

/// Fetch all block children of a Notion page/block and convert them to
/// markdown-style plain-text lines (one entry per block).
fn fetch_notion_block_lines(
    client: &reqwest::blocking::Client,
    token: &str,
    block_id: &str,
) -> crate::error::Result<Vec<String>> {
    let mut lines: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let url = build_blocks_url(block_id, cursor.as_deref());
        let body = match notion_get(client, &url, token) {
            Ok(json) => json,
            Err(_) => break,
        };

        if let Some(results) = body.get("results").and_then(|r| r.as_array()) {
            lines.extend(results.iter().filter_map(notion_block_to_text));
        }

        if !notion_has_more(&body, &mut cursor) {
            break;
        }
    }

    Ok(lines)
}

/// Convert a single Notion block to a markdown-style text line.
fn notion_block_to_text(block: &serde_json::Value) -> Option<String> {
    let block_type = block.get("type")?.as_str()?;
    match block_type {
        "heading_1" => block_rich_text(block, "heading_1").map(|t| format!("# {}", t)),
        "heading_2" => block_rich_text(block, "heading_2").map(|t| format!("## {}", t)),
        "heading_3" => block_rich_text(block, "heading_3").map(|t| format!("### {}", t)),
        "paragraph" => block_rich_text(block, "paragraph"),
        "bulleted_list_item" => {
            block_rich_text(block, "bulleted_list_item").map(|t| format!("- {}", t))
        }
        "numbered_list_item" => {
            block_rich_text(block, "numbered_list_item").map(|t| format!("1. {}", t))
        }
        "to_do" => format_todo_block(block),
        "quote" => block_rich_text(block, "quote").map(|t| format!("> {}", t)),
        "code" => format_code_block(block),
        "callout" => block_rich_text(block, "callout").map(|t| format!("> {}", t)),
        "toggle" => block_rich_text(block, "toggle"),
        "divider" => Some("---".to_string()),
        _ => None,
    }
}

/// Extract the rich_text plain content from a named sub-object of a block.
fn block_rich_text(block: &serde_json::Value, key: &str) -> Option<String> {
    Some(rich_text_plain(block.get(key)?))
}

fn format_todo_block(block: &serde_json::Value) -> Option<String> {
    let obj = block.get("to_do")?;
    let checked = obj
        .get("checked")
        .and_then(|c| c.as_bool())
        .unwrap_or(false);
    if checked {
        return None; // Skip completed to-dos
    }
    let text = rich_text_plain(obj);
    Some(format!("- [ ] {}", text))
}

fn format_code_block(block: &serde_json::Value) -> Option<String> {
    let obj = block.get("code")?;
    let text = rich_text_plain(obj);
    let lang = obj.get("language").and_then(|l| l.as_str()).unwrap_or("");
    Some(format!("```{}\n{}\n```", lang, text))
}

/// Extract concatenated plain_text from a Notion block's `rich_text` array.
fn rich_text_plain(obj: &serde_json::Value) -> String {
    let arr = match obj.get("rich_text").and_then(|r| r.as_array()) {
        Some(a) => a,
        None => return String::new(),
    };
    arr.iter()
        .filter_map(|t| t.get("plain_text").and_then(|p| p.as_str()))
        .collect::<Vec<_>>()
        .join("")
}

/// Extract the title from a Notion page object returned by the Search API.
///
/// Tries multiple strategies:
/// 1. Look in `properties` for a property with `"type": "title"` (works for
///    both standalone pages and database pages).
/// 2. Fall back to the `child_page.title` field (returned for some child pages).
fn extract_notion_page_title(obj: &serde_json::Value) -> String {
    // Strategy 1: properties → first "title"-type property.
    if let Some(props) = obj.get("properties").and_then(|p| p.as_object()) {
        if let Some(title) = props.values().find_map(|prop| {
            if prop.get("type")?.as_str()? != "title" {
                return None;
            }
            let arr = prop.get("title")?.as_array()?;
            let parts: Vec<&str> = arr
                .iter()
                .filter_map(|t| t.get("plain_text").and_then(|p| p.as_str()))
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(""))
            }
        }) {
            return title;
        }
    }

    // Strategy 2: child_page block title (some search results include this).
    if let Some(title) = obj
        .get("child_page")
        .and_then(|cp| cp.get("title"))
        .and_then(|t| t.as_str())
    {
        if !title.is_empty() {
            return title.to_string();
        }
    }

    String::new()
}

/// Extract a Notion page UUID from either a raw UUID or a Notion URL.
fn extract_notion_page_id(id_or_url: &str) -> String {
    // Notion URLs look like: https://www.notion.so/Page-Title-<32-hex-id>?pvs=4
    // or https://www.notion.so/<32-hex-id>?v=...
    // Strip query parameters and fragment first.
    let clean_input = id_or_url.split('?').next().unwrap_or(id_or_url);
    let clean_input = clean_input.split('#').next().unwrap_or(clean_input);

    if let Some(last_segment) = clean_input.rsplit('/').next() {
        // The id is the last 32 hex chars (possibly with hyphens)
        if let Some(pos) = last_segment.rfind('-') {
            let candidate = &last_segment[pos + 1..];
            if candidate.len() == 32 && candidate.chars().all(|c| c.is_ascii_hexdigit()) {
                return candidate.to_string();
            }
        }
        // Maybe the whole last segment is the id
        let clean = last_segment.replace('-', "");
        if clean.len() == 32 && clean.chars().all(|c| c.is_ascii_hexdigit()) {
            return clean;
        }
    }
    id_or_url.to_string()
}

/// Load a variable from `.Dirigent/.env` (preferred) or `.env` (fallback).
/// Returns `None` if neither file contains the key.
pub(crate) fn load_env_var(project_root: &Path, key: &str) -> Option<String> {
    crate::claude::load_env_var_with_dirigent_fallback(project_root, key)
}
