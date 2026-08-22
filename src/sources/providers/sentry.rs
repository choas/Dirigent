//! Sentry unresolved issues.

use super::super::external::{check_response, http_client, url_encode};
use super::super::types::SourceItem;
use crate::error::DirigentError;

/// Fetch unresolved issues from a Sentry project using the Sentry Web API.
/// Requires an auth token and an organization/project slug pair.
/// Supports both SaaS (`https://sentry.io`) and self-hosted instances.
pub(crate) fn fetch_sentry_issues(
    token: &str,
    organization: &str,
    project: &str,
    host_url: &str,
    source_label: &str,
) -> crate::error::Result<Vec<SourceItem>> {
    if token.is_empty() {
        return Err(DirigentError::Source(
            "Sentry auth token is empty (set in source config or SENTRY_AUTH_TOKEN in .env)"
                .to_string(),
        ));
    }
    if organization.is_empty() {
        return Err(DirigentError::Source(
            "Sentry organization slug is empty".to_string(),
        ));
    }
    if project.is_empty() {
        return Err(DirigentError::Source(
            "Sentry project slug is empty".to_string(),
        ));
    }

    // Default to the org's region subdomain (e.g. https://my-org.sentry.io).
    // A configured host may use `<org>` as a placeholder for the slug, so
    // substitute it in either case.
    let template = if host_url.is_empty() {
        "https://<org>.sentry.io"
    } else {
        host_url.trim_end_matches('/')
    };
    let base = template.replace("<org>", organization);

    let client = http_client()?;

    let url = format!(
        "{}/api/0/projects/{}/{}/issues/?query={}&limit=50",
        base,
        url_encode(organization),
        url_encode(project),
        url_encode("is:unresolved"),
    );

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .map_err(|e| DirigentError::Source(format!("Sentry request failed: {e}")))?;
    let resp = check_response(resp, "Sentry API")?;

    let issues: Vec<serde_json::Value> = resp
        .json()
        .map_err(|e| DirigentError::Source(format!("Sentry response parse error: {e}")))?;

    Ok(issues
        .iter()
        .filter_map(|issue| sentry_issue_to_item(issue, source_label))
        .collect())
}

/// Map one Sentry issue object to a `SourceItem`.
fn sentry_issue_to_item(issue: &serde_json::Value, source_label: &str) -> Option<SourceItem> {
    let id = issue.get("id")?.as_str()?;
    let title = issue.get("title")?.as_str()?;
    let culprit = issue.get("culprit").and_then(|c| c.as_str()).unwrap_or("");
    let level = issue
        .get("level")
        .and_then(|l| l.as_str())
        .unwrap_or("error");
    let count = issue.get("count").and_then(|c| c.as_str()).unwrap_or("0");
    let permalink = issue
        .get("permalink")
        .and_then(|p| p.as_str())
        .unwrap_or("");

    // Dedup on the stable Sentry issue id, never the permalink. The
    // `permalink` field can be absent or differ between responses, which would
    // flip the source_ref and re-import an already-fetched issue. The permalink
    // is kept in the cue text so the link stays available.
    let external_id = format!("sentry:{}", id);

    let location = if culprit.is_empty() {
        String::new()
    } else {
        format!(" in {}", culprit)
    };

    let link = if permalink.is_empty() {
        String::new()
    } else {
        format!("\n{}", permalink)
    };

    let text = format!(
        "[{}] {}{} ({}x){}",
        level.to_uppercase(),
        title,
        location,
        count,
        link
    );

    let mut item = SourceItem::new(external_id, text, source_label);
    if !culprit.is_empty() {
        item = item.with_location(culprit, 0);
    }
    Some(item)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue_fixture() -> serde_json::Value {
        serde_json::json!({
            "id": "4507112233445566",
            "title": "TypeError: Cannot read properties of undefined",
            "culprit": "src/checkout/cart.ts in applyCoupon",
            "level": "error",
            "count": "1274",
            "permalink": "https://acme.sentry.io/issues/4507112233445566/"
        })
    }

    #[test]
    fn maps_an_issue_with_level_count_and_link() {
        let item = sentry_issue_to_item(&issue_fixture(), "Sentry").expect("issue parses");
        assert_eq!(item.external_id, "sentry:4507112233445566");
        assert!(item.text.starts_with("[ERROR] "), "got: {}", item.text);
        assert!(item.text.contains("(1274x)"), "got: {}", item.text);
        assert!(
            item.text
                .contains("https://acme.sentry.io/issues/4507112233445566/"),
            "the permalink must survive in the cue text: {}",
            item.text
        );
        assert_eq!(item.file_path, "src/checkout/cart.ts in applyCoupon");
    }

    /// The permalink can be absent or change between responses; keying on it
    /// would re-import issues that were already fetched.
    #[test]
    fn dedup_key_is_the_issue_id_not_the_permalink() {
        let mut without = issue_fixture();
        without.as_object_mut().expect("object").remove("permalink");
        let a = sentry_issue_to_item(&issue_fixture(), "Sentry").expect("parses");
        let b = sentry_issue_to_item(&without, "Sentry").expect("parses");
        assert_eq!(a.external_id, b.external_id);
    }

    #[test]
    fn missing_optional_fields_fall_back() {
        let sparse = serde_json::json!({ "id": "1", "title": "Boom" });
        let item = sentry_issue_to_item(&sparse, "Sentry").expect("issue parses");
        assert_eq!(item.text, "[ERROR] Boom (0x)");
        assert_eq!(item.file_path, "");
    }

    #[test]
    fn issue_without_id_or_title_is_skipped() {
        assert!(sentry_issue_to_item(&serde_json::json!({ "title": "x" }), "Sentry").is_none());
        assert!(sentry_issue_to_item(&serde_json::json!({ "id": "1" }), "Sentry").is_none());
    }
}
