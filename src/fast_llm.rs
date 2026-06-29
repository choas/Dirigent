//! Fast LLM: a lightweight, OpenAI-compatible chat-completion client used for
//! simple helper calls (e.g. summarizing a commit message) that don't warrant
//! spinning up the full coding-agent CLI.
//!
//! It targets the OpenAI `/v1/chat/completions` interface, which is also spoken
//! by Ollama (the default), LM Studio, vLLM, and most local/hosted gateways.
//! Configure it under Settings → "Fast LLM".

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

impl FastLlmConfig {
    /// Build a config from settings, or `None` when the Fast LLM is disabled or
    /// not configured enough to call.
    pub fn from_settings(settings: &Settings) -> Option<FastLlmConfig> {
        if !settings.fast_llm_enabled {
            return None;
        }
        let base_url = settings.fast_llm_base_url.trim();
        let model = settings.fast_llm_model.trim();
        if base_url.is_empty() || model.is_empty() {
            return None;
        }
        Some(FastLlmConfig {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: settings.fast_llm_api_key.trim().to_string(),
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
    /// Per-hunk selection accepted from the model but unused in whole-file v1.
    #[serde(default)]
    #[allow(dead_code)]
    pub hunks: Vec<serde_json::Value>,
}

/// Ask the Fast LLM to group a working diff into logical, file-disjoint change
/// sets. Returns the raw groups as parsed from the model; callers are expected
/// to normalize them (merge overlaps, account for omitted files) before use.
pub fn analyze_change_sets(
    config: &FastLlmConfig,
    diff: &str,
) -> Result<Vec<ChangeSetGroupRaw>, String> {
    const SYSTEM: &str = "You are a helpful assistant that organizes a messy git working tree. \
        Given a unified diff, group the changed files into logical, self-describing \
        feature sets. Respond with ONLY a JSON array, no markdown fences and no commentary, \
        shaped like: \
        [{\"title\": \"...\", \"description\": \"...\", \"files\": [{\"path\": \"...\", \"hunks\": []}]}]. \
        Rules: put each changed file in exactly one group; cover every file in the diff; \
        write a short imperative title (max ~50 chars) and a one-line description per group; \
        use the file paths exactly as they appear in the diff.";

    // Cap the diff so a huge change set doesn't blow past the model's context.
    let trimmed: String = diff.chars().take(12_000).collect();
    let prompt = format!("Group the changed files in the following diff:\n\n{trimmed}");
    let result = complete(config, Some(SYSTEM), &prompt)?;
    parse_change_sets(&result)
}

/// Defensively parse the model's change-set response into raw groups, tolerating
/// markdown code fences and surrounding prose.
fn parse_change_sets(response: &str) -> Result<Vec<ChangeSetGroupRaw>, String> {
    let json = crate::util::json_extract::extract_json(response);
    let groups: Vec<ChangeSetGroupRaw> =
        serde_json::from_str(&json).map_err(|e| format!("parse change-set JSON: {e}"))?;
    Ok(groups)
}

/// Summarize a git diff into a concise, conventional commit message subject line.
pub fn summarize_commit_message(config: &FastLlmConfig, diff: &str) -> Result<String, String> {
    const SYSTEM: &str = "You are a helpful assistant that writes git commit messages. \
        Given a diff, respond with a single concise commit message subject line \
        (imperative mood, no more than 72 characters). Output only the subject line, \
        with no quotes, prefixes, or explanation.";

    // Cap the diff so a huge change set doesn't blow past the model's context.
    let trimmed: String = diff.chars().take(12_000).collect();
    let prompt = format!("Summarize the following diff as a commit message:\n\n{trimmed}");
    let result = complete(config, Some(SYSTEM), &prompt)?;
    // Defensively take only the first non-empty line.
    let subject = result
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .trim_matches('"')
        .to_string();
    if subject.is_empty() {
        return Err("model returned an empty commit message".to_string());
    }
    Ok(subject)
}

#[cfg(test)]
mod tests {
    use super::*;

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
