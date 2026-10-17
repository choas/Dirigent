//! Slack channel history.

use super::super::external::http_client;
use super::super::types::SourceItem;
use crate::error::DirigentError;

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
        .filter_map(|msg| slack_message_to_item(msg, channel, source_label))
        .collect())
}

/// Map one `conversations.history` message to a `SourceItem`.
///
/// The dedup key is `<channel>/<ts>`: a Slack timestamp is unique per channel,
/// not globally.
fn slack_message_to_item(
    msg: &serde_json::Value,
    channel: &str,
    source_label: &str,
) -> Option<SourceItem> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message_fixture() -> serde_json::Value {
        serde_json::json!({
            "type": "message",
            "user": "U024BE7LH",
            "text": "Deploy is stuck on the migration step",
            "ts": "1719843200.001500"
        })
    }

    #[test]
    fn maps_a_message_keyed_on_channel_and_timestamp() {
        let item =
            slack_message_to_item(&message_fixture(), "C012AB3CD", "Slack").expect("msg parses");
        assert_eq!(item.external_id, "C012AB3CD/1719843200.001500");
        assert_eq!(
            item.text,
            "[U024BE7LH] Deploy is stuck on the migration step"
        );
    }

    /// Bot posts and file shares often have no `user`.
    #[test]
    fn message_without_a_user_falls_back() {
        let mut msg = message_fixture();
        msg.as_object_mut().expect("object").remove("user");
        let item = slack_message_to_item(&msg, "C1", "Slack").expect("msg parses");
        assert!(item.text.starts_with("[unknown] "), "got: {}", item.text);
    }

    #[test]
    fn empty_or_incomplete_messages_are_skipped() {
        let mut blank = message_fixture();
        blank["text"] = serde_json::json!("   ");
        assert!(slack_message_to_item(&blank, "C1", "Slack").is_none());

        let mut no_ts = message_fixture();
        no_ts.as_object_mut().expect("object").remove("ts");
        assert!(slack_message_to_item(&no_ts, "C1", "Slack").is_none());

        // A join/leave event carries no text at all.
        let event = serde_json::json!({ "type": "message", "subtype": "channel_join" });
        assert!(slack_message_to_item(&event, "C1", "Slack").is_none());
    }
}
