//! Fast LLM: a lightweight, OpenAI-compatible chat-completion client used for
//! simple helper calls (e.g. summarizing a commit message) that don't warrant
//! spinning up the full coding-agent CLI.
//!
//! It targets the OpenAI `/v1/chat/completions` interface, which is also spoken
//! by Ollama (the default), LM Studio, vLLM, and most local/hosted gateways.
//! Configure it under Settings → "Fast LLM".
//!
//! Security: calls send real source data (working diffs), so the base URL is an
//! exfiltration-capable setting. It is honored only for trusted repositories
//! (see [`settings::trust`](crate::settings)), must be `https` unless the host
//! is local/private-network, and a stored API key is only ever sent to an
//! origin the user approved for that key through the settings UI.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::settings::Settings;

/// Timeout for a single Fast LLM request. Local models can be slow on first
/// load, so this is generous compared to other HTTP calls in the app.
const FAST_LLM_TIMEOUT_SECS: u64 = 120;

/// Resolved configuration for a Fast LLM call, derived from [`Settings`].
#[derive(Debug, Clone)]
pub struct FastLlmConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

/// Validate a Fast LLM base URL and return its origin (`scheme://host:port`).
///
/// Fast LLM calls carry real source data (working diffs), so the endpoint is
/// exfiltration-capable. `https` is required; plain `http` is accepted only for
/// loopback and private-network hosts (local Ollama, LM Studio, a LAN vLLM box),
/// where the traffic cannot reach an external attacker.
fn validate_base_url(base_url: &str) -> Result<String, String> {
    let url = url::Url::parse(base_url).map_err(|e| format!("invalid base URL: {e}"))?;
    let host = url.host().ok_or("base URL has no host")?;
    match url.scheme() {
        "https" => {}
        "http" => {
            let local = match &host {
                url::Host::Domain(d) => d.eq_ignore_ascii_case("localhost"),
                url::Host::Ipv4(ip) => ip.is_loopback() || ip.is_private(),
                url::Host::Ipv6(ip) => ip.is_loopback(),
            };
            if !local {
                return Err(
                    "plain http is only allowed for localhost / private-network hosts; use https"
                        .to_string(),
                );
            }
        }
        other => return Err(format!("unsupported scheme {other:?}; use https")),
    }
    let port = url.port_or_known_default().ok_or("base URL has no port")?;
    Ok(format!("{}://{}:{}", url.scheme(), host, port))
}

/// Record the user's approval of the currently configured API-key → origin
/// pair. Called when the user saves the settings dialog — editing the Fast LLM
/// section there is the explicit approval that a repository-shipped
/// `.Dirigent/settings.json` (even a trusted one) can never grant on its own.
pub fn approve_configured_key_host(settings: &Settings) {
    let api_key = settings.fast_llm_api_key.trim();
    if api_key.is_empty() {
        return;
    }
    let base_url = settings.fast_llm_base_url.trim().trim_end_matches('/');
    if let Ok(origin) = validate_base_url(base_url) {
        crate::settings::approve_fast_llm_key_host(api_key, &origin);
    }
}

impl FastLlmConfig {
    /// Build a config from settings, or `None` when the Fast LLM is disabled or
    /// not configured enough to call.
    ///
    /// The base URL is an execution-capable (exfiltration-capable) setting, so
    /// this also re-checks the repository trust gate immediately before any
    /// data leaves the machine (see [`settings::trust`](crate::settings)), and
    /// refuses to pair a stored API key with an origin the user has not
    /// explicitly approved for that key in the settings UI.
    pub fn from_settings(settings: &Settings, project_root: &Path) -> Option<FastLlmConfig> {
        if !settings.fast_llm_enabled {
            return None;
        }
        let base_url = settings.fast_llm_base_url.trim();
        let model = settings.fast_llm_model.trim();
        if base_url.is_empty() || model.is_empty() {
            return None;
        }

        // Defense-in-depth: load_settings already strips unapproved
        // execution-capable fields, but code about to send data off the
        // machine must not have to trust that every caller went through it.
        if !crate::settings::repo_settings_trusted(project_root) {
            log::warn!(
                "[fast_llm] repository settings are not trusted; refusing to call the Fast LLM"
            );
            return None;
        }

        let base_url = base_url.trim_end_matches('/').to_string();
        let origin = match validate_base_url(&base_url) {
            Ok(origin) => origin,
            Err(e) => {
                log::warn!("[fast_llm] rejecting base URL {base_url:?}: {e}");
                return None;
            }
        };

        let api_key = settings.fast_llm_api_key.trim().to_string();
        if !crate::settings::fast_llm_key_host_approved(&api_key, &origin) {
            log::warn!(
                "[fast_llm] API key is not approved for {origin}; \
                 re-save the Fast LLM section in Settings to approve it"
            );
            return None;
        }

        Some(FastLlmConfig {
            base_url,
            api_key,
            model: model.to_string(),
        })
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
    temperature: f32,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    #[serde(default)]
    message: ChatChoiceMessage,
}

#[derive(Deserialize, Default)]
struct ChatChoiceMessage {
    #[serde(default)]
    content: String,
}

/// Send a single-shot chat completion and return the assistant's text.
///
/// `system` is an optional steering instruction; `user` is the prompt content.
pub fn complete(
    config: &FastLlmConfig,
    system: Option<&str>,
    user: &str,
) -> Result<String, String> {
    let mut messages = Vec::new();
    if let Some(sys) = system {
        if !sys.is_empty() {
            messages.push(ChatMessage {
                role: "system",
                content: sys,
            });
        }
    }
    messages.push(ChatMessage {
        role: "user",
        content: user,
    });

    let body = ChatRequest {
        model: &config.model,
        messages,
        stream: false,
        temperature: 0.2,
    };

    // Final gate, independent of how `config` was built: the prompt can carry
    // source data, so never send it over plain http to a non-local host, and
    // never attach an API key to an origin the user has not approved for it.
    let origin = validate_base_url(&config.base_url)?;
    if !crate::settings::fast_llm_key_host_approved(&config.api_key, &origin) {
        return Err(format!(
            "API key is not approved for {origin} — re-save the Fast LLM section in Settings"
        ));
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(FAST_LLM_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("build HTTP client: {e}"))?;

    let url = format!("{}/chat/completions", config.base_url);
    let mut req = client.post(&url).json(&body);
    if !config.api_key.is_empty() {
        req = req.bearer_auth(&config.api_key);
    }

    let resp = req.send().map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(format!("HTTP {status}: {}", text.trim()));
    }

    let parsed: ChatResponse = resp.json().map_err(|e| format!("parse response: {e}"))?;
    let content = parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .unwrap_or_default();
    let content = content.trim().to_string();
    if content.is_empty() {
        return Err("model returned an empty response".to_string());
    }
    Ok(content)
}

/// One change-set group as returned by the model: a logical feature set with a
/// short title, a one-line description, and the files it covers.
///
/// The schema is `{ "title", "description", "files": [{ "path", "hunks": [...] }] }`.
/// In the whole-file v1 analyzer the per-hunk data is accepted but ignored, so the
/// same schema extends unchanged to the partial (per-hunk) version later.
#[derive(Debug, Clone, Deserialize)]
pub struct ChangeSetGroupRaw {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub files: Vec<ChangeSetFileRaw>,
}

/// One file within a [`ChangeSetGroupRaw`].
#[derive(Debug, Clone, Deserialize)]
pub struct ChangeSetFileRaw {
    #[serde(default)]
    pub path: String,
    /// 1-based hunk numbers the model assigned to this group (v2 partial
    /// analysis). Empty means the group owns the whole file (v1 behavior).
    #[serde(default)]
    pub hunks: Vec<serde_json::Value>,
}

impl ChangeSetFileRaw {
    /// The 1-based hunk numbers the model assigned, accepting both JSON numbers
    /// (`1`) and numeric strings (`"1"`). Non-numeric entries are ignored.
    pub fn hunk_indices(&self) -> Vec<usize> {
        self.hunks
            .iter()
            .filter_map(|v| {
                v.as_u64()
                    .map(|n| n as usize)
                    .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
            })
            .collect()
    }
}

/// Defensively parse the model's change-set response into raw groups, tolerating
/// markdown code fences and surrounding prose.
pub(crate) fn parse_change_sets(response: &str) -> Result<Vec<ChangeSetGroupRaw>, String> {
    let json = crate::util::json_extract::extract_json(response);
    let groups: Vec<ChangeSetGroupRaw> =
        serde_json::from_str(&json).map_err(|e| format!("parse change-set JSON: {e}"))?;
    Ok(groups)
}

/// Summarize a git diff into a detailed conventional commit message: a concise
/// subject line followed by a body explaining what changed and why.
pub fn summarize_commit_message(config: &FastLlmConfig, diff: &str) -> Result<String, String> {
    const SYSTEM: &str = "You are a helpful assistant that writes git commit messages. \
        Given a diff, respond with a complete commit message: a concise subject line \
        (imperative mood, no more than 72 characters), then a blank line, then a short \
        body of 2-6 lines (plain sentences or bullet points) explaining what changed \
        and why. Output only the commit message, with no quotes, markdown fences, \
        prefixes, or explanation.";

    // Cap the diff so a huge change set doesn't blow past the model's context.
    let trimmed: String = diff.chars().take(12_000).collect();
    let prompt = format!("Summarize the following diff as a commit message:\n\n{trimmed}");
    let result = complete(config, Some(SYSTEM), &prompt)?;
    // Defensively strip wrapping quotes/fences but keep the multi-line body.
    let message = result
        .trim()
        .trim_matches('"')
        .trim_matches('`')
        .trim()
        .to_string();
    if message.is_empty() {
        return Err("model returned an empty commit message".to_string());
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_requires_https_for_remote_hosts() {
        // https anywhere is fine.
        assert_eq!(
            validate_base_url("https://api.openai.com/v1").unwrap(),
            "https://api.openai.com:443"
        );
        // Plain http is fine for loopback and private-network hosts.
        assert_eq!(
            validate_base_url("http://localhost:11434/v1").unwrap(),
            "http://localhost:11434"
        );
        assert!(validate_base_url("http://127.0.0.1:8080/v1").is_ok());
        assert!(validate_base_url("http://[::1]:11434/v1").is_ok());
        assert!(validate_base_url("http://192.168.1.5:11434/v1").is_ok());
        // Plain http to a public host is silent exfiltration — rejected.
        assert!(validate_base_url("http://evil.tld/v1").is_err());
        assert!(validate_base_url("http://8.8.8.8/v1").is_err());
        // Non-http(s) schemes and garbage are rejected.
        assert!(validate_base_url("ftp://localhost/v1").is_err());
        assert!(validate_base_url("file:///etc/passwd").is_err());
        assert!(validate_base_url("not a url").is_err());
    }

    #[test]
    fn parse_change_sets_well_formed() {
        let resp = r#"[
            {"title": "Add login", "description": "auth flow", "files": [{"path": "src/auth.rs", "hunks": []}]},
            {"title": "Docs", "description": "readme", "files": [{"path": "README.md"}]}
        ]"#;
        let groups = parse_change_sets(resp).expect("should parse");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].title, "Add login");
        assert_eq!(groups[0].files[0].path, "src/auth.rs");
        // A file object without a `hunks` key still parses (hunks defaults empty).
        assert_eq!(groups[1].files[0].path, "README.md");
    }

    #[test]
    fn parse_change_sets_fenced_with_prose() {
        let resp = "Sure! Here are the groups:\n```json\n[{\"title\":\"T\",\"description\":\"d\",\"files\":[{\"path\":\"a.rs\"}]}]\n```\nHope that helps.";
        let groups = parse_change_sets(resp).expect("should parse fenced output");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].files[0].path, "a.rs");
    }

    #[test]
    fn parse_change_sets_malformed_errors() {
        // Not JSON of the expected shape at all.
        assert!(parse_change_sets("the model refused to answer").is_err());
        // A JSON object rather than the expected array.
        assert!(parse_change_sets(r#"{"oops": true}"#).is_err());
    }
}
