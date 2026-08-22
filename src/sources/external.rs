use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::DirigentError;

use super::custom::SUBPROCESS_TIMEOUT_SECS;
use super::providers::notion::build_notion_query_filter;

/// Percent-encode a string for safe use as a URL query parameter value.
/// Encodes all characters that are not unreserved per RFC 3986.
pub(super) fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(b as char);
            }
            _ => {
                result.push('%');
                result.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                result.push(char::from(b"0123456789ABCDEF"[(b & 0x0F) as usize]));
            }
        }
    }
    result
}

pub(super) fn title_with_optional_body(title: &str, body: &str) -> String {
    if body.is_empty() {
        title.to_string()
    } else {
        format!("{}\n\n{}", title, body)
    }
}

/// Print a warning to stderr at most once per flag.
pub(super) fn warn_once(flag: &AtomicBool, msg: &str) {
    if !flag.swap(true, Ordering::Relaxed) {
        log::warn!("{msg}");
    }
}

/// Check that an HTTP response indicates success; return a descriptive error otherwise.
pub(super) fn check_response(
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
pub(super) fn http_client() -> crate::error::Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS))
        .build()
        .map_err(|e| DirigentError::Source(format!("HTTP client error: {e}")))
}

/// Load a variable from `.Dirigent/.env` (preferred) or `.env` (fallback).
/// Returns `None` if neither file contains the key.
pub(crate) fn load_env_var(project_root: &Path, key: &str) -> Option<String> {
    crate::claude::load_env_var_with_dirigent_fallback(project_root, key)
}

/// Single-quote a value for safe use in a POSIX shell command.
pub(super) fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Return `value` if non-empty, otherwise a `$ENV_VAR` placeholder so the
/// generated command stays runnable (and free of leaked secrets) when the
/// token/key has not been entered in Dirigent.
pub(super) fn token_or_env(value: &str, env: &str) -> String {
    if value.is_empty() {
        format!("${env}")
    } else {
        value.to_string()
    }
}

/// Return `value` if non-empty, otherwise a literal placeholder.
pub(super) fn nonempty_or<'a>(value: &'a str, placeholder: &'a str) -> &'a str {
    if value.is_empty() {
        placeholder
    } else {
        value
    }
}

/// Build a copy-pasteable shell command that reproduces the request this
/// source's "Fetch Now" performs.  HTTP-backed sources yield a `curl`
/// invocation; the `gh`-based GitHub source and shell-command sources yield
/// the underlying CLI command.  Secrets are filled in when known, otherwise a
/// `$ENV_VAR` placeholder is emitted so the command is safe to paste anywhere.
pub(crate) fn build_source_curl(
    source: &crate::settings::SourceConfig,
    project_root: &Path,
) -> String {
    use crate::settings::SourceKind;

    let token = crate::sources::resolve_source_token(source, project_root);

    match source.kind {
        SourceKind::GitHubIssues => {
            let mut cmd =
                String::from("gh issue list --json number,title,body,url --limit 50 --state open");
            if !source.filter.is_empty() {
                cmd.push_str(&format!(" --label {}", sh_quote(&source.filter)));
            }
            cmd
        }
        SourceKind::Slack => {
            let tok = token_or_env(&token, "SLACK_BOT_TOKEN");
            let channel = nonempty_or(&source.channel, "CHANNEL_ID");
            format!(
                "curl -G {url} \\\n  --data-urlencode {ch} \\\n  --data-urlencode 'limit=50' \\\n  -H {auth}",
                url = sh_quote("https://slack.com/api/conversations.history"),
                ch = sh_quote(&format!("channel={channel}")),
                auth = sh_quote(&format!("Authorization: Bearer {tok}")),
            )
        }
        SourceKind::SonarQube => {
            let base = nonempty_or(&source.host_url, "http://localhost:9000").trim_end_matches('/');
            let key = nonempty_or(&source.project_key, "PROJECT_KEY");
            let tok = token_or_env(&token, "SONAR_TOKEN");
            let url = format!(
                "{base}/api/issues/search?componentKeys={}&resolved=false&ps=100",
                url_encode(key),
            );
            format!(
                "curl -u {auth} {url}",
                auth = sh_quote(&format!("{tok}:")),
                url = sh_quote(&url),
            )
        }
        SourceKind::Trello => {
            let api_key = if source.api_key.is_empty() {
                std::env::var("TRELLO_API_KEY")
                    .ok()
                    .or_else(|| load_env_var(project_root, "TRELLO_API_KEY"))
                    .unwrap_or_default()
            } else {
                source.api_key.clone()
            };
            let api_key = token_or_env(&api_key, "TRELLO_API_KEY");
            let tok = token_or_env(&token, "TRELLO_TOKEN");
            let board = nonempty_or(&source.project_key, "BOARD_ID");
            format!(
                "curl -G {url} \\\n  --data-urlencode {key} \\\n  --data-urlencode {tok} \\\n  --data-urlencode 'fields=name,desc,shortUrl,idList' \\\n  --data-urlencode 'limit=100'",
                url = sh_quote(&format!("https://api.trello.com/1/boards/{board}/cards")),
                key = sh_quote(&format!("key={api_key}")),
                tok = sh_quote(&format!("token={tok}")),
            )
        }
        SourceKind::Asana => {
            let tok = token_or_env(&token, "ASANA_TOKEN");
            let gid = nonempty_or(&source.project_key, "PROJECT_GID");
            let url = format!(
                "https://app.asana.com/api/1.0/projects/{gid}/tasks?opt_fields=name,notes,permalink_url,completed&limit=100",
            );
            format!(
                "curl {url} \\\n  -H {auth}",
                url = sh_quote(&url),
                auth = sh_quote(&format!("Authorization: Bearer {tok}")),
            )
        }
        SourceKind::Sentry => {
            let tok = token_or_env(&token, "SENTRY_AUTH_TOKEN");
            let org = nonempty_or(&source.channel, "ORG");
            let project = nonempty_or(&source.project_key, "PROJECT");
            let template = if source.host_url.is_empty() {
                "https://<org>.sentry.io".to_string()
            } else {
                source.host_url.trim_end_matches('/').to_string()
            };
            let base = template.replace("<org>", org);
            let url = format!(
                "{base}/api/0/projects/{}/{}/issues/?query={}&limit=50",
                url_encode(org),
                url_encode(project),
                url_encode("is:unresolved"),
            );
            format!(
                "curl {url} \\\n  -H {auth}",
                url = sh_quote(&url),
                auth = sh_quote(&format!("Authorization: Bearer {tok}")),
            )
        }
        SourceKind::Notion => {
            let tok = token_or_env(&token, "NOTION_TOKEN");
            let db = nonempty_or(&source.project_key, "DATABASE_ID");
            let inbox = (!source.filter.is_empty()).then_some(source.filter.as_str());
            let filter = build_notion_query_filter(
                &source.notion_page_type,
                inbox,
                &source.notion_done_value,
                &source.notion_status_property,
            );
            let body = serde_json::to_string(&filter).unwrap_or_else(|_| "{}".to_string());
            format!(
                "curl -X POST {url} \\\n  -H {auth} \\\n  -H 'Notion-Version: 2022-06-28' \\\n  -H 'Content-Type: application/json' \\\n  -d {body}",
                url = sh_quote(&format!("https://api.notion.com/v1/databases/{db}/query")),
                auth = sh_quote(&format!("Authorization: Bearer {tok}")),
                body = sh_quote(&body),
            )
        }
        SourceKind::Custom | SourceKind::Mcp => {
            if source.command.is_empty() {
                "# No command configured for this source".to_string()
            } else {
                source.command.clone()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encode_escapes_everything_but_unreserved_characters() {
        assert_eq!(url_encode("abc-123_x.y~z"), "abc-123_x.y~z");
        assert_eq!(url_encode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(url_encode("is:unresolved"), "is%3Aunresolved");
        // Multi-byte input is encoded byte by byte.
        assert_eq!(url_encode("é"), "%C3%A9");
    }

    #[test]
    fn title_with_optional_body_omits_the_separator_when_empty() {
        assert_eq!(title_with_optional_body("Title", ""), "Title");
        assert_eq!(title_with_optional_body("Title", "Body"), "Title\n\nBody");
    }

    #[test]
    fn sh_quote_produces_a_pasteable_single_quoted_string() {
        assert_eq!(sh_quote("plain"), "'plain'");
        // An embedded single quote must not end the quoting.
        assert_eq!(sh_quote("it's"), r#"'it'\''s'"#);
    }

    #[test]
    fn token_or_env_falls_back_to_a_shell_variable_placeholder() {
        assert_eq!(token_or_env("secret", "SLACK_BOT_TOKEN"), "secret");
        assert_eq!(token_or_env("", "SLACK_BOT_TOKEN"), "$SLACK_BOT_TOKEN");
    }

    #[test]
    fn nonempty_or_uses_the_placeholder_only_when_blank() {
        assert_eq!(nonempty_or("value", "PLACEHOLDER"), "value");
        assert_eq!(nonempty_or("", "PLACEHOLDER"), "PLACEHOLDER");
    }
}
