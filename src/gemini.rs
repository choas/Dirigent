use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::ai_process;
use crate::claude;

#[derive(Debug)]
pub(crate) enum GeminiError {
    NotFound,
    SpawnFailed(std::io::Error),
    StreamReadError(std::io::Error),
    Cancelled,
    NonZeroExit(std::process::ExitStatus),
    InvalidExtraArgs(String),
}

impl std::error::Error for GeminiError {}

impl std::fmt::Display for GeminiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GeminiError::NotFound => write!(f, "gemini CLI not found on PATH"),
            GeminiError::SpawnFailed(e) => write!(f, "failed to spawn gemini: {e}"),
            GeminiError::StreamReadError(e) => write!(f, "failed to read gemini stdout: {e}"),
            GeminiError::Cancelled => write!(f, "cancelled"),
            GeminiError::InvalidExtraArgs(args) => {
                write!(f, "failed to parse extra_args (unmatched quote?): {args}")
            }
            GeminiError::NonZeroExit(status) => {
                write!(f, "gemini exited with {status}")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GeminiResponse {
    pub stdout: String,
    pub edited_files: Vec<String>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    pub num_turns: Option<u64>,
}

#[derive(Default)]
struct StreamMetrics {
    cost_usd: f64,
    num_turns: u64,
}

pub(crate) struct GeminiRunConfig<'a> {
    pub model: &'a str,
    pub cli_path: &'a str,
    pub extra_args: &'a str,
    pub env_vars: &'a str,
    pub pre_run_script: &'a str,
    pub post_run_script: &'a str,
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

fn run_hook_script(
    label: &str,
    script: &str,
    project_root: &Path,
    on_log: &mut impl FnMut(&str),
    fail_on_error: bool,
) -> Result<(), GeminiError> {
    if script.trim().is_empty() {
        return Ok(());
    }
    on_log(&format!("\u{25B6} {}: {}\n", label, script.trim()));
    let result = Command::new("sh")
        .arg("-c")
        .arg(script.trim())
        .current_dir(project_root)
        .output();
    match result {
        Ok(output) => {
            if !output.stdout.is_empty() {
                on_log(&String::from_utf8_lossy(&output.stdout));
            }
            if !output.stderr.is_empty() {
                on_log(&String::from_utf8_lossy(&output.stderr));
            }
            if !output.status.success() {
                let msg = format!("{} script failed (exit {})", label, output.status);
                on_log(&format!("\u{2717} {}\n", msg));
                if fail_on_error {
                    return Err(GeminiError::SpawnFailed(std::io::Error::other(msg)));
                }
            }
        }
        Err(e) => {
            on_log(&format!("\u{2717} {} script error: {}\n", label, e));
            if fail_on_error {
                return Err(GeminiError::SpawnFailed(e));
            }
        }
    }
    Ok(())
}

fn resolve_gemini_bin(cli_path: &str) -> Result<PathBuf, GeminiError> {
    let bin = if cli_path.is_empty() {
        "gemini"
    } else {
        cli_path
    };
    which::which(bin).map_err(|_| GeminiError::NotFound)
}

fn build_gemini_command(
    gemini_bin: &Path,
    prompt: &str,
    project_root: &Path,
    config: &GeminiRunConfig<'_>,
    on_log: &mut dyn FnMut(&str),
) -> Result<Command, GeminiError> {
    use std::process::Stdio;

    let mut cmd = Command::new(gemini_bin);
    cmd.arg("-y").arg("--output-format").arg("json");

    if !config.model.is_empty() {
        cmd.arg("--model").arg(config.model);
    }

    if !config.extra_args.trim().is_empty() {
        let args = shlex::split(config.extra_args)
            .ok_or_else(|| GeminiError::InvalidExtraArgs(config.extra_args.to_string()))?;
        for arg in args {
            cmd.arg(arg);
        }
    }

    cmd.arg(prompt);

    claude::apply_env_vars(&mut cmd, config.env_vars, on_log);
    crate::claude::apply_dirigent_env(&mut cmd, project_root, on_log);

    cmd.current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    Ok(cmd)
}

fn process_event_stream(
    stdout_handle: impl std::io::Read,
    cancel: &AtomicBool,
    on_log: &mut impl FnMut(&str),
) -> Result<(String, Vec<String>, StreamMetrics), std::io::Error> {
    use std::io::BufRead;

    let reader = std::io::BufReader::new(stdout_handle);
    let mut final_result = String::new();
    let mut edited_files: Vec<String> = Vec::new();
    let mut metrics = StreamMetrics::default();
    let mut json_buffer = String::new();

    for line_result in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let line = line_result?;

        let json_str = if let Some(data) = line.strip_prefix("data:") {
            data.trim()
        } else if line.is_empty()
            || line.starts_with("event:")
            || line.starts_with("id:")
            || line.starts_with("retry:")
            || line.starts_with(':')
        {
            continue;
        } else {
            line.trim()
        };

        if json_str.is_empty() {
            continue;
        }

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

fn wait_for_child(child: &Arc<Mutex<std::process::Child>>) -> Option<std::process::ExitStatus> {
    match child.lock() {
        Ok(mut c) => c.wait().ok(),
        Err(poisoned) => poisoned.into_inner().wait().ok(),
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

fn check_exit_status<F: FnMut(&str)>(
    exit_status: Option<std::process::ExitStatus>,
    stderr: &str,
    on_log: &Arc<Mutex<F>>,
) -> Result<(), GeminiError> {
    let Some(status) = exit_status.filter(|s| !s.success()) else {
        return Ok(());
    };
    if !stderr.is_empty() {
        let display_msg = friendly_error_from_stderr(stderr).unwrap_or_else(|| stderr.to_string());
        let mut log = on_log.lock().unwrap_or_else(|e| {
            log::error!(
                "Mutex poisoned while acquiring on_log for non-zero exit error: {:?}",
                e
            );
            e.into_inner()
        });
        log(&format!("\nError: {}\n", display_msg));
    }
    Err(GeminiError::NonZeroExit(status))
}

pub(crate) fn invoke_gemini_streaming(
    prompt: &str,
    project_root: &Path,
    config: &GeminiRunConfig<'_>,
    on_log: impl FnMut(&str) + Send + 'static,
    cancel: Arc<AtomicBool>,
) -> Result<GeminiResponse, GeminiError> {
    let on_log = Arc::new(Mutex::new(on_log));

    let gemini_bin = resolve_gemini_bin(config.cli_path)?;

    {
        let mut log = on_log.lock().unwrap_or_else(|e| {
            log::error!("Mutex poisoned while acquiring on_log for pre-run: {:?}", e);
            e.into_inner()
        });
        run_hook_script(
            "pre-run",
            config.pre_run_script,
            project_root,
            &mut *log,
            true,
        )?;
    }

    let run_start = Instant::now();
    let mut child = {
        let mut log = on_log.lock().unwrap_or_else(|e| {
            log::error!(
                "Mutex poisoned while acquiring on_log for command build: {:?}",
                e
            );
            e.into_inner()
        });
        build_gemini_command(&gemini_bin, prompt, project_root, config, &mut *log)?
            .spawn()
            .map_err(GeminiError::SpawnFailed)?
    };

    let stderr_handle = child.stderr.take().expect("stderr must be piped");
    let stdout_handle = child.stdout.take().expect("stdout must be piped");

    let child = Arc::new(Mutex::new(child));
    let done = Arc::new(AtomicBool::new(false));
    let watchdog = ai_process::spawn_cancel_watchdog(&child, &cancel, &done);

    let on_log_for_stderr = Arc::clone(&on_log);
    let stderr_thread = std::thread::spawn(move || {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(stderr_handle);
        let mut full_stderr = String::new();
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    full_stderr.push_str(&line);
                    full_stderr.push('\n');
                    if is_stderr_noise(&line) {
                        continue;
                    }
                    let display = simplify_stderr_line(&line);
                    let mut log = on_log_for_stderr.lock().unwrap_or_else(|e| e.into_inner());
                    log(&display);
                    log("\n");
                }
                Err(e) => {
                    let msg = format!("stderr read error: {e}\n");
                    let mut log = on_log_for_stderr.lock().unwrap_or_else(|e| e.into_inner());
                    log(&msg);
                    full_stderr.push_str(&msg);
                    break;
                }
            }
        }
        full_stderr
    });

    let stream_result = process_event_stream(stdout_handle, &cancel, &mut |text: &str| {
        let mut log = on_log.lock().unwrap_or_else(|e| e.into_inner());
        log(text);
    });

    done.store(true, Ordering::Relaxed);
    let _ = watchdog.join();
    let exit_status = wait_for_child(&child);

    let (final_result, edited_files, stream_metrics) =
        stream_result.map_err(GeminiError::StreamReadError)?;
    let stderr = stderr_thread.join().unwrap_or_default();

    if cancel.load(Ordering::Relaxed) {
        return Err(GeminiError::Cancelled);
    }

    check_exit_status(exit_status, &stderr, &on_log)?;

    if final_result.is_empty() && !stderr.is_empty() {
        let mut log = on_log.lock().unwrap_or_else(|e| {
            log::error!(
                "Mutex poisoned while acquiring on_log for empty-result error: {:?}",
                e
            );
            e.into_inner()
        });
        log(&format!("\nError: {}\n", stderr));
    }

    {
        let mut log = on_log.lock().unwrap_or_else(|e| {
            log::error!(
                "Mutex poisoned while acquiring on_log for post-run: {:?}",
                e
            );
            e.into_inner()
        });
        run_hook_script(
            "post-run",
            config.post_run_script,
            project_root,
            &mut *log,
            false,
        )?;
    }

    let elapsed_ms = run_start.elapsed().as_millis() as u64;

    Ok(GeminiResponse {
        stdout: final_result,
        edited_files,
        cost_usd: (stream_metrics.cost_usd > 0.0).then_some(stream_metrics.cost_usd),
        duration_ms: Some(elapsed_ms),
        num_turns: (stream_metrics.num_turns > 0).then_some(stream_metrics.num_turns),
    })
}

pub(crate) fn parse_diff_from_response(response: &str) -> Option<String> {
    claude::parse_diff_from_response(response)
}
