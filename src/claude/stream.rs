use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::types::RunMetrics;

const WATCHDOG_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Spawn a watchdog thread that kills the child process when `cancel` is set.
/// Returns `(done_flag, join_handle)`.
pub(super) fn spawn_watchdog(
    child: &Arc<Mutex<std::process::Child>>,
    cancel: &Arc<AtomicBool>,
) -> (Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    let done = Arc::new(AtomicBool::new(false));
    let handle = {
        let child = Arc::clone(child);
        let cancel = Arc::clone(cancel);
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                if cancel.load(Ordering::Relaxed) {
                    if let Ok(mut c) = child.lock() {
                        let _ = c.kill();
                    }
                    return;
                }
                std::thread::sleep(WATCHDOG_POLL_INTERVAL);
            }
        })
    };
    (done, handle)
}

/// Spawn a thread that collects stderr into a String.
pub(super) fn spawn_stderr_reader(
    stderr_handle: std::process::ChildStderr,
) -> std::thread::JoinHandle<String> {
    use std::io::Read;
    std::thread::spawn(move || {
        let mut s = String::new();
        std::io::BufReader::new(stderr_handle)
            .read_to_string(&mut s)
            .ok();
        s
    })
}

/// State accumulated while reading the stream-json stdout.
pub(super) struct StreamState {
    pub final_result: String,
    pub edited_files: Vec<String>,
    pub metrics: RunMetrics,
}

/// Read stream-json events from stdout, dispatching each to the appropriate handler.
pub(super) fn read_stream_events(
    stdout_handle: std::process::ChildStdout,
    cancel: &AtomicBool,
    on_log: &mut dyn FnMut(&str),
) -> StreamState {
    use std::io::BufRead;
    let reader = std::io::BufReader::new(stdout_handle);
    let mut state = StreamState {
        final_result: String::new(),
        edited_files: Vec::new(),
        metrics: RunMetrics::default(),
    };

    for line_result in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let line = match line_result {
            Ok(l) => l,
            Err(_) => break,
        };
        let event: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                handle_non_json_line(&line, on_log);
                continue;
            }
        };
        dispatch_stream_event(&event, &mut state, on_log);
    }
    state
}

/// Public wrapper so `invoke.rs` can filter stderr through the same logic.
pub(super) fn handle_non_json_line_for_claude(line: &str, on_log: &mut dyn FnMut(&str)) {
    handle_non_json_line(line, on_log);
}

/// Filter OpenCode stderr: drop DEBUG and INFO noise, keep WARN/ERROR.
pub(crate) fn filter_opencode_log_line(line: &str, on_log: &mut dyn FnMut(&str)) {
    if !is_opencode_log_line(line) {
        // Not a structured log line — pass through (plain text output).
        on_log(line);
        on_log("\n");
        return;
    }
    // Drop DEBUG and INFO lines — too noisy.
    if line.starts_with("DEBUG") || line.starts_with("INFO") {
        return;
    }
    // Pass through WARN, ERROR lines.
    on_log(line);
    on_log("\n");
}

/// Handle a line that isn't valid JSON — either an OpenCode structured log line
/// or plain text from another CLI.
fn handle_non_json_line(line: &str, on_log: &mut dyn FnMut(&str)) {
    // OpenCode structured log lines: "INFO  2026-03-27T10:56:41 ..." or "DEBUG ...".
    // Detect by matching INFO/DEBUG/WARN/ERROR followed by whitespace and an ISO timestamp.
    if is_opencode_log_line(line) {
        // Always pass WARN/ERROR through — these are important.
        if line.starts_with("WARN") || line.starts_with("ERROR") {
            on_log(line);
            on_log("\n");
            return;
        }
        // Extract the one useful bit: LLM model + provider.
        if line.contains("service=llm") {
            let model = extract_kv(line, "modelID").unwrap_or("?");
            let provider = extract_kv(line, "providerID").unwrap_or("?");
            on_log(&format!("\u{2192} {} ({})\n", model, provider));
        }
        // Everything else (INFO/DEBUG) is noise — drop it.
        return;
    }
    // Pass through everything else (plain text output).
    on_log(line);
    on_log("\n");
}

/// Returns true if `line` looks like an OpenCode structured log line
/// (INFO/DEBUG/WARN/ERROR followed by whitespace and an ISO-8601 timestamp).
fn is_opencode_log_line(line: &str) -> bool {
    let rest = if let Some(r) = line.strip_prefix("INFO") {
        r
    } else if let Some(r) = line.strip_prefix("DEBUG") {
        r
    } else if let Some(r) = line.strip_prefix("WARN") {
        r
    } else if let Some(r) = line.strip_prefix("ERROR") {
        r
    } else {
        return false;
    };
    // After the level keyword there must be whitespace then a timestamp (YYYY-MM-DD).
    let trimmed = rest.trim_start();
    trimmed.len() >= 10
        && trimmed.as_bytes()[4] == b'-'
        && trimmed.as_bytes()[7] == b'-'
        && trimmed.as_bytes()[0..4].iter().all(|b| b.is_ascii_digit())
}

/// Extract the value for a `key=value` token in a space-separated log line.
fn extract_kv<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    for token in line.split_whitespace() {
        if let Some(val) = token
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix('='))
        {
            return Some(val);
        }
    }
    None
}

/// Route a single parsed JSON event to the correct handler.
fn dispatch_stream_event(
    event: &serde_json::Value,
    state: &mut StreamState,
    on_log: &mut dyn FnMut(&str),
) {
    let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match event_type {
        "assistant" => handle_assistant_event(event, &mut state.edited_files, on_log),
        "result" => handle_result_event(event, &mut state.final_result, &mut state.metrics, on_log),
        "system" | "user" | "tool" => {}
        "rate_limit_event" => handle_rate_limit_event(event, on_log),
        _ => {
            if !event_type.is_empty() {
                on_log(&format!("[{}]\n", event_type));
            }
        }
    }
}

/// Handle an "assistant" stream event: log text blocks, track tool_use edits.
fn handle_assistant_event(
    event: &serde_json::Value,
    edited_files: &mut Vec<String>,
    on_log: &mut dyn FnMut(&str),
) {
    let content = match event
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    {
        Some(c) => c,
        None => return,
    };
    for block in content {
        process_content_block(block, edited_files, on_log);
    }
}

/// Process a single content block inside an assistant message.
fn process_content_block(
    block: &serde_json::Value,
    edited_files: &mut Vec<String>,
    on_log: &mut dyn FnMut(&str),
) {
    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match block_type {
        "text" => {
            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                on_log(text);
                on_log("\n");
            }
        }
        "tool_use" => process_tool_use_block(block, edited_files, on_log),
        _ => {}
    }
}

/// Process a tool_use content block: track edited files and log the action.
fn process_tool_use_block(
    block: &serde_json::Value,
    edited_files: &mut Vec<String>,
    on_log: &mut dyn FnMut(&str),
) {
    let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("?");
    let input = block.get("input").cloned().unwrap_or_default();
    track_edited_file(name, &input, edited_files);
    let detail = extract_tool_detail(&input);
    on_log(&format!("\u{2192} {}{}\n", name, detail));
}

/// If the tool is an edit/write tool, record the file path.
fn track_edited_file(name: &str, input: &serde_json::Value, edited_files: &mut Vec<String>) {
    if !matches!(name, "Edit" | "Write" | "NotebookEdit") {
        return;
    }
    if let Some(path) = input.get("file_path").and_then(|p| p.as_str()) {
        if !edited_files.contains(&path.to_string()) {
            edited_files.push(path.to_string());
        }
    }
}

/// Build a human-readable detail string from a tool_use input.
fn extract_tool_detail(input: &serde_json::Value) -> String {
    if let Some(cmd) = input.get("command").and_then(|c| c.as_str()) {
        return format!(" $ {}", cmd.lines().next().unwrap_or(""));
    }
    if let Some(path) = input.get("file_path").and_then(|p| p.as_str()) {
        return format!(" {}", path);
    }
    if let Some(pattern) = input.get("pattern").and_then(|p| p.as_str()) {
        return format!(" \"{}\"", pattern);
    }
    String::new()
}

/// Handle a "result" stream event: capture final text and metrics.
fn handle_result_event(
    event: &serde_json::Value,
    final_result: &mut String,
    metrics: &mut RunMetrics,
    on_log: &mut dyn FnMut(&str),
) {
    if let Some(result) = event.get("result").and_then(|r| r.as_str()) {
        *final_result = result.to_string();
    }
    *metrics = extract_metrics(event);
    on_log(&format!(
        "\n\u{2713} Done ({} turns, {:.1}s, ${:.4})\n",
        metrics.num_turns,
        metrics.duration_ms as f64 / 1000.0,
        metrics.cost_usd
    ));
}

/// Extract run metrics from a result event.
fn extract_metrics(event: &serde_json::Value) -> RunMetrics {
    RunMetrics {
        cost_usd: event
            .get("cost_usd")
            .and_then(|c| c.as_f64())
            .unwrap_or(0.0),
        duration_ms: event
            .get("duration_ms")
            .and_then(|d| d.as_u64())
            .unwrap_or(0),
        num_turns: event.get("num_turns").and_then(|t| t.as_u64()).unwrap_or(0),
        input_tokens: event
            .get("total_input_tokens")
            .and_then(|t| t.as_u64())
            .or_else(|| event.get("input_tokens").and_then(|t| t.as_u64()))
            .unwrap_or(0),
        output_tokens: event
            .get("total_output_tokens")
            .and_then(|t| t.as_u64())
            .or_else(|| event.get("output_tokens").and_then(|t| t.as_u64()))
            .unwrap_or(0),
    }
}

/// Handle a "rate_limit_event": log the retry delay if present.
fn handle_rate_limit_event(event: &serde_json::Value, on_log: &mut dyn FnMut(&str)) {
    if let Some(seconds) = event
        .get("retry_after_seconds")
        .and_then(|v| v.as_f64())
        .filter(|&s| s > 0.0)
    {
        on_log(&format!(
            "\u{23f3} Rate limited, retrying in {:.0}s\n",
            seconds
        ));
    }
}
