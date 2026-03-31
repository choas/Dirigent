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
                source_id: String::new(),
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
                source_id: String::new(),
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
                source_id: String::new(),
            })
        })
        .collect())
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

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS))
        .build()
        .map_err(|e| DirigentError::Source(format!("HTTP client error: {e}")))?;

    // Fetch cards with their list info so we can filter by list name.
    let url = format!(
        "https://api.trello.com/1/boards/{}/cards?key={}&token={}&fields=name,desc,shortUrl,idList&limit=100",
        board_id, api_key, token,
    );

    let resp = client.get(&url).send().map_err(|e| {
        DirigentError::Source(format!("Trello request failed: {}", e.without_url()))
    })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(DirigentError::Source(format!(
            "Trello API error ({}): {body}",
            status
        )));
    }

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
    let lists_url = format!(
        "https://api.trello.com/1/boards/{}/lists?key={}&token={}&fields=name",
        board_id, api_key, token,
    );
    let lists_resp = client.get(&lists_url).send().map_err(|e| {
        DirigentError::Source(format!("Trello lists request failed: {}", e.without_url()))
    })?;

    if !lists_resp.status().is_success() {
        let status = lists_resp.status();
        let body = lists_resp.text().unwrap_or_default();
        return Err(DirigentError::Source(format!(
            "Trello lists API error ({status}): {body}"
        )));
    }

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

    Some(SourceItem {
        external_id: url.to_string(),
        text,
        source_label: source_label.to_string(),
        source_id: String::new(),
    })
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

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS))
        .build()
        .map_err(|e| DirigentError::Source(format!("HTTP client error: {e}")))?;

    let url = format!(
        "https://app.asana.com/api/1.0/projects/{}/tasks?opt_fields=name,notes,permalink_url,completed&limit=100",
        project_gid,
    );

    let resp_raw = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .map_err(|e| DirigentError::Source(format!("Asana request failed: {e}")))?;

    if !resp_raw.status().is_success() {
        let status = resp_raw.status();
        let body = resp_raw.text().unwrap_or_default();
        return Err(DirigentError::Source(format!(
            "Asana API error ({status}): {body}"
        )));
    }

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

            Some(SourceItem {
                external_id,
                text,
                source_label: source_label.to_string(),
                source_id: String::new(),
            })
        })
        .collect())
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
    use crate::settings::NotionPageType;

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

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS))
        .build()
        .map_err(|e| DirigentError::Source(format!("HTTP client error: {e}")))?;

    let url = format!("https://api.notion.com/v1/databases/{}/query", database_id,);

    // Build a filter to exclude completed items.
    let filter = match page_type {
        NotionPageType::TodoList => {
            let prop = if done_property.is_empty() {
                "Done"
            } else {
                done_property
            };
            serde_json::json!({
                "filter": {
                    "property": prop,
                    "checkbox": { "equals": false }
                },
                "page_size": 100
            })
        }
        NotionPageType::KanbanBoard => {
            let status_prop = if status_property.is_empty() {
                "Status"
            } else {
                status_property
            };
            if let Some(status) = inbox_status.filter(|s| !s.is_empty()) {
                serde_json::json!({
                    "filter": {
                        "property": status_prop,
                        "status": { "equals": status }
                    },
                    "page_size": 100
                })
            } else {
                // No filter — fetch all non-done items.
                let done_val = if done_property.is_empty() {
                    "Done"
                } else {
                    done_property
                };
                serde_json::json!({
                    "filter": {
                        "property": status_prop,
                        "status": { "does_not_equal": done_val }
                    },
                    "page_size": 100
                })
            }
        }
    };

    let mut all_results: Vec<serde_json::Value> = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let mut body = filter.clone();
        if let Some(ref c) = cursor {
            body["start_cursor"] = serde_json::json!(c);
        }

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Notion-Version", "2022-06-28")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .map_err(|e| DirigentError::Source(format!("Notion request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let resp_body = resp.text().unwrap_or_default();
            return Err(DirigentError::Source(format!(
                "Notion API error ({status}): {resp_body}"
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .map_err(|e| DirigentError::Source(format!("Notion response parse error: {e}")))?;

        if let Some(page_results) = json.get("results").and_then(|v| v.as_array()) {
            all_results.extend(page_results.iter().cloned());
        }

        let has_more = json
            .get("has_more")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !has_more {
            break;
        }

        cursor = json
            .get("next_cursor")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        if cursor.is_none() {
            break;
        }
    }

    Ok(all_results
        .iter()
        .filter_map(|page| notion_page_to_item(page, source_label))
        .collect())
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

    Some(SourceItem {
        external_id: id.to_string(),
        text: title,
        source_label: source_label.to_string(),
        source_id: String::new(),
    })
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

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS))
        .build()
        .map_err(|e| DirigentError::Source(format!("HTTP client error: {e}")))?;

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
                    done_val: { "checkbox": true }
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
                    status_prop: {
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

/// Extract a Notion page UUID from either a raw UUID or a Notion URL.
fn extract_notion_page_id(id_or_url: &str) -> &str {
    // Notion URLs look like: https://www.notion.so/Page-Title-<32-hex-id>
    // or https://www.notion.so/<32-hex-id>
    if let Some(last_segment) = id_or_url.rsplit('/').next() {
        // The id is the last 32 hex chars (possibly with hyphens)
        if let Some(pos) = last_segment.rfind('-') {
            let candidate = &last_segment[pos + 1..];
            if candidate.len() == 32 && candidate.chars().all(|c| c.is_ascii_hexdigit()) {
                return candidate;
            }
        }
        // Maybe the whole last segment is the id
        let clean = last_segment.replace('-', "");
        if clean.len() == 32 && clean.chars().all(|c| c.is_ascii_hexdigit()) {
            return last_segment;
        }
    }
    id_or_url
}

/// Load a variable from `.Dirigent/.env` (preferred) or `.env` (fallback).
/// Returns `None` if neither file contains the key.
pub(crate) fn load_env_var(project_root: &Path, key: &str) -> Option<String> {
    crate::claude::load_env_var_with_dirigent_fallback(project_root, key)
}
