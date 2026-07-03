use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::ai_process;
use crate::claude;

#[derive(Debug)]
pub(crate) enum OpenCodeError {
    NotFound,
    SpawnFailed(std::io::Error),
    StreamReadError(std::io::Error),
    WaitFailed(std::io::Error),
    Cancelled,
    NonZeroExit(std::process::ExitStatus),
    InvalidExtraArgs(String),
}

impl std::error::Error for OpenCodeError {}

impl std::fmt::Display for OpenCodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenCodeError::NotFound => write!(f, "opencode CLI not found on PATH"),
            OpenCodeError::SpawnFailed(e) => write!(f, "failed to spawn opencode: {e}"),
            OpenCodeError::StreamReadError(e) => {
                write!(f, "failed to read opencode stdout: {e}")
            }
            OpenCodeError::WaitFailed(e) => write!(f, "failed to wait for opencode: {e}"),
            OpenCodeError::Cancelled => write!(f, "cancelled"),
            OpenCodeError::InvalidExtraArgs(args) => {
                write!(f, "failed to parse extra_args (unmatched quote?): {args}")
            }
            OpenCodeError::NonZeroExit(status) => {
                write!(f, "opencode exited with {status}")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct OpenCodeResponse {
    pub stdout: String,
    pub edited_files: Vec<String>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    pub num_turns: Option<u64>,
}

/// Accumulated metrics from the OpenCode JSON event stream.
#[derive(Default)]
struct StreamMetrics {
    cost_usd: f64,
    num_turns: u64,
}

pub(crate) fn get_available_models(cli_path: &str) -> Vec<String> {
    let opencode_bin = if cli_path.is_empty() {
        "opencode"
    } else {
        cli_path
    };

    let output = Command::new(opencode_bin).arg("models").output();

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Extract text content from a text event, trying multiple field paths
/// to handle different OpenCode stream format versions.
fn extract_text_from_event(event: &serde_json::Value) -> Option<&str> {
    // Primary: event.part.text
    event
        .get("part")
        .and_then(|p| p.get("text"))
        .and_then(|t| t.as_str())
        // Fallback: event.text (flat format)
        .or_else(|| event.get("text").and_then(|t| t.as_str()))
        // Fallback: event.content (alternative format)
        .or_else(|| event.get("content").and_then(|t| t.as_str()))
}

/// Process a "text" event from the OpenCode JSON stream.
fn process_text_event(event: &serde_json::Value, on_log: &mut impl FnMut(&str)) {
    if let Some(text) = extract_text_from_event(event) {
        on_log(text);
        on_log("\n");
    } else {
        on_log(&format!(
            "[DEBUG] text event but no text found: {:?}\n",
            event
        ));
    }
}

/// Process a "tool_use" or "tool" event from the OpenCode JSON stream.
/// Returns any newly discovered file path to add to edited_files.
fn process_tool_event(event: &serde_json::Value, on_log: &mut impl FnMut(&str)) -> Option<String> {
    let part = event.get("part");
    let name = part
        .and_then(|p| p.get("tool").or_else(|| p.get("name")))
        .and_then(|n| n.as_str())
        .unwrap_or("?");
    let input = part
        .and_then(|p| p.get("input"))
        .cloned()
        .unwrap_or_default();

    let mut new_edited_file = None;
    if is_file_tool(name) {
        new_edited_file = ai_process::extract_file_path_from_input(&input);
        if new_edited_file.is_none() {
            // Log bash commands when no file path is found
            if let Some(command) = input.get("command").and_then(|c| c.as_str()) {
                if name.eq_ignore_ascii_case("bash") {
                    on_log(&format!(
                        "\u{2192} {} {}\n",
                        name,
                        command.lines().next().unwrap_or("")
                    ));
                }
            }
        }
    }

    let detail = ai_process::build_tool_detail(&input, true);
    if !detail.is_empty() {
        on_log(&format!("\u{2192} {}{}\n", name, detail));
    }

    new_edited_file
}

/// Check whether a tool name refers to a file-modifying tool.
fn is_file_tool(name: &str) -> bool {
    ai_process::is_file_related_tool(name)
}

/// Process a "step_finish" event from the OpenCode JSON stream.
/// Returns the final result text if the step finished with reason "stop".
fn process_step_finish_event(
    event: &serde_json::Value,
    on_log: &mut impl FnMut(&str),
) -> Option<String> {
    let part = event.get("part");
    let reason = part
        .and_then(|p| p.get("reason"))
        .and_then(|r| r.as_str())
        .unwrap_or("");
    if reason != "stop" {
        return None;
    }

    let cost = part
        .and_then(|p| p.get("cost"))
        .and_then(|c| c.as_f64())
        .unwrap_or(0.0);
    let tokens = part.and_then(|p| p.get("tokens"));
    let duration = tokens
        .and_then(|t| t.get("total"))
        .and_then(|t| t.as_u64())
        .unwrap_or(0);

    let final_text = part
        .and_then(|p| p.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();

    on_log(&format!(
        "\n\u{2713} Done ({:.1}s, ${:.4})\n",
        duration as f64 / 1_000_000.0,
        cost
    ));

    Some(final_text)
}

/// Process an "error" event from the OpenCode JSON stream.
fn process_error_event(event: &serde_json::Value, on_log: &mut impl FnMut(&str)) {
    if let Some(error_msg) = event.get("error").and_then(|e| e.get("message")) {
        on_log(&format!("\nError: {}\n", error_msg));
    }
}

/// Configuration for the OpenCode CLI invocation, bundling string parameters.
pub(crate) struct OpenCodeRunConfig<'a> {
    pub model: &'a str,
    pub cli_path: &'a str,
    pub extra_args: &'a str,
    pub env_vars: &'a str,
    pub pre_run_script: &'a str,
    pub post_run_script: &'a str,
}

/// Resolve the opencode binary to an absolute, currently-existing executable.
///
/// A configured `cli_path` is tried first; if it is stale we fall back to
/// resolving `opencode` from PATH (including the login-shell PATH that macOS
/// `.app` bundles need) so the run still works. Only when nothing resolves do
/// we return [`OpenCodeError::NotFound`].
fn resolve_opencode_bin(cli_path: &str) -> Result<PathBuf, OpenCodeError> {
    if !cli_path.is_empty() {
        if let Some(path) = crate::settings::resolve_in_path(cli_path) {
            return Ok(PathBuf::from(path));
        }
    }
    crate::settings::resolve_in_path("opencode")
        .map(PathBuf::from)
        .ok_or(OpenCodeError::NotFound)
}

/// Build the opencode Command with arguments and environment variables.
fn build_opencode_command(
    opencode_bin: &Path,
    prompt: &str,
    project_root: &Path,
    config: &OpenCodeRunConfig<'_>,
    on_log: &mut dyn FnMut(&str),
) -> Result<Command, OpenCodeError> {
    use std::process::Stdio;

    let mut cmd = Command::new(opencode_bin);
    cmd.arg("run")
        .arg(prompt)
        .arg("--format")
        .arg("json")
        .arg("--print-logs");
    if !config.model.is_empty() {
        cmd.arg("--model").arg(config.model);
    }
    if !config.extra_args.trim().is_empty() {
        let args = shlex::split(config.extra_args)
            .ok_or_else(|| OpenCodeError::InvalidExtraArgs(config.extra_args.to_string()))?;
        for arg in args {
            cmd.arg(arg);
        }
    }
    claude::apply_env_vars(&mut cmd, config.env_vars, on_log);
    // Inject .Dirigent/.env overrides so AI runs use dev credentials.
    crate::claude::apply_dirigent_env(&mut cmd, project_root, on_log);
    cmd.current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(cmd)
}

/// Process the JSON event stream from stdout, collecting results, edited files, and metrics.
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
    let mut accumulated_text = String::new();

    for line_result in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let line = line_result?;

        let Some(json_str) = ai_process::stream_json_payload(&line) else {
            continue;
        };

        let Ok(event) = serde_json::from_str::<serde_json::Value>(json_str) else {
            on_log(&line);
            on_log("\n");
            continue;
        };
        dispatch_event(
            &event,
            on_log,
            &mut final_result,
            &mut edited_files,
            &mut metrics,
            &mut accumulated_text,
        );
    }

    // Use accumulated text output as the response when step_finish didn't
    // provide a final result — matches Claude's behaviour where the "result"
    // event carries the full response text.
    if final_result.is_empty() && !accumulated_text.is_empty() {
        final_result = accumulated_text;
    }

    Ok((final_result, edited_files, metrics))
}

/// Dispatch a single parsed JSON event to the appropriate handler.
fn dispatch_event(
    event: &serde_json::Value,
    on_log: &mut impl FnMut(&str),
    final_result: &mut String,
    edited_files: &mut Vec<String>,
    metrics: &mut StreamMetrics,
    accumulated_text: &mut String,
) {
    let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match event_type {
        "text" => {
            process_text_event(event, on_log);
            // Accumulate text for the response (like Claude's "result" event).
            if let Some(text) = extract_text_from_event(event) {
                accumulated_text.push_str(text);
                accumulated_text.push('\n');
            }
        }
        "tool_use" | "tool" => {
            if let Some(path) =
                process_tool_event(event, on_log).filter(|p| !edited_files.contains(p))
            {
                edited_files.push(path);
            }
        }
        "step_finish" => {
            // Accumulate metrics from every step_finish event.
            if let Some(part) = event.get("part") {
                if let Some(cost) = part.get("cost").and_then(|c| c.as_f64()) {
                    metrics.cost_usd += cost;
                }
                metrics.num_turns += 1;
            }
            if let Some(text) = process_step_finish_event(event, on_log) {
                if !text.is_empty() {
                    *final_result = text;
                }
            }
        }
        "error" => process_error_event(event, on_log),
        _ => {}
    }
}

/// Check the child process exit status and log stderr on failure.
fn check_exit_status<F: FnMut(&str)>(
    exit_status: std::process::ExitStatus,
    stderr: &str,
    on_log: &Arc<Mutex<F>>,
) -> Result<(), OpenCodeError> {
    if exit_status.success() {
        return Ok(());
    }
    let status = exit_status;
    if !stderr.is_empty() {
        ai_process::with_locked_log(on_log, "non-zero exit error", |log| {
            log(&format!("\nError: {}\n", stderr));
        });
    }
    Err(OpenCodeError::NonZeroExit(status))
}

pub(crate) fn invoke_opencode_streaming(
    prompt: &str,
    project_root: &Path,
    config: &OpenCodeRunConfig<'_>,
    on_log: impl FnMut(&str) + Send + 'static,
    cancel: Arc<AtomicBool>,
) -> Result<OpenCodeResponse, OpenCodeError> {
    // Wrap on_log in Arc<Mutex<>> so both the stderr streaming thread and the
    // main stdout processing can log concurrently.  stderr from OpenCode is
    // line-buffered (even when stdout is pipe-buffered), so streaming it gives
    // the user real-time visibility while the run is in progress.
    let on_log = Arc::new(Mutex::new(on_log));

    let opencode_bin = resolve_opencode_bin(config.cli_path)?;

    ai_process::run_hook_with_locked_log(
        &on_log,
        "pre-run",
        config.pre_run_script,
        project_root,
        true,
    )
    .map_err(OpenCodeError::SpawnFailed)?;

    let run_start = Instant::now();
    let mut child = ai_process::with_locked_log(&on_log, "command build", |log| {
        build_opencode_command(&opencode_bin, prompt, project_root, config, &mut *log)?
            .spawn()
            .map_err(OpenCodeError::SpawnFailed)
    })?;

    let stderr_handle = child.stderr.take().expect("stderr must be piped");
    let stdout_handle = child.stdout.take().expect("stdout must be piped");

    let child = Arc::new(Mutex::new(child));
    let done = Arc::new(AtomicBool::new(false));
    let watchdog = ai_process::spawn_cancel_watchdog(&child, &cancel, &done);

    // Stream stderr line-by-line so the user sees OpenCode progress in real
    // time (stderr is unbuffered / line-buffered even when stdout is piped).
    // We filter through `filter_opencode_log_line` to suppress verbose
    // INFO/DEBUG noise while preserving WARN/ERROR and useful metadata.
    let stderr_thread =
        ai_process::spawn_stderr_collector(stderr_handle, Arc::clone(&on_log), |line, log| {
            claude::filter_opencode_log_line(line, log);
        });

    let stream_result = process_event_stream(stdout_handle, &cancel, &mut |text: &str| {
        ai_process::with_locked_log(&on_log, "stdout", |log| log(text));
    });

    done.store(true, Ordering::Relaxed);

    // If the stdout stream errored, the child may still be running. Kill it
    // first so the subsequent `wait` cannot block indefinitely, then drain the
    // helper threads before surfacing the read error.
    let (final_result, edited_files, stream_metrics) = match stream_result {
        Ok(result) => result,
        Err(error) => {
            ai_process::kill_child(&child);
            let _ = watchdog.join();
            let _ = ai_process::wait_for_child(&child);
            let _ = stderr_thread.join();
            return Err(OpenCodeError::StreamReadError(error));
        }
    };

    let _ = watchdog.join();
    let exit_status = ai_process::wait_for_child(&child);
    let stderr = stderr_thread.join().unwrap_or_default();

    if cancel.load(Ordering::Relaxed) {
        return Err(OpenCodeError::Cancelled);
    }

    let exit_status = exit_status.map_err(OpenCodeError::WaitFailed)?;
    check_exit_status(exit_status, &stderr, &on_log)?;

    ai_process::log_empty_result_stderr(&on_log, &final_result, &stderr);

    ai_process::run_hook_with_locked_log(
        &on_log,
        "post-run",
        config.post_run_script,
        project_root,
        false,
    )
    .map_err(OpenCodeError::SpawnFailed)?;

    let elapsed_ms = run_start.elapsed().as_millis() as u64;

    Ok(OpenCodeResponse {
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
