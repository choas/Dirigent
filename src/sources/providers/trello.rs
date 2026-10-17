//! Trello board cards.

use super::super::external::{check_response, http_client, title_with_optional_body};
use super::super::types::SourceItem;
use crate::error::DirigentError;

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

    let text = title_with_optional_body(name, desc);

    Some(SourceItem::new(url, text, source_label))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card_fixture() -> serde_json::Value {
        serde_json::json!({
            "id": "65a1b2c3d4e5f60718293a4b",
            "name": "Ship the new onboarding",
            "desc": "Blocked on the copy review.",
            "shortUrl": "https://trello.com/c/AbCdEfGh",
            "idList": "list-inbox"
        })
    }

    #[test]
    fn maps_a_card_to_an_item_keyed_on_its_short_url() {
        let item = trello_card_to_item(&card_fixture(), None, "Trello").expect("card parses");
        assert_eq!(item.external_id, "https://trello.com/c/AbCdEfGh");
        assert_eq!(
            item.text,
            "Ship the new onboarding\n\nBlocked on the copy review."
        );
    }

    #[test]
    fn card_without_a_description_is_just_its_name() {
        let mut card = card_fixture();
        card["desc"] = serde_json::json!("");
        let item = trello_card_to_item(&card, None, "Trello").expect("card parses");
        assert_eq!(item.text, "Ship the new onboarding");
    }

    #[test]
    fn list_filter_keeps_only_cards_in_the_allowed_lists() {
        let card = card_fixture();
        let allowed = ["list-inbox".to_string()];
        assert!(trello_card_to_item(&card, Some(&allowed), "Trello").is_some());

        let other = ["list-done".to_string()];
        assert!(trello_card_to_item(&card, Some(&other), "Trello").is_none());
    }

    /// With a filter active, a card whose list is unknown is excluded rather
    /// than let through.
    #[test]
    fn card_without_a_list_id_is_excluded_when_filtering() {
        let mut card = card_fixture();
        card.as_object_mut().expect("object").remove("idList");
        let allowed = ["list-inbox".to_string()];
        assert!(trello_card_to_item(&card, Some(&allowed), "Trello").is_none());
        // Without a filter the missing field does not matter.
        assert!(trello_card_to_item(&card, None, "Trello").is_some());
    }

    #[test]
    fn card_without_name_or_url_is_skipped() {
        let mut no_name = card_fixture();
        no_name.as_object_mut().expect("object").remove("name");
        assert!(trello_card_to_item(&no_name, None, "Trello").is_none());

        let mut no_url = card_fixture();
        no_url.as_object_mut().expect("object").remove("shortUrl");
        assert!(trello_card_to_item(&no_url, None, "Trello").is_none());
    }
}
