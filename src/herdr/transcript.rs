//! Getting the run's output back from Claude Code, which herdr owns.
//!
//! Dirigent never sees the pane's stdout, so the conversation and metrics are
//! rebuilt from Claude Code's session transcript (JSONL). Sources, in order:
//!
//!   1. the Stop-hook payload's `transcript_path`,
//!   2. `~/.claude/projects/*/<session-id>.jsonl`,
//!   3. the pane's recent text (`pane.read`),
//!   4. the payload's `last_assistant_message`.
//!
//! The transcript format is observable, not a public contract, so every field
//! is read leniently and anything unknown is skipped.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// `~/.claude/projects`, where Claude Code keeps one directory per cwd.
pub(crate) fn claude_projects_dir() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".claude").join("projects"))
}

/// Find `<session-id>.jsonl` under the projects dir. The directory name
/// encodes the cwd in a way that is not worth reconstructing, so every
/// project directory is checked.
pub(crate) fn find_session_transcript(projects_dir: &Path, session_id: &str) -> Option<PathBuf> {
    let file_name = format!("{session_id}.jsonl");
    std::fs::read_dir(projects_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join(&file_name))
        .find(|candidate| candidate.is_file())
}

/// `transcript_path` from the raw Stop-hook payload (verified against Claude
/// Code 2.1: `session_id`, `transcript_path`, `cwd`, `hook_event_name`,
/// `last_assistant_message`, …).
pub(crate) fn transcript_path_from_payload(payload: &str) -> Option<PathBuf> {
    let value: Value = serde_json::from_str(payload).ok()?;
    value
        .get("transcript_path")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}

/// Read what was appended to the transcript after byte `offset`. A resumed
/// session's transcript already holds earlier runs; the offset taken before
/// launch keeps this run's output to this run.
pub(crate) fn read_from(path: &Path, offset: u64) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    // A transcript that shrank was rewritten; read all of it.
    let start = if offset <= len { offset } else { 0 };
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut raw = Vec::new();
    file.read_to_end(&mut raw).ok()?;
    Some(String::from_utf8_lossy(&raw).into_owned())
}

/// What a transcript slice says about the run.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct TranscriptSummary {
    /// `You: …` / `Claude: …` lines; `None` when nothing renderable was found.
    pub text: Option<String>,
    /// Model responses (distinct assistant message ids).
    pub num_turns: Option<u64>,
    /// Prompt tokens including cache reads and writes.
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// Render a transcript slice and total its usage. Subagent (sidechain)
/// entries are left out of both.
pub(crate) fn summarize(raw: &str) -> TranscriptSummary {
    let mut text = String::new();
    // One API response is written as one line per content block, each
    // repeating the message's usage, so usage is counted once per message id.
    let mut usage_by_message: HashMap<String, (u64, u64)> = HashMap::new();
    let mut anonymous_turns = 0u64;
    let mut anonymous_usage = (0u64, 0u64);

    for line in raw.lines() {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if entry.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if let Some(rendered) = transcript_line_text(&entry) {
            text.push_str(&rendered);
            text.push('\n');
        }
        if entry.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let usage = entry
            .pointer("/message/usage")
            .map(usage_tokens)
            .unwrap_or_default();
        match entry.pointer("/message/id").and_then(Value::as_str) {
            Some(id) => {
                usage_by_message.insert(id.to_string(), usage);
            }
            None => {
                anonymous_turns += 1;
                anonymous_usage.0 += usage.0;
                anonymous_usage.1 += usage.1;
            }
        }
    }

    let turns = usage_by_message.len() as u64 + anonymous_turns;
    let (input, output) = usage_by_message
        .values()
        .fold(anonymous_usage, |acc, u| (acc.0 + u.0, acc.1 + u.1));
    TranscriptSummary {
        text: (!text.trim().is_empty()).then_some(text),
        num_turns: (turns > 0).then_some(turns),
        input_tokens: (turns > 0).then_some(input),
        output_tokens: (turns > 0).then_some(output),
    }
}

fn usage_tokens(usage: &Value) -> (u64, u64) {
    let field = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
    (
        field("input_tokens")
            + field("cache_creation_input_tokens")
            + field("cache_read_input_tokens"),
        field("output_tokens"),
    )
}

/// Render a single JSONL transcript entry as a `"You: …"` / `"Claude: …"` line.
/// Only user and assistant messages are surfaced; tool calls are collapsed to a
/// `[tool: <name>]` marker. Returns `None` for entries with no textual body.
pub(crate) fn transcript_line_text(v: &Value) -> Option<String> {
    let label = match v.get("type").and_then(|t| t.as_str())? {
        "user" => "You",
        "assistant" => "Claude",
        _ => return None,
    };
    let body = match v.pointer("/message/content")? {
        Value::String(s) => s.clone(),
        Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                match item.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                            parts.push(t.to_string());
                        }
                    }
                    Some("tool_use") => {
                        if let Some(name) = item.get("name").and_then(|t| t.as_str()) {
                            parts.push(format!("[tool: {name}]"));
                        }
                    }
                    _ => {}
                }
            }
            parts.join("\n")
        }
        _ => return None,
    };
    if body.trim().is_empty() {
        return None;
    }
    Some(format!("{label}: {body}"))
}

/// Pick the conversation log stored on the execution: the transcript, else
/// the pane's text (only fetched when needed), else the Stop hook's last
/// assistant message.
pub(crate) fn conversation_log(
    transcript_text: Option<String>,
    pane_text: impl FnOnce() -> Option<String>,
    last_assistant_message: Option<&str>,
) -> String {
    if let Some(text) = transcript_text {
        return text;
    }
    if let Some(text) = pane_text().filter(|t| !t.trim().is_empty()) {
        return text;
    }
    match last_assistant_message {
        Some(msg) if !msg.trim().is_empty() => msg.to_string(),
        _ => "Run finished in herdr pane (no transcript captured).".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn transcript_line_text_renders_user_and_assistant() {
        let user = json(r#"{"type":"user","message":{"role":"user","content":"fix the bug"}}"#);
        assert_eq!(
            transcript_line_text(&user).as_deref(),
            Some("You: fix the bug")
        );

        let assistant = json(
            r#"{"type":"assistant","message":{"role":"assistant","content":[
                {"type":"text","text":"On it."},
                {"type":"tool_use","name":"Edit","input":{}}
            ]}}"#,
        );
        assert_eq!(
            transcript_line_text(&assistant).as_deref(),
            Some("Claude: On it.\n[tool: Edit]")
        );
    }

    #[test]
    fn transcript_line_text_skips_non_message_and_empty_entries() {
        assert_eq!(
            transcript_line_text(&json(r#"{"type":"summary","summary":"x"}"#)),
            None
        );
        assert_eq!(
            transcript_line_text(&json(
                r#"{"type":"assistant","message":{"content":[{"type":"tool_result"}]}}"#
            )),
            None
        );
        assert_eq!(
            transcript_line_text(&json(r#"{"type":"attachment","attachment":{}}"#)),
            None
        );
    }

    /// Shape taken from a real Claude Code 2.1 transcript: one response split
    /// over two lines sharing a message id and usage, plus bookkeeping entries.
    const TRANSCRIPT: &str = r#"{"type":"queue-operation","operation":"enqueue","sessionId":"s"}
{"type":"user","message":{"role":"user","content":"Reply with hi"},"sessionId":"s"}
{"type":"attachment","attachment":{"type":"x"},"sessionId":"s"}
{"type":"assistant","message":{"id":"msg_1","content":[{"type":"text","text":"Let me look."}],"usage":{"input_tokens":2,"cache_creation_input_tokens":100,"cache_read_input_tokens":50,"output_tokens":7}}}
{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","name":"Read","input":{}}],"usage":{"input_tokens":2,"cache_creation_input_tokens":100,"cache_read_input_tokens":50,"output_tokens":7}}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"file"}]}}
{"type":"assistant","isSidechain":true,"message":{"id":"msg_sub","content":[{"type":"text","text":"subagent"}],"usage":{"input_tokens":999,"output_tokens":999}}}
{"type":"assistant","message":{"id":"msg_2","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":3,"output_tokens":1}}}
not json
{"type":"cost-state","totalCostUSD":0.01}
"#;

    #[test]
    fn summarize_renders_text_and_counts_each_response_once() {
        let summary = summarize(TRANSCRIPT);
        assert_eq!(
            summary.text.as_deref(),
            Some("You: Reply with hi\nClaude: Let me look.\nClaude: [tool: Read]\nClaude: hi\n")
        );
        assert_eq!(summary.num_turns, Some(2));
        assert_eq!(summary.input_tokens, Some(152 + 3));
        assert_eq!(summary.output_tokens, Some(7 + 1));
    }

    #[test]
    fn summarize_of_nothing_reports_unknown_not_zero() {
        assert_eq!(summarize(""), TranscriptSummary::default());
        let only_user = summarize(r#"{"type":"user","message":{"content":"x"}}"#);
        assert_eq!(only_user.text.as_deref(), Some("You: x\n"));
        assert_eq!(only_user.num_turns, None);
        assert_eq!(only_user.input_tokens, None);
    }

    #[test]
    fn read_from_skips_earlier_runs_of_a_resumed_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        std::fs::write(&path, "old line\n").unwrap();
        let offset = std::fs::metadata(&path).unwrap().len();
        std::fs::write(&path, "old line\nnew line\n").unwrap();
        assert_eq!(read_from(&path, offset).as_deref(), Some("new line\n"));
        assert_eq!(read_from(&path, 0).as_deref(), Some("old line\nnew line\n"));
        // Shrunk (rewritten) file: read everything rather than nothing.
        assert_eq!(
            read_from(&path, 10_000).as_deref(),
            Some("old line\nnew line\n")
        );
        assert_eq!(read_from(&dir.path().join("missing"), 0), None);
    }

    #[test]
    fn finds_transcript_by_session_id_in_any_project_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("-a")).unwrap();
        std::fs::create_dir_all(dir.path().join("-b")).unwrap();
        std::fs::write(dir.path().join("-b/sess-1.jsonl"), "").unwrap();
        assert_eq!(
            find_session_transcript(dir.path(), "sess-1"),
            Some(dir.path().join("-b/sess-1.jsonl"))
        );
        assert_eq!(find_session_transcript(dir.path(), "sess-2"), None);
    }

    #[test]
    fn payload_transcript_path_is_read_from_the_real_stop_hook_shape() {
        let payload = r#"{"session_id":"d9","transcript_path":"/h/.claude/projects/-p/d9.jsonl",
            "cwd":"/p","hook_event_name":"Stop","stop_hook_active":false,
            "last_assistant_message":"hello"}"#;
        assert_eq!(
            transcript_path_from_payload(payload),
            Some(PathBuf::from("/h/.claude/projects/-p/d9.jsonl"))
        );
        assert_eq!(transcript_path_from_payload(r#"{"session_id":"d9"}"#), None);
        assert_eq!(transcript_path_from_payload("garbage"), None);
    }

    #[test]
    fn conversation_log_falls_back_in_order() {
        assert_eq!(
            conversation_log(Some("t".into()), || panic!("not needed"), Some("m")),
            "t"
        );
        assert_eq!(
            conversation_log(None, || Some("pane".into()), Some("m")),
            "pane"
        );
        assert_eq!(conversation_log(None, || Some("  ".into()), Some("m")), "m");
        assert!(conversation_log(None, || None, Some(" ")).contains("no transcript"));
    }
}
