//! Lightweight OpenTelemetry log emitter for Dirigent.
//!
//! When `DIRIGENT_OTEL_ENDPOINT` is set (e.g. `http://localhost:4318`),
//! structured log events are POSTed as OTLP HTTP logs to the collector.
//! All sends happen on background threads to avoid blocking the UI.

use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

static CLIENT: OnceLock<Option<Inner>> = OnceLock::new();

struct Inner {
    url: String,
    session_id: String,
    http: reqwest::blocking::Client,
}

/// Initialize telemetry from environment. Call once at startup.
pub(crate) fn init() {
    CLIENT.get_or_init(|| {
        let endpoint = std::env::var("DIRIGENT_OTEL_ENDPOINT").ok()?;
        if endpoint.is_empty() {
            return None;
        }
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .ok()?;
        let url = format!("{}/v1/logs", endpoint.trim_end_matches('/'));
        Some(Inner {
            url,
            session_id: uuid::Uuid::new_v4().to_string(),
            http,
        })
    });
}

// ---------------------------------------------------------------------------
// Public event emitters
// ---------------------------------------------------------------------------

pub(crate) fn emit_app_started(project: &str) {
    emit("app.started", &[("project", str_val(project))]);
}

pub(crate) fn emit_execution_started(project: &str, cue_id: i64, provider: &str, model: &str) {
    emit(
        "execution.started",
        &[
            ("project", str_val(project)),
            ("cue_id", double_val(cue_id as f64)),
            ("provider", str_val(provider)),
            ("model", str_val(model)),
        ],
    );
}

pub(crate) fn emit_execution_completed(
    project: &str,
    cue_id: i64,
    provider: &str,
    cost_usd: f64,
    duration_ms: u64,
    num_turns: u64,
    input_tokens: u64,
    output_tokens: u64,
    has_diff: bool,
) {
    emit(
        "execution.completed",
        &[
            ("project", str_val(project)),
            ("cue_id", double_val(cue_id as f64)),
            ("provider", str_val(provider)),
            ("cost_usd", double_val(cost_usd)),
            ("duration_ms", double_val(duration_ms as f64)),
            ("num_turns", double_val(num_turns as f64)),
            ("input_tokens", double_val(input_tokens as f64)),
            ("output_tokens", double_val(output_tokens as f64)),
            ("has_diff", str_val(if has_diff { "true" } else { "false" })),
        ],
    );
}

pub(crate) fn emit_execution_failed(project: &str, cue_id: i64, provider: &str, error: &str) {
    emit(
        "execution.failed",
        &[
            ("project", str_val(project)),
            ("cue_id", double_val(cue_id as f64)),
            ("provider", str_val(provider)),
            ("error", str_val(error)),
        ],
    );
}

pub(crate) fn emit_execution_rate_limited(project: &str, cue_id: i64, message: &str) {
    emit(
        "execution.rate_limited",
        &[
            ("project", str_val(project)),
            ("cue_id", double_val(cue_id as f64)),
            ("message", str_val(message)),
        ],
    );
}

pub(crate) fn emit_agent_completed(
    project: &str,
    agent_kind: &str,
    status: &str,
    duration_ms: u64,
    cue_id: Option<i64>,
) {
    let mut attrs = vec![
        ("project", str_val(project)),
        ("agent_kind", str_val(agent_kind)),
        ("status", str_val(status)),
        ("duration_ms", double_val(duration_ms as f64)),
    ];
    if let Some(id) = cue_id {
        attrs.push(("cue_id", double_val(id as f64)));
    }
    emit("agent.completed", &attrs);
}

pub(crate) fn emit_git_commit(project: &str, files_changed: usize) {
    emit(
        "git.commit",
        &[
            ("project", str_val(project)),
            ("files_changed", double_val(files_changed as f64)),
        ],
    );
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn str_val(s: &str) -> Value {
    json!({"stringValue": s})
}

fn double_val(n: f64) -> Value {
    json!({"doubleValue": n})
}

fn emit(event: &str, attributes: &[(&str, Value)]) {
    let inner = match CLIENT.get().and_then(|c| c.as_ref()) {
        Some(c) => c,
        None => return,
    };

    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string();

    let mut attrs: Vec<Value> = Vec::with_capacity(attributes.len() + 1);
    attrs.push(json!({"key": "session_id", "value": {"stringValue": inner.session_id}}));
    for (key, value) in attributes {
        attrs.push(json!({"key": key, "value": value}));
    }

    let payload = json!({
        "resourceLogs": [{
            "resource": {
                "attributes": [
                    {"key": "service.name", "value": {"stringValue": "dirigent"}}
                ]
            },
            "scopeLogs": [{
                "scope": {"name": "dirigent"},
                "logRecords": [{
                    "timeUnixNano": now_ns,
                    "severityNumber": 9,
                    "severityText": "INFO",
                    "body": {"stringValue": event},
                    "attributes": attrs
                }]
            }]
        }]
    });

    let url = inner.url.clone();
    let http = inner.http.clone();
    std::thread::spawn(move || {
        let _ = http
            .post(&url)
            .header("Content-Type", "application/json")
            .body(payload.to_string())
            .send();
    });
}
