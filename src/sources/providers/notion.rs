//! Notion databases and pages: search, query, pagination and block-tree
//! rendering.

use super::super::external::{check_response, http_client, title_with_optional_body};
use super::super::types::SourceItem;
use crate::error::DirigentError;

/// Fetch all databases and pages visible to the Notion integration token.
///
/// Uses the Notion Search API (`POST /v1/search`) to list every object the
/// integration has been shared with.  Returns a vec of [`NotionObject`] with
/// both databases and pages, sorted databases-first.
pub(crate) fn fetch_notion_objects(
    token: &str,
) -> crate::error::Result<Vec<super::super::types::NotionObject>> {
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
    let mut all: Vec<super::super::types::NotionObject> =
        raw.iter().filter_map(parse_notion_search_result).collect();
    sort_notion_objects(&mut all);
    Ok(all)
}
fn sort_notion_objects(objects: &mut [super::super::types::NotionObject]) {
    objects.sort_by(|a, b| {
        let type_order = |t: &str| -> u8 { if t == "database" { 0 } else { 1 } };
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
fn parse_notion_search_result(
    obj: &serde_json::Value,
) -> Option<super::super::types::NotionObject> {
    use super::super::types::NotionObject;

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
pub(in crate::sources) fn build_notion_query_filter(
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

        // Count occurrences per text so that duplicate todo texts on the
        // same page produce distinct but position-stable IDs.
        let mut text_occurrence: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        let mut items: Vec<SourceItem> = todo_texts
            .iter()
            .map(|text| {
                let occurrence = text_occurrence.entry(text).or_insert(0);
                let mut hasher = DefaultHasher::new();
                text.hash(&mut hasher);
                occurrence.hash(&mut hasher);
                let hash = hasher.finish();
                *occurrence += 1;
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
    let text = title_with_optional_body(&title, &body);
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
    if let Some(props) = obj.get("properties").and_then(|p| p.as_object())
        && let Some(title) = props.values().find_map(|prop| {
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
        })
    {
        return title;
    }

    // Strategy 2: child_page block title (some search results include this).
    if let Some(title) = obj
        .get("child_page")
        .and_then(|cp| cp.get("title"))
        .and_then(|t| t.as_str())
        && !title.is_empty()
    {
        return title.to_string();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::NotionPageType;

    // -- Search results --

    #[test]
    fn parses_a_database_search_result() {
        let db = serde_json::json!({
            "object": "database",
            "id": "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d",
            "title": [
                { "type": "text", "plain_text": "Engineering " },
                { "type": "text", "plain_text": "Backlog" }
            ]
        });
        let obj = parse_notion_search_result(&db).expect("database parses");
        assert_eq!(obj.object_type, "database");
        // Notion splits titles across rich-text runs; they must be rejoined.
        assert_eq!(obj.title, "Engineering Backlog");
    }

    #[test]
    fn untitled_objects_fall_back_to_a_short_id() {
        let db = serde_json::json!({
            "object": "database",
            "id": "abcdef0123456789abcdef0123456789",
            "title": []
        });
        let obj = parse_notion_search_result(&db).expect("database parses");
        assert_eq!(obj.title, "Untitled (abcdef01…)");
    }

    #[test]
    fn unknown_object_types_are_skipped() {
        let block = serde_json::json!({ "object": "block", "id": "x" });
        assert!(parse_notion_search_result(&block).is_none());
    }

    // -- Pagination --

    #[test]
    fn pagination_stops_when_has_more_is_false() {
        let mut cursor = None;
        let page = serde_json::json!({ "has_more": false, "next_cursor": "abc" });
        assert!(!notion_has_more(&page, &mut cursor));
        assert_eq!(cursor, None);
    }

    #[test]
    fn pagination_follows_the_next_cursor() {
        let mut cursor = None;
        let page = serde_json::json!({ "has_more": true, "next_cursor": "cursor-2" });
        assert!(notion_has_more(&page, &mut cursor));
        assert_eq!(cursor.as_deref(), Some("cursor-2"));
    }

    /// `has_more: true` with a null cursor would otherwise loop forever.
    #[test]
    fn pagination_stops_when_the_cursor_is_missing() {
        let mut cursor = Some("stale".to_string());
        let page = serde_json::json!({ "has_more": true, "next_cursor": serde_json::Value::Null });
        assert!(!notion_has_more(&page, &mut cursor));
    }

    // -- Database pages --

    #[test]
    fn page_to_item_uses_the_title_property_whatever_it_is_named() {
        let page = serde_json::json!({
            "id": "page-1",
            "properties": {
                "Status": { "type": "status", "status": { "name": "Not started" } },
                "Task name": {
                    "type": "title",
                    "title": [{ "plain_text": "Fix the " }, { "plain_text": "flaky test" }]
                }
            }
        });
        let item = notion_page_to_item(&page, "Notion").expect("page parses");
        assert_eq!(item.external_id, "page-1");
        assert_eq!(item.text, "Fix the flaky test");
    }

    #[test]
    fn page_without_a_usable_title_is_skipped() {
        let no_title_prop = serde_json::json!({
            "id": "page-2",
            "properties": { "Status": { "type": "status" } }
        });
        assert!(notion_page_to_item(&no_title_prop, "Notion").is_none());

        let blank_title = serde_json::json!({
            "id": "page-3",
            "properties": {
                "Name": { "type": "title", "title": [{ "plain_text": "   " }] }
            }
        });
        assert!(notion_page_to_item(&blank_title, "Notion").is_none());
    }

    // -- Query filters --

    #[test]
    fn todo_list_filter_selects_unchecked_items() {
        let f = build_notion_query_filter(&NotionPageType::TodoList, None, "Complete", "");
        assert_eq!(f["filter"]["property"], "Complete");
        assert_eq!(f["filter"]["checkbox"]["equals"], false);
        assert_eq!(f["page_size"], 100);
    }

    #[test]
    fn todo_list_filter_defaults_the_property_name() {
        let f = build_notion_query_filter(&NotionPageType::TodoList, None, "", "");
        assert_eq!(f["filter"]["property"], "Done");
    }

    #[test]
    fn kanban_filter_matches_the_inbox_status_when_given() {
        let f = build_notion_query_filter(
            &NotionPageType::KanbanBoard,
            Some("Not started"),
            "Done",
            "Stage",
        );
        assert_eq!(f["filter"]["property"], "Stage");
        assert_eq!(f["filter"]["status"]["equals"], "Not started");
    }

    /// With no inbox status configured, everything that is not Done is inbox.
    #[test]
    fn kanban_filter_excludes_done_when_no_inbox_status_is_set() {
        let f = build_notion_query_filter(&NotionPageType::KanbanBoard, None, "Shipped", "");
        assert_eq!(f["filter"]["property"], "Status");
        assert_eq!(f["filter"]["status"]["does_not_equal"], "Shipped");
    }

    #[test]
    fn kanban_filter_treats_an_empty_inbox_status_as_unset() {
        let f = build_notion_query_filter(&NotionPageType::KanbanBoard, Some(""), "Done", "");
        assert_eq!(f["filter"]["status"]["does_not_equal"], "Done");
    }

    // -- Block-tree rendering --

    fn block(kind: &str, text: &str) -> serde_json::Value {
        serde_json::json!({
            "type": kind,
            kind: { "rich_text": [{ "plain_text": text }] }
        })
    }

    #[test]
    fn blocks_render_to_markdown() {
        assert_eq!(
            notion_block_to_text(&block("heading_1", "Title")).as_deref(),
            Some("# Title")
        );
        assert_eq!(
            notion_block_to_text(&block("heading_3", "Sub")).as_deref(),
            Some("### Sub")
        );
        assert_eq!(
            notion_block_to_text(&block("paragraph", "Body")).as_deref(),
            Some("Body")
        );
        assert_eq!(
            notion_block_to_text(&block("bulleted_list_item", "One")).as_deref(),
            Some("- One")
        );
        assert_eq!(
            notion_block_to_text(&block("numbered_list_item", "One")).as_deref(),
            Some("1. One")
        );
        assert_eq!(
            notion_block_to_text(&block("quote", "Said")).as_deref(),
            Some("> Said")
        );
        assert_eq!(
            notion_block_to_text(&serde_json::json!({ "type": "divider", "divider": {} }))
                .as_deref(),
            Some("---")
        );
    }

    #[test]
    fn unsupported_block_types_are_dropped() {
        let unsupported = serde_json::json!({ "type": "image", "image": {} });
        assert_eq!(notion_block_to_text(&unsupported), None);
    }

    #[test]
    fn unchecked_todos_render_and_checked_ones_are_skipped() {
        let open = serde_json::json!({
            "type": "to_do",
            "to_do": { "checked": false, "rich_text": [{ "plain_text": "Write tests" }] }
        });
        assert_eq!(
            notion_block_to_text(&open).as_deref(),
            Some("- [ ] Write tests")
        );

        let done = serde_json::json!({
            "type": "to_do",
            "to_do": { "checked": true, "rich_text": [{ "plain_text": "Write tests" }] }
        });
        assert_eq!(notion_block_to_text(&done), None);
    }

    #[test]
    fn code_blocks_keep_their_language_fence() {
        let code = serde_json::json!({
            "type": "code",
            "code": { "language": "rust", "rich_text": [{ "plain_text": "fn main() {}" }] }
        });
        assert_eq!(
            notion_block_to_text(&code).as_deref(),
            Some("```rust\nfn main() {}\n```")
        );
    }

    #[test]
    fn rich_text_without_an_array_is_empty_not_a_panic() {
        assert_eq!(rich_text_plain(&serde_json::json!({})), String::new());
        assert_eq!(
            rich_text_plain(&serde_json::json!({ "rich_text": "not an array" })),
            String::new()
        );
    }

    // -- Page splitting --

    #[test]
    fn h3_headings_start_new_sections() {
        let lines: Vec<String> = ["intro", "### First", "a", "### Second", "b"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let sections = split_h3_sections(&lines);
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].0, None);
        assert_eq!(sections[0].1, vec!["intro".to_string()]);
        assert_eq!(sections[1].0.as_deref(), Some("First"));
        assert_eq!(sections[2].0.as_deref(), Some("Second"));
        assert_eq!(sections[2].1, vec!["b".to_string()]);
    }

    #[test]
    fn preamble_section_is_titled_and_empty_sections_are_dropped() {
        assert_eq!(
            section_text(None, "body", "Page title").as_deref(),
            Some("Page title\n\nbody")
        );
        assert_eq!(section_text(None, "", "Page title"), None);
        assert_eq!(section_text(Some("H"), "", "T").as_deref(), Some("### H"));
        assert_eq!(
            section_text(Some("H"), "body", "T").as_deref(),
            Some("### H\nbody")
        );
    }

    // -- Page id extraction --

    #[test]
    fn extracts_the_page_id_from_a_notion_url() {
        assert_eq!(
            extract_notion_page_id(
                "https://www.notion.so/Some-Page-Title-1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d?pvs=4"
            ),
            "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d"
        );
        assert_eq!(
            extract_notion_page_id("https://www.notion.so/abcdef0123456789abcdef0123456789#frag"),
            "abcdef0123456789abcdef0123456789"
        );
    }

    #[test]
    fn accepts_a_hyphenated_bare_id() {
        assert_eq!(
            extract_notion_page_id("1a2b3c4d-5e6f-7a8b-9c0d-1e2f3a4b5c6d"),
            "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d"
        );
    }

    #[test]
    fn non_id_input_is_returned_unchanged() {
        assert_eq!(extract_notion_page_id("not-an-id"), "not-an-id");
    }
}
