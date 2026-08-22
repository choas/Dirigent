use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::ai_backend::{BackendError, BackendRequest, BackendResponse, SharedLog};
use crate::ai_process;
use crate::claude;

#[derive(Default)]
struct StreamMetrics {
    cost_usd: f64,
    num_turns: u64,
}

pub(crate) fn get_available_models(cli_path: &str) -> Vec<String> {
    let bin = if cli_path.is_empty() {
        "gemini"
    } else {
        cli_path
    };

    let output = Command::new(bin).arg("models").arg("list").output();

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }
        _ => {
            vec![
                "gemini-3-pro-preview".to_string(),
                "gemini-3-flash-preview".to_string(),
                "gemini-2.5-pro".to_string(),
                "gemini-2.5-flash".to_string(),
                "gemini-2.5-flash-lite".to_string(),
            ]
        }
    }
}

fn resolve_gemini_bin(cli_path: &str) -> Result<PathBuf, BackendError> {
    let bin = if cli_path.is_empty() {
        "gemini"
    } else {
        cli_path
    };
    which::which(bin).map_err(|_| BackendError::NotFound { backend: "gemini" })
}

fn build_gemini_command(
    gemini_bin: &Path,
    req: &BackendRequest<'_>,
    on_log: &mut dyn FnMut(&str),
) -> Result<Command, BackendError> {
    use std::process::Stdio;

    let mut cmd = Command::new(gemini_bin);
    cmd.arg("-y").arg("--output-format").arg("json");

    if !req.model.is_empty() {
        cmd.arg("--model").arg(req.model);
    }

    if !req.extra_args.trim().is_empty() {
        let args = shlex::split(req.extra_args).ok_or_else(|| BackendError::InvalidExtraArgs {
            backend: "gemini",
            args: req.extra_args.to_string(),
        })?;
        for arg in args {
            cmd.arg(arg);
        }
    }

    cmd.arg(req.prompt);

    claude::apply_env_vars(&mut cmd, req.env_vars, on_log);
    crate::claude::apply_dirigent_env(&mut cmd, req.project_root, on_log);

    cmd.current_dir(req.project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    Ok(cmd)
}

fn process_event_stream(
    stdout_handle: impl std::io::Read,
    cancel: &AtomicBool,
    on_log: &mut impl FnMut(&str),
) -> Result<(String, Vec<String>, StreamMetrics), std::io::Error> {
    let reader = std::io::BufReader::new(stdout_handle);
    let mut final_result = String::new();
    let mut edited_files: Vec<String> = Vec::new();
    let mut metrics = StreamMetrics::default();
    let mut json_buffer = String::new();

    for line_result in ai_process::bounded_lines(reader) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let line = line_result?;

        let Some(json_str) = ai_process::stream_json_payload(&line) else {
            continue;
        };

        let Ok(event) = serde_json::from_str::<serde_json::Value>(json_str) else {
            json_buffer.push_str(json_str);
            json_buffer.push('\n');
            continue;
        };

        dispatch_event(
            &event,
            &mut final_result,
            &mut edited_files,
            &mut metrics,
            on_log,
        );
    }

    if final_result.is_empty() && !json_buffer.is_empty() {
        if let Ok(blob) = serde_json::from_str::<serde_json::Value>(&json_buffer) {
            handle_json_blob(&blob, &mut final_result, &mut metrics, on_log);
        } else {
            on_log(&json_buffer);
        }
    }

    Ok((final_result, edited_files, metrics))
}

fn handle_json_blob(
    blob: &serde_json::Value,
    final_result: &mut String,
    metrics: &mut StreamMetrics,
    on_log: &mut impl FnMut(&str),
) {
    if let Some(response) = blob.get("response").and_then(|r| r.as_str()) {
        on_log(response);
        on_log("\n");
        *final_result = response.to_string();
    }
    if let Some(turns) = blob
        .get("stats")
        .and_then(|s| s.get("tools"))
        .and_then(|t| t.get("totalCalls"))
        .and_then(|c| c.as_u64())
    {
        metrics.num_turns = turns;
    }
}

fn dispatch_event(
    event: &serde_json::Value,
    final_result: &mut String,
    edited_files: &mut Vec<String>,
    metrics: &mut StreamMetrics,
    on_log: &mut impl FnMut(&str),
) {
    let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match event_type {
        "initialization" => {
            if let Some(model) = event.get("model").and_then(|m| m.as_str()) {
                on_log(&format!("[Gemini: {}]\n", model));
            }
        }
        "messages" => handle_messages_event(event, final_result, on_log),
        "tools" => handle_tools_event(event, edited_files, on_log),
        "turn_complete" => {
            metrics.num_turns += 1;
            on_log("\n");
        }
        "response" => {
            if let Some(text) = event.get("text").and_then(|t| t.as_str()) {
                emit_text(text, final_result, on_log);
            }
        }
        _ => {}
    }
}

fn emit_text(text: &str, final_result: &mut String, on_log: &mut impl FnMut(&str)) {
    on_log(text);
    on_log("\n");
    final_result.push_str(text);
    final_result.push('\n');
}

fn handle_messages_event(
    event: &serde_json::Value,
    final_result: &mut String,
    on_log: &mut impl FnMut(&str),
) {
    let Some(messages) = event.get("messages").and_then(|m| m.as_array()) else {
        return;
    };
    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role != "assistant" && role != "model" {
            continue;
        }
        if let Some(content) = msg.get("content") {
            handle_message_content(content, final_result, on_log);
        }
    }
}

fn handle_message_content(
    content: &serde_json::Value,
    final_result: &mut String,
    on_log: &mut impl FnMut(&str),
) {
    if let Some(text) = content.as_str() {
        emit_text(text, final_result, on_log);
    } else if let Some(parts) = content.as_array() {
        for part in parts {
            if part.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                    emit_text(text, final_result, on_log);
                }
            }
        }
    }
}

fn handle_tools_event(
    event: &serde_json::Value,
    edited_files: &mut Vec<String>,
    on_log: &mut impl FnMut(&str),
) {
    let Some(tools) = event.get("tools").and_then(|t| t.as_array()) else {
        return;
    };
    for tool in tools {
        let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("?");
        if ai_process::is_file_related_tool(name) {
            record_edited_file(tool, edited_files);
        }
        on_log(&format!("\u{2192} {}", name));
        if let Some(input) = tool.get("input") {
            let detail = ai_process::build_tool_detail(input, false);
            if !detail.is_empty() {
                on_log(&detail);
            }
        }
        on_log("\n");
    }
}

fn record_edited_file(tool: &serde_json::Value, edited_files: &mut Vec<String>) {
    let Some(input) = tool.get("input") else {
        return;
    };
    let Some(path) = ai_process::extract_file_path_from_input(input) else {
        return;
    };
    if !edited_files.contains(&path) {
        edited_files.push(path);
    }
}

fn is_stderr_noise(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.starts_with("(node:") && trimmed.contains("DeprecationWarning") {
        return true;
    }
    if trimmed.starts_with("(Use `node --trace-deprecation") {
        return true;
    }
    if trimmed.starts_with("    at ") || trimmed.starts_with("    at\t") {
        return true;
    }
    if trimmed == "{" || trimmed == "}" || trimmed.starts_with("status:") {
        return true;
    }
    if trimmed.contains("YOLO mode is enabled") {
        return true;
    }
    if trimmed.starts_with("Ripgrep is not available") {
        return true;
    }
    if trimmed.starts_with("MCP issues detected") {
        return true;
    }
    false
}

fn simplify_stderr_line(line: &str) -> String {
    if let Some(idx) = line.find("Retrying with backoff") {
        let prefix = &line[..idx];
        return format!("{}Retrying…", prefix);
    }
    line.to_string()
}

fn friendly_error_from_stderr(stderr: &str) -> Option<String> {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("503") && (lower.contains("high demand") || lower.contains("unavailable")) {
        return Some(
            "Gemini API is temporarily unavailable (503). The model is experiencing high demand — please try again later.".to_string(),
        );
    }
    if lower.contains("429") || lower.contains("rate limit") || lower.contains("quota") {
        return Some(
            "Gemini API rate limit exceeded (429). Please wait a moment and try again.".to_string(),
        );
    }
    if lower.contains("401") || lower.contains("unauthorized") || lower.contains("unauthenticated")
    {
        return Some(
            "Gemini API authentication failed. Please check your API key or credentials."
                .to_string(),
        );
    }
    None
}

fn check_exit_status(
    exit_status: Option<std::process::ExitStatus>,
    stderr: &str,
    on_log: &SharedLog,
) -> Result<(), BackendError> {
    let Some(status) = exit_status.filter(|s| !s.success()) else {
        return Ok(());
    };
    if !stderr.is_empty() {
        let display_msg = friendly_error_from_stderr(stderr).unwrap_or_else(|| stderr.to_string());
        ai_process::with_locked_log(on_log, "non-zero exit error", |log| {
            log(&format!("\nError: {}\n", display_msg));
        });
    }
    Err(BackendError::NonZeroExit {
        backend: "gemini",
        status,
    })
}

/// Spawn the gemini CLI, stream its JSON event output, and wait for exit.
/// Pre/post-run hooks and the final cancel mapping are owned by the
/// `AiBackend` driver.
pub(crate) fn run_gemini_cli(
    req: &BackendRequest<'_>,
    on_log: &SharedLog,
    cancel: &Arc<AtomicBool>,
) -> Result<BackendResponse, BackendError> {
    let gemini_bin = resolve_gemini_bin(req.cli_path)?;

    let mut child = ai_process::with_locked_log(on_log, "command build", |log| {
        build_gemini_command(&gemini_bin, req, &mut *log)?
            .spawn()
            .map_err(|source| BackendError::SpawnFailed {
                backend: "gemini",
                source,
            })
    })?;

    let (stdout_handle, stderr_handle) =
        ai_process::take_child_streams(&mut child).map_err(|source| BackendError::SpawnFailed {
            backend: "gemini",
            source,
        })?;

    let child = Arc::new(std::sync::Mutex::new(child));
    let done = Arc::new(AtomicBool::new(false));
    let watchdog = ai_process::spawn_cancel_watchdog(&child, cancel, &done);

    let stderr_thread =
        ai_process::spawn_stderr_collector(stderr_handle, Arc::clone(on_log), |line, log| {
            if is_stderr_noise(line) {
                return;
            }
            let display = simplify_stderr_line(line);
            log(&display);
            log("\n");
        });

    let stream_result = process_event_stream(stdout_handle, cancel, &mut |text: &str| {
        ai_process::with_locked_log(on_log, "stdout", |log| log(text));
    });

    done.store(true, Ordering::Relaxed);

    // If the stdout stream errored, the child may still be running. Kill it
    // first so the subsequent `wait` cannot block indefinitely, then drain the
    // helper threads before surfacing the read error.
    let (final_result, edited_files, stream_metrics) = match stream_result {
        Ok(result) => result,
        Err(source) => {
            ai_process::kill_child(&child);
            let _ = watchdog.join();
            let _ = ai_process::wait_for_child(&child);
            let _ = stderr_thread.join();
            return Err(BackendError::StreamReadError {
                backend: "gemini",
                source,
            });
        }
    };

    let _ = watchdog.join();
    let exit_status = ai_process::wait_for_child(&child).ok();
    let stderr = stderr_thread.join().unwrap_or_default();

    // A cancelled run's child was killed by the watchdog; report the cancel
    // rather than a noisy non-zero exit. The driver double-checks anyway.
    if cancel.load(Ordering::Relaxed) {
        return Err(BackendError::Cancelled);
    }

    check_exit_status(exit_status, &stderr, on_log)?;

    ai_process::log_empty_result_stderr(on_log, &final_result, &stderr);

    Ok(BackendResponse {
        stdout: final_result,
        edited_files,
        cost_usd: (stream_metrics.cost_usd > 0.0).then_some(stream_metrics.cost_usd),
        duration_ms: None,
        num_turns: (stream_metrics.num_turns > 0).then_some(stream_metrics.num_turns),
        metadata: None,
    })
}
