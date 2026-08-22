//! Asana project tasks.

use super::super::external::{check_response, http_client, title_with_optional_body};
use super::super::types::SourceItem;
use crate::error::DirigentError;

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
        .filter_map(|task| asana_task_to_item(task, source_label))
        .collect())
}

/// Map one Asana task object to a `SourceItem`. Completed tasks and tasks with
/// a blank name are skipped.
fn asana_task_to_item(task: &serde_json::Value, source_label: &str) -> Option<SourceItem> {
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

    let text = title_with_optional_body(name, notes);

    // Prefer the permalink as the dedup key; fall back to the gid when the API
    // omits it, so the task is not re-imported on every fetch.
    let external_id = if permalink.is_empty() {
        format!("asana:{}", gid)
    } else {
        permalink.to_string()
    };

    Some(SourceItem::new(external_id, text, source_label))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_fixture() -> serde_json::Value {
        serde_json::json!({
            "gid": "1201234567890123",
            "name": "Fix the login redirect",
            "notes": "Users land on / instead of the requested page.",
            "permalink_url": "https://app.asana.com/0/123/1201234567890123",
            "completed": false
        })
    }

    #[test]
    fn maps_a_task_to_an_item_keyed_on_its_permalink() {
        let item = asana_task_to_item(&task_fixture(), "Asana").expect("task parses");
        assert_eq!(
            item.external_id,
            "https://app.asana.com/0/123/1201234567890123"
        );
        assert_eq!(
            item.text,
            "Fix the login redirect\n\nUsers land on / instead of the requested page."
        );
    }

    /// Without a permalink the gid keys the item, so a re-fetch does not
    /// re-import the same task under a new id.
    #[test]
    fn falls_back_to_the_gid_when_no_permalink_is_returned() {
        let mut task = task_fixture();
        task.as_object_mut()
            .expect("object")
            .remove("permalink_url");
        let item = asana_task_to_item(&task, "Asana").expect("task parses");
        assert_eq!(item.external_id, "asana:1201234567890123");
    }

    #[test]
    fn task_without_notes_is_just_its_name() {
        let mut task = task_fixture();
        task["notes"] = serde_json::json!("");
        let item = asana_task_to_item(&task, "Asana").expect("task parses");
        assert_eq!(item.text, "Fix the login redirect");
    }

    #[test]
    fn completed_and_nameless_tasks_are_skipped() {
        let mut done = task_fixture();
        done["completed"] = serde_json::json!(true);
        assert!(asana_task_to_item(&done, "Asana").is_none());

        let mut blank = task_fixture();
        blank["name"] = serde_json::json!("   ");
        assert!(asana_task_to_item(&blank, "Asana").is_none());

        let mut no_gid = task_fixture();
        no_gid.as_object_mut().expect("object").remove("gid");
        assert!(asana_task_to_item(&no_gid, "Asana").is_none());
    }
}
