use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::ai_backend::{BackendError, BackendRequest, BackendResponse};
use crate::claude;

const WATCHDOG_POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Default)]
struct StreamMetrics {
    cost_usd: Option<f64>,
    duration_ms: Option<u64>,
    num_turns: Option<u64>,
}

#[derive(Default)]
struct CodexStreamState {
    final_result: String,
    edited_files: Vec<String>,
    metrics: StreamMetrics,
    logged_tool_item_ids: Vec<String>,
}

fn resolve_codex_bin(cli_path: &str) -> Result<PathBuf, BackendError> {
    let bin = if cli_path.is_empty() {
        "codex"
    } else {
        cli_path
    };
    which::which(bin).map_err(|_| BackendError::NotFound { backend: "codex" })
}

fn spawn_child_waiter(
    mut child: Child,
    cancel: Arc<AtomicBool>,
    terminate: Arc<AtomicBool>,
) -> std::thread::JoinHandle<std::io::Result<ExitStatus>> {
    std::thread::spawn(move || loop {
        if cancel.load(Ordering::Relaxed) || terminate.load(Ordering::Relaxed) {
            let _ = child.kill();
            return child.wait();
        }
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        std::thread::sleep(WATCHDOG_POLL_INTERVAL);
    })
}

fn extract_text_from_event(event: &serde_json::Value) -> Option<&str> {
    event
        .get("text")
        .and_then(|t| t.as_str())
        .or_else(|| event.get("message").and_then(|t| t.as_str()))
        .or_else(|| {
            event.get("msg").and_then(|m| {
                let msg_type = m.get("type").and_then(|t| t.as_str());
                if msg_type.is_none() || msg_type == Some("text") {
                    m.get("content").and_then(|c| c.as_str())
                } else {
                    None
                }
            })
        })
        .or_else(|| {
            event
                .get("item")
                .and_then(|i| i.get("text"))
                .and_then(|t| t.as_str())
        })
        .or_else(|| {
            event
                .get("part")
                .and_then(|p| p.get("text"))
                .and_then(|t| t.as_str())
        })
}

fn is_turn_complete_event(event: &serde_json::Value) -> bool {
    matches!(
        event.get("type").and_then(|t| t.as_str()),
        Some("turn.completed") | Some("turn_complete")
    ) || matches!(
        event
            .get("msg")
            .and_then(|m| m.get("type"))
            .and_then(|t| t.as_str()),
        Some("turn_complete")
    )
}

fn extract_file_path_from_input(input: &serde_json::Value) -> Option<String> {
    input
        .get("file_path")
        .or_else(|| input.get("path"))
        .or_else(|| input.get("file"))
        .and_then(|p| p.as_str())
        .map(|s| s.to_string())
}

fn append_unique_path(paths: &mut Vec<String>, path: String) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn collect_edited_paths(event: &serde_json::Value, edited_files: &mut Vec<String>) {
    if let Some(path) = event
        .get("path")
        .and_then(|p| p.as_str())
        .or_else(|| event.get("file_path").and_then(|p| p.as_str()))
    {
        append_unique_path(edited_files, path.to_string());
    }

    let Some(item) = event.get("item") else {
        return;
    };

    if let Some(path) = item
        .get("path")
        .and_then(|p| p.as_str())
        .or_else(|| item.get("file_path").and_then(|p| p.as_str()))
    {
        append_unique_path(edited_files, path.to_string());
    }

    if item.get("type").and_then(|t| t.as_str()) == Some("file_change") {
        if let Some(changes) = item.get("changes").and_then(|c| c.as_array()) {
            for change in changes {
                if let Some(path) = change.get("path").and_then(|p| p.as_str()) {
                    append_unique_path(edited_files, path.to_string());
                }
            }
        }
    }
}

fn build_tool_detail(input: &serde_json::Value) -> String {
    if let Some(command) = input.get("command").and_then(|c| c.as_str()) {
        format!(" $ {}", command.lines().next().unwrap_or(""))
    } else if let Some(file_path) = extract_file_path_from_input(input) {
        format!(" {}", file_path)
    } else if let Some(pattern) = input.get("pattern").and_then(|p| p.as_str()) {
        format!(" \"{}\"", pattern)
    } else if let Some(arguments) = input
        .get("arguments")
        .or_else(|| input.get("input"))
        .or_else(|| input.get("params"))
    {
        match arguments {
            serde_json::Value::String(s) if !s.is_empty() => format!(" {}", s),
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                format!(" {}", arguments)
            }
            _ => String::new(),
        }
    } else {
        String::new()
    }
}

fn build_file_change_detail(item: &serde_json::Value) -> String {
    let Some(changes) = item.get("changes").and_then(|c| c.as_array()) else {
        return String::new();
    };

    let mut parts = Vec::new();
    for change in changes {
        let path = change.get("path").and_then(|p| p.as_str()).unwrap_or("?");
        let kind = change.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        if kind.is_empty() {
            parts.push(path.to_string());
        } else {
            parts.push(format!("{} {}", kind, path));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" {}", parts.join(", "))
    }
}

fn display_name_for_tool_type(tool_type: &str) -> &str {
    match tool_type {
        "command_execution" | "local_shell_call" => "Bash",
        "file_change" => "FileChange",
        "function_call" => "Tool",
        other => other,
    }
}

fn extract_tool_call_from_item(item: &serde_json::Value) -> Option<(String, String)> {
    let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if matches!(
        item_type,
        "" | "agent_message" | "message" | "reasoning" | "reasoning_summary"
    ) {
        return None;
    }

    if item_type == "file_change" {
        return Some((
            display_name_for_tool_type(item_type).to_string(),
            build_file_change_detail(item),
        ));
    }

    if item_type == "command_execution" || item_type == "local_shell_call" {
        let detail = item
            .get("command")
            .and_then(|c| c.as_str())
            .map(|command| format!(" $ {}", command.lines().next().unwrap_or("")))
            .unwrap_or_default();
        return Some((display_name_for_tool_type(item_type).to_string(), detail));
    }

    let name = item
        .get("name")
        .or_else(|| item.get("tool"))
        .or_else(|| item.get("tool_name"))
        .and_then(|n| n.as_str())
        .map(|name| name.to_string())
        .unwrap_or_else(|| display_name_for_tool_type(item_type).to_string());
    let detail = build_tool_detail(item);

    Some((name, detail))
}

fn extract_tool_call_from_event(event: &serde_json::Value) -> Option<(String, String)> {
    if let Some(item) = event.get("item") {
        return extract_tool_call_from_item(item);
    }

    let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if matches!(
        event_type,
        "tool_call" | "tool_use" | "tool" | "function_call" | "mcp_tool_call"
    ) {
        let name = event
            .get("name")
            .or_else(|| event.get("tool"))
            .or_else(|| event.get("tool_name"))
            .and_then(|n| n.as_str())
            .map(|name| name.to_string())
            .unwrap_or_else(|| display_name_for_tool_type(event_type).to_string());
        return Some((name, build_tool_detail(event)));
    }

    None
}

fn should_log_tool_event(event_type: &str, item_id: Option<&str>, logged_ids: &[String]) -> bool {
    match event_type {
        "item.started" => true,
        "item.completed" => match item_id {
            Some(id) => !logged_ids.iter().any(|seen| seen == id),
            None => true,
        },
        "tool_call" | "tool_use" | "tool" | "function_call" | "mcp_tool_call" => true,
        _ => false,
    }
}

fn process_event_stream(
    stdout_handle: impl std::io::Read,
    cancel: &AtomicBool,
    on_log: &mut impl FnMut(&str),
) -> Result<(String, Vec<String>, StreamMetrics), std::io::Error> {
    let reader = std::io::BufReader::new(stdout_handle);
    let mut state = CodexStreamState::default();

    for line_result in crate::ai_process::bounded_lines(reader) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let line = line_result?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        process_codex_stream_line(line, on_log, &mut state);
    }

    Ok((state.final_result, state.edited_files, state.metrics))
}

fn process_codex_stream_line(
    line: &str,
    on_log: &mut dyn FnMut(&str),
    state: &mut CodexStreamState,
) {
    match serde_json::from_str::<serde_json::Value>(line) {
        Ok(event) => process_codex_json_event(&event, on_log, state),
        Err(_) => append_plain_codex_line(line, on_log, &mut state.final_result),
    }
}

fn process_codex_json_event(
    event: &serde_json::Value,
    on_log: &mut dyn FnMut(&str),
    state: &mut CodexStreamState,
) {
    log_codex_tool_event(event, on_log, &mut state.logged_tool_item_ids);
    append_codex_text_event(event, on_log, &mut state.final_result);
    collect_edited_paths(event, &mut state.edited_files);
    update_codex_metrics(event, &mut state.metrics);
}

fn log_codex_tool_event(
    event: &serde_json::Value,
    on_log: &mut dyn FnMut(&str),
    logged_tool_item_ids: &mut Vec<String>,
) {
    let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let item_id = event
        .get("item")
        .and_then(|item| item.get("id"))
        .and_then(|id| id.as_str());
    if !should_log_tool_event(event_type, item_id, logged_tool_item_ids) {
        return;
    }
    if let Some((name, detail)) = extract_tool_call_from_event(event) {
        on_log(&format!("\u{2192} {}{}\n", name, detail));
        if let Some(id) = item_id {
            logged_tool_item_ids.push(id.to_string());
        }
    }
}

fn append_codex_text_event(
    event: &serde_json::Value,
    on_log: &mut dyn FnMut(&str),
    final_result: &mut String,
) {
    if let Some(text) = extract_text_from_event(event) {
        on_log(text);
        on_log("\n");
        final_result.push_str(text);
        final_result.push('\n');
    }
}

fn append_plain_codex_line(line: &str, on_log: &mut dyn FnMut(&str), final_result: &mut String) {
    on_log(&format!("{}\n", line));
    final_result.push_str(line);
    final_result.push('\n');
}

fn update_codex_metrics(event: &serde_json::Value, metrics: &mut StreamMetrics) {
    if let Some(cost) = event.get("cost_usd").and_then(|c| c.as_f64()) {
        metrics.cost_usd = Some(cost);
    }
    if let Some(duration_ms) = event.get("duration_ms").and_then(|d| d.as_u64()) {
        metrics.duration_ms = Some(duration_ms);
    }
    if is_turn_complete_event(event) {
        metrics.num_turns = Some(metrics.num_turns.unwrap_or(0).saturating_add(1));
    }
}

fn stream_stderr(
    stderr_handle: impl std::io::Read,
    cancel: Arc<AtomicBool>,
) -> Result<String, std::io::Error> {
    let reader = std::io::BufReader::new(stderr_handle);
    let mut all = String::new();
    for line_result in crate::ai_process::bounded_lines(reader) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let line = line_result?;
        if line.trim().is_empty() {
            continue;
        }
        // Keep draining after the cap so the child never stalls on a full
        // pipe, but stop growing the buffer.
        if all.len() < crate::ai_process::MAX_STDERR_BYTES {
            all.push_str(&line);
            all.push('\n');
        }
    }
    Ok(all)
}

/// Spawn the codex CLI, stream its JSON event output, and wait for exit.
/// Pre/post-run hooks (through the trust-validating [`run_hook_script`]) and
/// the final cancel mapping are owned by the `AiBackend` driver.
pub(crate) fn run_codex_cli(
    req: &BackendRequest<'_>,
    on_log: &mut dyn FnMut(&str),
    cancel: &Arc<AtomicBool>,
) -> Result<BackendResponse, BackendError> {
    let codex_bin = resolve_codex_bin(req.cli_path)?;

    let mut cmd = Command::new(codex_bin);
    cmd.arg("exec").arg("--json");
    if req.skip_permissions {
        cmd.arg("--yolo");
    }
    if !req.model.is_empty() {
        cmd.arg("--model").arg(req.model);
    }
    if !req.extra_args.trim().is_empty() {
        let args = shlex::split(req.extra_args).ok_or_else(|| BackendError::InvalidExtraArgs {
            backend: "codex",
            args: req.extra_args.to_string(),
        })?;
        for arg in args {
            cmd.arg(arg);
        }
    }
    cmd.arg(req.prompt)
        .current_dir(req.project_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let yolo_note = if req.skip_permissions { "--yolo " } else { "" };
    let model_note = if req.model.is_empty() {
        String::new()
    } else {
        format!("--model {} ", req.model)
    };
    on_log(&format!(
        "▶ codex exec --json {}{}<prompt>\n",
        yolo_note, model_note
    ));

    claude::apply_env_vars(&mut cmd, req.env_vars, on_log);
    claude::apply_dirigent_env(&mut cmd, req.project_root, on_log);

    let mut child = cmd.spawn().map_err(|source| BackendError::SpawnFailed {
        backend: "codex",
        source,
    })?;
    let (stdout_handle, stderr_handle) = crate::ai_process::take_child_streams(&mut child)
        .map_err(|source| BackendError::SpawnFailed {
            backend: "codex",
            source,
        })?;
    let terminate_child = Arc::new(AtomicBool::new(false));
    let child_waiter = spawn_child_waiter(child, Arc::clone(cancel), Arc::clone(&terminate_child));

    let cancel_for_stderr = Arc::clone(cancel);
    let stderr_thread = std::thread::spawn(move || stream_stderr(stderr_handle, cancel_for_stderr));

    let stream_result = process_event_stream(stdout_handle, cancel, &mut |s: &str| on_log(s));
    if stream_result.is_err() {
        terminate_child.store(true, Ordering::Relaxed);
    }

    let stderr = stderr_thread
        .join()
        .unwrap_or_else(|_| Ok(String::new()))
        .unwrap_or_default();

    let status = child_waiter
        .join()
        .unwrap_or_else(|_| Err(std::io::Error::other("codex waiter thread panicked")))
        .map_err(|source| BackendError::SpawnFailed {
            backend: "codex",
            source,
        })?;
    let (final_result, edited_files, metrics) =
        stream_result.map_err(|source| BackendError::StreamReadError {
            backend: "codex",
            source,
        })?;

    if !stderr.is_empty() {
        on_log(&stderr);
    }

    // A cancelled run's child was killed by the waiter; report the cancel
    // rather than a noisy non-zero exit. The driver double-checks anyway.
    if cancel.load(Ordering::Relaxed) {
        return Err(BackendError::Cancelled);
    }
    if !status.success() {
        return Err(BackendError::NonZeroExit {
            backend: "codex",
            status,
        });
    }

    Ok(BackendResponse {
        stdout: final_result,
        edited_files,
        cost_usd: metrics.cost_usd,
        duration_ms: metrics.duration_ms,
        num_turns: metrics.num_turns,
        metadata: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_from_codex_msg_content() {
        let event = serde_json::json!({
            "msg": {
                "type": "text",
                "content": "codex response"
            }
        });

        assert_eq!(extract_text_from_event(&event), Some("codex response"));
    }

    #[test]
    fn ignores_non_text_msg_content() {
        let event = serde_json::json!({
            "msg": {
                "type": "turn_complete",
                "content": "not response text"
            }
        });

        assert_eq!(extract_text_from_event(&event), None);
    }

    #[test]
    fn process_event_stream_accumulates_codex_msg_content() {
        let input = concat!(
            "{\"msg\":{\"type\":\"text\",\"content\":\"first\"}}\n",
            "{\"msg\":{\"type\":\"text\",\"content\":\"second\"}}\n",
            "{\"msg\":{\"type\":\"turn_complete\"}}\n",
        );
        let cancel = AtomicBool::new(false);
        let mut log = String::new();

        let (final_result, edited_files, metrics) =
            process_event_stream(input.as_bytes(), &cancel, &mut |text| log.push_str(text))
                .expect("stream should parse");

        assert_eq!(final_result, "first\nsecond\n");
        assert_eq!(log, "first\nsecond\n");
        assert!(edited_files.is_empty());
        assert_eq!(metrics.cost_usd, None);
        assert_eq!(metrics.duration_ms, None);
        assert_eq!(metrics.num_turns, Some(1));
    }

    #[test]
    fn process_event_stream_logs_codex_command_execution_once() {
        let input = concat!(
            "{\"type\":\"item.started\",\"item\":{\"id\":\"item_0\",\"type\":\"command_execution\",\"command\":\"/bin/zsh -lc 'printf probe'\",\"status\":\"in_progress\"}}\n",
            "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_0\",\"type\":\"command_execution\",\"command\":\"/bin/zsh -lc 'printf probe'\",\"aggregated_output\":\"probe\",\"exit_code\":0,\"status\":\"completed\"}}\n",
            "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_1\",\"type\":\"agent_message\",\"text\":\"done\"}}\n",
        );
        let cancel = AtomicBool::new(false);
        let mut log = String::new();

        let (final_result, edited_files, _metrics) =
            process_event_stream(input.as_bytes(), &cancel, &mut |text| log.push_str(text))
                .expect("stream should parse");

        assert_eq!(final_result, "done\n");
        assert_eq!(log, "\u{2192} Bash $ /bin/zsh -lc 'printf probe'\ndone\n");
        assert!(edited_files.is_empty());
    }

    #[test]
    fn process_event_stream_logs_codex_file_change_and_tracks_path() {
        let input = concat!(
            "{\"type\":\"item.started\",\"item\":{\"id\":\"item_0\",\"type\":\"file_change\",\"changes\":[{\"path\":\"src/codex.rs\",\"kind\":\"modify\"}],\"status\":\"in_progress\"}}\n",
            "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_0\",\"type\":\"file_change\",\"changes\":[{\"path\":\"src/codex.rs\",\"kind\":\"modify\"}],\"status\":\"completed\"}}\n",
        );
        let cancel = AtomicBool::new(false);
        let mut log = String::new();

        let (_final_result, edited_files, _metrics) =
            process_event_stream(input.as_bytes(), &cancel, &mut |text| log.push_str(text))
                .expect("stream should parse");

        assert_eq!(log, "\u{2192} FileChange modify src/codex.rs\n");
        assert_eq!(edited_files, vec!["src/codex.rs".to_string()]);
    }

    #[test]
    fn process_event_stream_logs_completed_tool_without_started_event() {
        let input = "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_0\",\"type\":\"function_call\",\"name\":\"read_file\",\"arguments\":{\"path\":\"src/main.rs\"}}}\n";
        let cancel = AtomicBool::new(false);
        let mut log = String::new();

        process_event_stream(input.as_bytes(), &cancel, &mut |text| log.push_str(text))
            .expect("stream should parse");

        assert_eq!(log, "\u{2192} read_file {\"path\":\"src/main.rs\"}\n");
    }

    #[test]
    fn process_event_stream_preserves_emitted_zero_metrics() {
        let input = "{\"type\":\"turn.completed\",\"cost_usd\":0.0,\"duration_ms\":0}\n";
        let cancel = AtomicBool::new(false);
        let mut log = String::new();

        let (_final_result, _edited_files, metrics) =
            process_event_stream(input.as_bytes(), &cancel, &mut |text| log.push_str(text))
                .expect("stream should parse");

        assert_eq!(metrics.cost_usd, Some(0.0));
        assert_eq!(metrics.duration_ms, Some(0));
        assert_eq!(metrics.num_turns, Some(1));
    }
}
