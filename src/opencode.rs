use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::claude;

const WATCHDOG_POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug)]
pub(crate) enum OpenCodeError {
    NotFound,
    SpawnFailed(std::io::Error),
    StreamReadError(std::io::Error),
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

/// Run a hook script (pre-run or post-run) and log its output.
/// Returns `Err` only for pre-run failures (when `fail_on_error` is true).
fn run_hook_script(
    label: &str,
    script: &str,
    project_root: &Path,
    on_log: &mut impl FnMut(&str),
    fail_on_error: bool,
) -> Result<(), OpenCodeError> {
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
                    return Err(OpenCodeError::SpawnFailed(std::io::Error::other(msg)));
                }
            }
        }
        Err(e) => {
            on_log(&format!("\u{2717} {} script error: {}\n", label, e));
            if fail_on_error {
                return Err(OpenCodeError::SpawnFailed(e));
            }
        }
    }
    Ok(())
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
        new_edited_file = extract_file_path_from_input(&input);
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

    let detail = build_tool_detail(&input);
    if !detail.is_empty() {
        on_log(&format!("\u{2192} {}{}\n", name, detail));
    }

    new_edited_file
}

/// Check whether a tool name refers to a file-modifying tool.
fn is_file_tool(name: &str) -> bool {
    let name_lower = name.to_ascii_lowercase();
    matches!(name, "Write" | "Edit" | "Bash" | "Task")
        || matches!(
            name_lower.as_str(),
            "write"
                | "edit"
                | "bash"
                | "task"
                | "write_file"
                | "edit_file"
                | "create_file"
                | "str_replace_editor"
                | "file_editor"
                | "write_to_file"
                | "apply_diff"
        )
}

/// Extract a file path from tool input using common field names.
fn extract_file_path_from_input(input: &serde_json::Value) -> Option<String> {
    input
        .get("file_path")
        .or_else(|| input.get("path"))
        .or_else(|| input.get("file"))
        .and_then(|p| p.as_str())
        .map(|s| s.to_string())
}

/// Build a human-readable detail string from tool input.
fn build_tool_detail(input: &serde_json::Value) -> String {
    if let Some(file_path) = input.get("file_path").and_then(|p| p.as_str()) {
        format!(" {}", file_path)
    } else if let Some(command) = input.get("command").and_then(|c| c.as_str()) {
        format!(" $ {}", command.lines().next().unwrap_or(""))
    } else if let Some(grep) = input.get("pattern").and_then(|p| p.as_str()) {
        format!(" \"{}\"", grep)
    } else {
        String::new()
    }
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

/// Resolve the opencode binary name and verify it exists on PATH.
fn resolve_opencode_bin(cli_path: &str) -> Result<PathBuf, OpenCodeError> {
    let bin = if cli_path.is_empty() {
        "opencode"
    } else {
        cli_path
    };
    which::which(bin).map_err(|_| OpenCodeError::NotFound)
}

/// Build the opencode Command with arguments and environment variables.
fn build_opencode_command(
    opencode_bin: &Path,
    prompt: &str,
    project_root: &Path,
    config: &OpenCodeRunConfig<'_>,
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
    apply_env_vars(&mut cmd, config.env_vars);
    // Inject .Dirigent/.env overrides so AI runs use dev credentials.
    crate::claude::apply_dirigent_env(&mut cmd, project_root);
    cmd.current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(cmd)
}

/// Resolve environment variable **names** (one per line, # comments allowed)
/// from the current process environment and apply them to the command.
/// Lines containing `=` are treated as bare names (the `=…` suffix is stripped)
/// for backward compatibility with old KEY=VALUE config entries.
fn apply_env_vars(cmd: &mut Command, env_vars: &str) {
    for line in env_vars.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Accept bare names; strip any =value suffix left over from old config.
        let name = match line.split_once('=') {
            Some((key, _)) => key.trim(),
            None => line,
        };
        if name.is_empty() {
            continue;
        }
        match std::env::var(name) {
            Ok(value) => {
                cmd.env(name, value);
            }
            Err(_) => {
                eprintln!(
                    "warning: env var '{}' not found in environment, skipping",
                    name
                );
            }
        }
    }
}

/// Spawn a watchdog thread that kills the child process on cancellation.
fn spawn_watchdog(
    child: &Arc<Mutex<std::process::Child>>,
    cancel: &Arc<AtomicBool>,
    done: &Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    let child = Arc::clone(child);
    let cancel = Arc::clone(cancel);
    let done = Arc::clone(done);
    std::thread::spawn(move || {
        while !done.load(Ordering::Relaxed) {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.lock().map(|mut c| c.kill());
                return;
            }
            std::thread::sleep(WATCHDOG_POLL_INTERVAL);
        }
    })
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

        // The stream format may use SSE (Server-Sent Events) framing.
        // Strip `data:` prefixes and skip non-data SSE lines.
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

/// Wait for the child process to exit (handles poisoned mutex).
/// Returns the exit status, or `None` if waiting failed.
fn wait_for_child(child: &Arc<Mutex<std::process::Child>>) -> Option<std::process::ExitStatus> {
    match child.lock() {
        Ok(mut c) => c.wait().ok(),
        Err(poisoned) => poisoned.into_inner().wait().ok(),
    }
}

/// Check the child process exit status and log stderr on failure.
fn check_exit_status<F: FnMut(&str)>(
    exit_status: Option<std::process::ExitStatus>,
    stderr: &str,
    on_log: &Arc<Mutex<F>>,
) -> Result<(), OpenCodeError> {
    let Some(status) = exit_status.filter(|s| !s.success()) else {
        return Ok(());
    };
    if !stderr.is_empty() {
        let mut log = on_log.lock().unwrap_or_else(|e| {
            eprintln!(
                "Mutex poisoned while acquiring on_log for non-zero exit error: {:?}",
                e
            );
            e.into_inner()
        });
        log(&format!("\nError: {}\n", stderr));
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

    {
        let mut log = on_log.lock().unwrap_or_else(|e| {
            eprintln!("Mutex poisoned while acquiring on_log for pre-run: {:?}", e);
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
    let mut child = build_opencode_command(&opencode_bin, prompt, project_root, config)?
        .spawn()
        .map_err(OpenCodeError::SpawnFailed)?;

    let stderr_handle = child.stderr.take().expect("stderr must be piped");
    let stdout_handle = child.stdout.take().expect("stdout must be piped");

    let child = Arc::new(Mutex::new(child));
    let done = Arc::new(AtomicBool::new(false));
    let watchdog = spawn_watchdog(&child, &cancel, &done);

    // Stream stderr line-by-line so the user sees OpenCode progress in real
    // time (stderr is unbuffered / line-buffered even when stdout is piped).
    // We filter through `filter_opencode_log_line` to suppress verbose
    // INFO/DEBUG noise while preserving WARN/ERROR and useful metadata.
    let on_log_for_stderr = Arc::clone(&on_log);
    let stderr_thread = std::thread::spawn(move || {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(stderr_handle);
        let mut full_stderr = String::new();
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    let mut log = on_log_for_stderr.lock().unwrap_or_else(|e| e.into_inner());
                    claude::filter_opencode_log_line(&line, &mut *log);
                    full_stderr.push_str(&line);
                    full_stderr.push('\n');
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
        stream_result.map_err(OpenCodeError::StreamReadError)?;
    let stderr = stderr_thread.join().unwrap_or_default();

    if cancel.load(Ordering::Relaxed) {
        return Err(OpenCodeError::Cancelled);
    }

    check_exit_status(exit_status, &stderr, &on_log)?;

    if final_result.is_empty() && !stderr.is_empty() {
        let mut log = on_log.lock().unwrap_or_else(|e| {
            eprintln!(
                "Mutex poisoned while acquiring on_log for empty-result error: {:?}",
                e
            );
            e.into_inner()
        });
        log(&format!("\nError: {}\n", stderr));
    }

    {
        let mut log = on_log.lock().unwrap_or_else(|e| {
            eprintln!(
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
