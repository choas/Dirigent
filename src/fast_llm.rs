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
