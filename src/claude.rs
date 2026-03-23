use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const WATCHDOG_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Maximum bytes per auto-context section (file snippet or git diff).
/// Keeps the final prompt well under OS `ARG_MAX` limits (~1 MB on macOS).
const AUTO_CONTEXT_MAX_BYTES: usize = 100_000;

#[derive(Debug)]
pub(crate) enum ClaudeError {
    NotFound,
    SpawnFailed(std::io::Error),
    Cancelled,
}

impl std::error::Error for ClaudeError {}

impl std::fmt::Display for ClaudeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClaudeError::NotFound => write!(f, "claude CLI not found on PATH"),
            ClaudeError::SpawnFailed(e) => write!(f, "failed to spawn claude: {e}"),
            ClaudeError::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ClaudeResponse {
    pub stdout: String,
    /// File paths that Claude edited (from Edit/Write tool_use events).
    pub edited_files: Vec<String>,
    /// Run metrics extracted from the stream-json "result" event.
    pub metrics: RunMetrics,
}

/// Cost and performance metrics from a Claude run.
#[derive(Debug, Clone, Default)]
pub(crate) struct RunMetrics {
    pub cost_usd: f64,
    pub duration_ms: u64,
    pub num_turns: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Parse a `[command]` prefix from cue text.
///
/// Returns `Some((command_name, remaining_text))` if the text starts with
/// `[word]`, otherwise `None`. The remaining text is trimmed.
pub(crate) fn parse_command_prefix(text: &str) -> Option<(&str, &str)> {
    let trimmed = text.trim_start();
    if !trimmed.starts_with('[') {
        return None;
    }
    let end = trimmed.find(']')?;
    let name = trimmed[1..end].trim();
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    let rest = trimmed[end + 1..].trim_start();
    Some((name, rest))
}

/// Build a structured prompt for Claude given a cue's context.
///
/// When `project_root` is provided and `file_path` is non-empty, the prompt
/// includes the surrounding file content (±50 lines) and any recent git diff
/// for the file, so Claude has immediate context without extra tool calls.
pub(crate) fn build_prompt(
    cue_text: &str,
    file_path: &str,
    line_number: usize,
    line_number_end: Option<usize>,
    images: &[String],
    _project_root: Option<&Path>,
) -> String {
    build_prompt_with_auto_context(
        cue_text,
        file_path,
        line_number,
        line_number_end,
        images,
        "",
    )
}

/// Build a structured prompt with optional auto-context (file snippet + git diff).
pub(crate) fn build_prompt_with_auto_context(
    cue_text: &str,
    file_path: &str,
    line_number: usize,
    line_number_end: Option<usize>,
    images: &[String],
    auto_context: &str,
) -> String {
    let images_section = if images.is_empty() {
        String::new()
    } else {
        let list: Vec<String> = images.iter().map(|p| format!("- {}", p)).collect();
        format!(
            "\n\n## Attached Images\n\n\
             The following images are attached. Use the Read tool to view them:\n{}",
            list.join("\n"),
        )
    };
    let auto_ctx_section = if auto_context.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", auto_context)
    };
    if file_path.is_empty() {
        format!(
            "## Task\n\n{}{}{}\n\n\
             ## Instructions\n\n\
             Make the requested changes directly by editing the files. \
             Do not output a diff — use your tools to edit files in place.",
            cue_text, images_section, auto_ctx_section,
        )
    } else {
        let line_ref = match line_number_end {
            Some(end) => format!("lines {}-{}", line_number, end),
            None => format!("line {}", line_number),
        };

        format!(
            "## Task\n\n{}{}\n\n\
             ## Context\n\n\
             Focus on {} in `{}`.\n{}\n\n\
             ## Instructions\n\n\
             Make the requested changes directly by editing the files. \
             Do not output a diff — use your tools to edit files in place.",
            cue_text, images_section, line_ref, file_path, auto_ctx_section,
        )
    }
}

/// Build the file-content snippet section for auto-context.
fn gather_file_snippet(
    project_root: &std::path::Path,
    file_path: &str,
    line_number: usize,
    line_number_end: Option<usize>,
) -> Option<String> {
    let full_path = project_root.join(file_path);
    let content = std::fs::read_to_string(&full_path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return None;
    }

    let center = line_number.saturating_sub(1); // 0-indexed
    let end_line = line_number_end.unwrap_or(line_number).saturating_sub(1);
    let span = end_line.saturating_sub(center) + 1;
    // Window: 50 lines total, centered on the target range
    let padding = 50usize.saturating_sub(span) / 2;
    let start = center.saturating_sub(padding);
    let end = (end_line + padding + 1).min(lines.len());

    let snippet: Vec<String> = lines[start..end]
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{:>4} | {}", start + i + 1, line))
        .collect();
    let snippet_text = snippet.join("\n");

    Some(format_file_snippet(file_path, start, end, &snippet_text))
}

/// Format the file snippet section, truncating if needed.
fn format_file_snippet(file_path: &str, start: usize, end: usize, snippet_text: &str) -> String {
    if snippet_text.len() <= AUTO_CONTEXT_MAX_BYTES {
        return format!(
            "## File Content\n\n\
             `{}` (lines {}-{}):\n```\n{}\n```",
            file_path,
            start + 1,
            end,
            snippet_text,
        );
    }
    // Truncate to fit within the byte ceiling
    let truncated: String = snippet_text
        .char_indices()
        .take_while(|&(i, _)| i < AUTO_CONTEXT_MAX_BYTES)
        .map(|(_, c)| c)
        .collect();
    format!(
        "## File Content\n\n\
         `{}` (lines {}-{}, truncated):\n```\n{}\n... (truncated)\n```",
        file_path,
        start + 1,
        end,
        truncated,
    )
}

/// Truncate text to fit within `AUTO_CONTEXT_MAX_BYTES`, appending a suffix if truncated.
fn truncate_to_byte_limit(text: &mut String) {
    if text.len() <= AUTO_CONTEXT_MAX_BYTES {
        return;
    }
    *text = text
        .char_indices()
        .take_while(|&(i, _)| i < AUTO_CONTEXT_MAX_BYTES)
        .map(|(_, c)| c)
        .collect();
    text.push_str("\n... (truncated)");
}

/// Build the git-diff section for auto-context.
fn gather_git_diff_section(
    project_root: &std::path::Path,
    file_path: &str,
) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["diff", "--", file_path])
        .current_dir(project_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let diff = String::from_utf8_lossy(&output.stdout);
    let diff = diff.trim();
    if diff.is_empty() {
        return None;
    }

    // Limit diff to ~200 lines to avoid bloating the prompt
    let diff_lines: Vec<&str> = diff.lines().collect();
    let mut truncated = if diff_lines.len() > 200 {
        format!(
            "{}\n... ({} more lines)",
            diff_lines[..200].join("\n"),
            diff_lines.len() - 200
        )
    } else {
        diff.to_string()
    };
    // Enforce byte ceiling on top of line-count limit
    truncate_to_byte_limit(&mut truncated);

    Some(format!(
        "## Recent Changes (uncommitted)\n\n\
         ```diff\n{}\n```",
        truncated,
    ))
}

/// Generate auto-context for a file-specific cue: a snippet of the file around
/// the target line(s), and the git diff for the file (recent uncommitted changes).
///
/// Returns a formatted string to include in the prompt, or empty if no context
/// could be gathered (e.g. file doesn't exist or is a global cue).
pub(crate) fn gather_auto_context(
    project_root: &std::path::Path,
    file_path: &str,
    line_number: usize,
    line_number_end: Option<usize>,
    include_file: bool,
    include_git_diff: bool,
) -> String {
    if file_path.is_empty() {
        return String::new();
    }

    let mut sections = Vec::new();

    if include_file {
        if let Some(snippet) = gather_file_snippet(project_root, file_path, line_number, line_number_end) {
            sections.push(snippet);
        }
    }

    if include_git_diff {
        if let Some(diff_section) = gather_git_diff_section(project_root, file_path) {
            sections.push(diff_section);
        }
    }

    sections.join("\n\n")
}

/// Build a follow-up prompt for replying to a Review cue with feedback.
/// Includes the original task, the previous diff, and the user's reply.
pub(crate) fn build_reply_prompt(
    original_cue: &str,
    file_path: &str,
    line_number: usize,
    line_number_end: Option<usize>,
    previous_diff: &str,
    reply: &str,
    images: &[String],
    _project_root: Option<&Path>,
) -> String {
    let context = if file_path.is_empty() {
        String::new()
    } else {
        let line_ref = match line_number_end {
            Some(end) => format!("lines {}-{}", line_number, end),
            None => format!("line {}", line_number),
        };
        format!(
            "## Context\n\n\
             Focus on {} in `{}`.\n\n",
            line_ref, file_path,
        )
    };
    let images_section = if images.is_empty() {
        String::new()
    } else {
        let list: Vec<String> = images.iter().map(|p| format!("- {}", p)).collect();
        format!(
            "\n\n## Attached Images\n\n\
             The following images are attached. Use the Read tool to view them:\n{}",
            list.join("\n"),
        )
    };
    format!(
        "## Original Task\n\n{}{}\n\n\
         {}\
         ## Previous Changes\n\n\
         You already made the following changes (currently applied in the working tree):\n\n\
         ```diff\n{}\n```\n\n\
         ## Feedback\n\n{}\n\n\
         ## Instructions\n\n\
         Adjust the code based on the feedback above. The previous changes are already applied — \
         build on them rather than starting over. \
         Make the requested changes directly by editing the files. \
         Do not output a diff — use your tools to edit files in place.",
        original_cue, images_section, context, previous_diff, reply,
    )
}

// ---------------------------------------------------------------------------
// Helper functions extracted from invoke_claude_streaming to reduce cognitive
// complexity (SonarQube S3776).
// ---------------------------------------------------------------------------

/// Resolve the Claude binary path and verify it exists on PATH.
fn resolve_claude_binary(cli_path: &str) -> Result<&str, ClaudeError> {
    let claude_bin = if cli_path.is_empty() { "claude" } else { cli_path };
    let which_result = Command::new("which").arg(claude_bin).output();
    match which_result {
        Ok(output) if !output.status.success() => Err(ClaudeError::NotFound),
        Err(_) => Err(ClaudeError::NotFound),
        _ => Ok(claude_bin),
    }
}

/// Build the `Command` with prompt, flags, extra args, and env vars.
fn build_claude_command(
    claude_bin: &str,
    prompt: &str,
    model: &str,
    extra_args: &str,
    env_vars: &str,
) -> Command {
    let mut cmd = Command::new(claude_bin);
    cmd.arg("-p")
        .arg(prompt)
        .arg("--verbose")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--dangerously-skip-permissions");
    if !model.is_empty() {
        cmd.arg("--model").arg(model);
    }
    append_extra_args(&mut cmd, extra_args);
    apply_env_vars(&mut cmd, env_vars);
    cmd
}

/// Append whitespace-separated extra arguments to the command.
fn append_extra_args(cmd: &mut Command, extra_args: &str) {
    for arg in extra_args.split_whitespace() {
        if !arg.is_empty() {
            cmd.arg(arg);
        }
    }
}

/// Apply KEY=VALUE environment variables (one per line, # comments allowed).
fn apply_env_vars(cmd: &mut Command, env_vars: &str) {
    for line in env_vars.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            if !key.is_empty() {
                cmd.env(key, value);
            }
        }
    }
}

/// Run a lifecycle script (pre-run or post-run).
///
/// Returns `Err` for pre-run failures (abort the run), logs but ignores
/// post-run failures when `fail_on_error` is false.
fn run_lifecycle_script(
    script: &str,
    label: &str,
    project_root: &Path,
    on_log: &mut dyn FnMut(&str),
    fail_on_error: bool,
) -> Result<(), ClaudeError> {
    let trimmed = script.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    on_log(&format!("\u{25B6} {}: {}\n", label, trimmed));
    match Command::new("sh").arg("-c").arg(trimmed).current_dir(project_root).output() {
        Ok(output) => handle_script_output(&output, label, on_log, fail_on_error),
        Err(e) => handle_script_error(e, label, on_log, fail_on_error),
    }
}

/// Process successful script execution output.
fn handle_script_output(
    output: &std::process::Output,
    label: &str,
    on_log: &mut dyn FnMut(&str),
    fail_on_error: bool,
) -> Result<(), ClaudeError> {
    if !output.stdout.is_empty() {
        on_log(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        on_log(&String::from_utf8_lossy(&output.stderr));
    }
    if output.status.success() {
        return Ok(());
    }
    let msg = format!("{} script failed (exit {})", label, output.status);
    on_log(&format!("\u{2717} {}\n", msg));
    if fail_on_error {
        return Err(ClaudeError::SpawnFailed(std::io::Error::new(
            std::io::ErrorKind::Other,
            msg,
        )));
    }
    Ok(())
}

/// Handle a script spawn error.
fn handle_script_error(
    e: std::io::Error,
    label: &str,
    on_log: &mut dyn FnMut(&str),
    fail_on_error: bool,
) -> Result<(), ClaudeError> {
    on_log(&format!("\u{2717} {} script error: {}\n", label, e));
    if fail_on_error {
        return Err(ClaudeError::SpawnFailed(e));
    }
    Ok(())
}

/// Spawn a watchdog thread that kills the child process when `cancel` is set.
/// Returns `(done_flag, join_handle)`.
fn spawn_watchdog(
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
fn spawn_stderr_reader(stderr_handle: std::process::ChildStderr) -> std::thread::JoinHandle<String> {
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
struct StreamState {
    final_result: String,
    edited_files: Vec<String>,
    metrics: RunMetrics,
}

/// Read stream-json events from stdout, dispatching each to the appropriate handler.
fn read_stream_events(
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
                on_log(&line);
                on_log("\n");
                continue;
            }
        };
        dispatch_stream_event(&event, &mut state, on_log);
    }
    state
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
        cost_usd: event.get("cost_usd").and_then(|c| c.as_f64()).unwrap_or(0.0),
        duration_ms: event.get("duration_ms").and_then(|d| d.as_u64()).unwrap_or(0),
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
        on_log(&format!("\u{23f3} Rate limited, retrying in {:.0}s\n", seconds));
    }
}

/// Reap the child process (works whether it exited naturally or was killed).
fn reap_child(child: &Arc<Mutex<std::process::Child>>) {
    match child.lock() {
        Ok(mut c) => { let _ = c.wait(); }
        Err(poisoned) => { let _ = poisoned.into_inner().wait(); }
    }
}

/// Invoke `claude -p <prompt> --output-format stream-json` with live progress
/// streaming to a shared log buffer. Parses JSON events from stdout in real-time.
///
/// The `cancel` token allows the caller to abort the run: a watchdog thread
/// monitors the flag and kills the child process when it is set.
pub(crate) fn invoke_claude_streaming(
    prompt: &str,
    project_root: &Path,
    model: &str,
    cli_path: &str,
    extra_args: &str,
    env_vars: &str,
    pre_run_script: &str,
    post_run_script: &str,
    mut on_log: impl FnMut(&str),
    cancel: Arc<AtomicBool>,
) -> Result<ClaudeResponse, ClaudeError> {
    use std::process::Stdio;

    let claude_bin = resolve_claude_binary(cli_path)?;
    let mut cmd = build_claude_command(claude_bin, prompt, model, extra_args, env_vars);

    run_lifecycle_script(pre_run_script, "pre-run", project_root, &mut on_log, true)?;

    let mut child = cmd
        .current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(ClaudeError::SpawnFailed)?;

    let stderr_handle = child.stderr.take().expect("stderr must be piped");
    let stdout_handle = child.stdout.take().expect("stdout must be piped");
    let child = Arc::new(Mutex::new(child));

    let (done, watchdog) = spawn_watchdog(&child, &cancel);
    let stderr_thread = spawn_stderr_reader(stderr_handle);
    let state = read_stream_events(stdout_handle, &cancel, &mut on_log);

    done.store(true, Ordering::Relaxed);
    let _ = watchdog.join();
    reap_child(&child);
    let stderr = stderr_thread.join().unwrap_or_default();

    if cancel.load(Ordering::Relaxed) {
        return Err(ClaudeError::Cancelled);
    }

    if state.final_result.is_empty() && !stderr.is_empty() {
        on_log(&format!("\nError: {}\n", stderr));
    }

    run_lifecycle_script(post_run_script, "post-run", project_root, &mut on_log, false)?;

    Ok(ClaudeResponse {
        stdout: state.final_result,
        edited_files: state.edited_files,
        metrics: state.metrics,
    })
}

/// Extract the user-facing text from a structured prompt.
///
/// For an initial prompt, returns the text between "## Task" and the next section.
/// For a reply prompt, returns the text from "## Feedback".
/// Falls back to the full prompt if no structure is found.
pub(crate) fn extract_user_text_from_prompt(prompt: &str) -> String {
    // Reply prompt: extract feedback section
    if let Some(pos) = prompt.find("## Feedback\n\n") {
        let start = pos + "## Feedback\n\n".len();
        let rest = &prompt[start..];
        let end = rest.find("\n\n## ").unwrap_or(rest.len());
        return rest[..end].trim().to_string();
    }
    // Initial prompt: extract task section
    if let Some(pos) = prompt.find("## Task\n\n") {
        let start = pos + "## Task\n\n".len();
        let rest = &prompt[start..];
        let end = rest.find("\n\n## ").unwrap_or(rest.len());
        return rest[..end].trim().to_string();
    }
    prompt.to_string()
}

/// Parse diff content from a Claude response.
pub(crate) fn parse_diff_from_response(response: &str) -> Option<String> {
    if let Some(diff) = extract_fenced_diff(response) {
        return Some(diff);
    }
    extract_unified_diff(response)
}

/// Extract fenced diff code blocks from a response string.
fn collect_fenced_blocks(response: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current_lines: Vec<&str> = Vec::new();
    let mut in_block = false;

    for line in response.lines() {
        let trimmed = line.trim_start();
        if !in_block && trimmed.starts_with("```diff") {
            in_block = true;
            current_lines.clear();
            continue;
        }
        if in_block && trimmed.starts_with("```") {
            if !current_lines.is_empty() {
                blocks.push(current_lines.join("\n"));
            }
            in_block = false;
            current_lines.clear();
            continue;
        }
        if in_block {
            current_lines.push(line);
        }
    }

    blocks
}

/// Ensure text ends with a newline.
fn ensure_trailing_newline(text: &mut String) {
    if !text.ends_with('\n') {
        text.push('\n');
    }
}

fn extract_fenced_diff(response: &str) -> Option<String> {
    let blocks = collect_fenced_blocks(response);
    if blocks.is_empty() {
        return None;
    }

    let diffs: Vec<String> = blocks
        .iter()
        .filter_map(|block| extract_unified_diff(block))
        .collect();

    if diffs.is_empty() {
        return None;
    }

    let mut result = diffs.join("\n");
    ensure_trailing_newline(&mut result);
    Some(result)
}

/// Check whether a line belongs to a unified diff hunk body.
fn is_diff_body_line(line: &str) -> bool {
    line.starts_with("@@ ")
        || line.starts_with('+')
        || line.starts_with('-')
        || line.starts_with(' ')
}

/// Check whether the line at `i` starts a new file header (`--- ` / `+++ ` pair).
fn is_file_header_start(lines: &[&str], i: usize) -> bool {
    lines[i].starts_with("--- ") && i + 1 < lines.len() && lines[i + 1].starts_with("+++ ")
}

/// Collect contiguous diff hunk lines starting at `i`, returning the new index.
fn collect_hunk_lines<'a>(lines: &[&'a str], mut i: usize, result: &mut Vec<&'a str>) -> usize {
    while i < lines.len() {
        if is_diff_body_line(lines[i]) {
            result.push(lines[i]);
            i += 1;
            continue;
        }
        // Stop at next file header or non-diff line
        break;
    }
    i
}

fn extract_unified_diff(response: &str) -> Option<String> {
    let lines: Vec<&str> = response.lines().collect();
    let mut result: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if !is_file_header_start(&lines, i) {
            i += 1;
            continue;
        }
        // Push the --- and +++ header lines
        result.push(lines[i]);
        result.push(lines[i + 1]);
        i += 2;
        i = collect_hunk_lines(&lines, i, &mut result);
    }

    if result.is_empty() {
        return None;
    }

    let mut text = result.join("\n");
    ensure_trailing_newline(&mut text);
    Some(fix_hunk_headers(&text))
}

/// Count old and new lines in a hunk body starting at index `start`.
fn count_hunk_lines(lines: &[&str], start: usize) -> (usize, usize) {
    let mut old_count = 0usize;
    let mut new_count = 0usize;
    let mut j = start;
    while j < lines.len() {
        let line = lines[j];
        let is_next_hunk = line.starts_with("@@ ") || is_file_header_start(lines, j);
        if is_next_hunk {
            break;
        }
        if line.starts_with('+') {
            new_count += 1;
        } else if line.starts_with('-') {
            old_count += 1;
        } else if line.starts_with(' ') {
            old_count += 1;
            new_count += 1;
        } else {
            break;
        }
        j += 1;
    }
    (old_count, new_count)
}

fn fix_hunk_headers(diff_text: &str) -> String {
    let lines: Vec<&str> = diff_text.lines().collect();
    let mut result = Vec::new();

    for (i, &line) in lines.iter().enumerate() {
        if !line.starts_with("@@ ") {
            result.push(line.to_string());
            continue;
        }
        let (old_start, new_start, tail) = parse_hunk_header(line);
        let (old_count, new_count) = count_hunk_lines(&lines, i + 1);
        result.push(format!(
            "@@ -{},{} +{},{} @@{}",
            old_start, old_count, new_start, new_count, tail
        ));
    }

    let mut text = result.join("\n");
    ensure_trailing_newline(&mut text);
    text
}

fn parse_hunk_header(header: &str) -> (usize, usize, &str) {
    let inner = header.strip_prefix("@@ ").unwrap_or(header);

    let (range_part, tail) = if let Some(pos) = inner.find(" @@") {
        let after = &inner[pos + 3..];
        (&inner[..pos], after)
    } else {
        (inner, "")
    };

    let parts: Vec<&str> = range_part.split_whitespace().collect();
    let old_start = parts
        .first()
        .and_then(|p| p.strip_prefix('-'))
        .and_then(|p| p.split(',').next())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1);
    let new_start = parts
        .get(1)
        .and_then(|p| p.strip_prefix('+'))
        .and_then(|p| p.split(',').next())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1);

    (old_start, new_start, tail)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- build_prompt --

    #[test]
    fn build_prompt_global_cue() {
        let prompt = build_prompt("Add tests", "", 0, None, &[], None);
        assert!(prompt.contains("Add tests"));
        assert!(!prompt.contains("Focus on"));
    }

    #[test]
    fn build_prompt_with_file_single_line() {
        let prompt = build_prompt("Fix bug", "src/main.rs", 42, None, &[], None);
        assert!(prompt.contains("Fix bug"));
        assert!(prompt.contains("line 42"));
        assert!(prompt.contains("`src/main.rs`"));
    }

    #[test]
    fn build_prompt_with_file_line_range() {
        let prompt = build_prompt("Refactor", "lib.rs", 10, Some(20), &[], None);
        assert!(prompt.contains("lines 10-20"));
        assert!(prompt.contains("`lib.rs`"));
    }

    #[test]
    fn build_prompt_with_images() {
        let images = vec![
            "/tmp/screenshot.png".to_string(),
            "/tmp/design.jpg".to_string(),
        ];
        let prompt = build_prompt("Implement this design", "", 0, None, &images, None);
        assert!(prompt.contains("Attached Images"));
        assert!(prompt.contains("/tmp/screenshot.png"));
        assert!(prompt.contains("/tmp/design.jpg"));
    }

    // -- parse_diff_from_response --

    #[test]
    fn parse_fenced_diff() {
        let response = "\
Here's the fix:

```diff
--- a/foo.rs
+++ b/foo.rs
@@ -1,3 +1,3 @@
 line1
-old
+new
```

Done!";
        let diff = parse_diff_from_response(response).unwrap();
        assert!(diff.contains("--- a/foo.rs"));
        assert!(diff.contains("+++ b/foo.rs"));
        assert!(diff.contains("-old"));
        assert!(diff.contains("+new"));
    }

    #[test]
    fn parse_inline_unified_diff() {
        let response = "\
--- a/foo.rs
+++ b/foo.rs
@@ -1,2 +1,2 @@
 keep
-remove
+add
";
        let diff = parse_diff_from_response(response).unwrap();
        assert!(diff.contains("-remove"));
        assert!(diff.contains("+add"));
    }

    #[test]
    fn parse_no_diff_returns_none() {
        assert!(parse_diff_from_response("Just some text, no diff here.").is_none());
    }

    #[test]
    fn parse_multiple_fenced_diffs() {
        let response = "\
```diff
--- a/one.rs
+++ b/one.rs
@@ -1,1 +1,1 @@
-a
+b
```

```diff
--- a/two.rs
+++ b/two.rs
@@ -1,1 +1,1 @@
-c
+d
```";
        let diff = parse_diff_from_response(response).unwrap();
        assert!(diff.contains("--- a/one.rs"));
        assert!(diff.contains("--- a/two.rs"));
    }

    // -- fix_hunk_headers --

    #[test]
    fn fix_hunk_headers_corrects_counts() {
        let input = "\
--- a/f.rs
+++ b/f.rs
@@ -1,999 +1,999 @@
 context
-old1
-old2
+new1
";
        let result = fix_hunk_headers(input);
        // 1 context + 2 old = 3 old lines, 1 context + 1 new = 2 new lines
        assert!(result.contains("@@ -1,3 +1,2 @@"));
    }

    #[test]
    fn fix_hunk_headers_preserves_tail() {
        let input = "\
--- a/f.rs
+++ b/f.rs
@@ -10,0 +10,0 @@ fn main()
+new_line
";
        let result = fix_hunk_headers(input);
        assert!(result.contains(" fn main()"));
    }

    // -- parse_hunk_header --

    #[test]
    fn parse_hunk_header_basic() {
        let (old, new, tail) = parse_hunk_header("@@ -10,5 +20,3 @@");
        assert_eq!(old, 10);
        assert_eq!(new, 20);
        assert_eq!(tail, "");
    }

    #[test]
    fn parse_hunk_header_with_function_context() {
        let (old, new, tail) = parse_hunk_header("@@ -1,4 +1,4 @@ fn main()");
        assert_eq!(old, 1);
        assert_eq!(new, 1);
        assert_eq!(tail, " fn main()");
    }

    #[test]
    fn parse_hunk_header_no_comma() {
        let (old, new, _) = parse_hunk_header("@@ -1 +1 @@");
        assert_eq!(old, 1);
        assert_eq!(new, 1);
    }

    // -- extract_user_text_from_prompt --

    #[test]
    fn extract_task_from_initial_prompt() {
        let prompt = build_prompt("Fix the bug", "src/main.rs", 42, None, &[], None);
        assert_eq!(extract_user_text_from_prompt(&prompt), "Fix the bug");
    }

    #[test]
    fn extract_task_from_global_prompt() {
        let prompt = build_prompt("Add tests", "", 0, None, &[], None);
        assert_eq!(extract_user_text_from_prompt(&prompt), "Add tests");
    }

    #[test]
    fn extract_feedback_from_reply_prompt() {
        let prompt = build_reply_prompt(
            "original task",
            "f.rs",
            1,
            None,
            "some diff",
            "please fix the typo",
            &[],
            None,
        );
        assert_eq!(
            extract_user_text_from_prompt(&prompt),
            "please fix the typo"
        );
    }

    #[test]
    fn extract_from_plain_text() {
        assert_eq!(
            extract_user_text_from_prompt("just plain text"),
            "just plain text"
        );
    }

    // -- parse_command_prefix --

    #[test]
    fn parse_command_prefix_basic() {
        let (name, rest) = parse_command_prefix("[plan] Add auth").unwrap();
        assert_eq!(name, "plan");
        assert_eq!(rest, "Add auth");
    }

    #[test]
    fn parse_command_prefix_no_bracket() {
        assert!(parse_command_prefix("just text").is_none());
    }

    #[test]
    fn parse_command_prefix_empty_name() {
        assert!(parse_command_prefix("[] some text").is_none());
    }

    #[test]
    fn parse_command_prefix_with_spaces_in_name() {
        assert!(parse_command_prefix("[two words] text").is_none());
    }

    #[test]
    fn parse_command_prefix_leading_whitespace() {
        let (name, rest) = parse_command_prefix("  [test] stuff").unwrap();
        assert_eq!(name, "test");
        assert_eq!(rest, "stuff");
    }

    #[test]
    fn parse_command_prefix_no_rest() {
        let (name, rest) = parse_command_prefix("[review]").unwrap();
        assert_eq!(name, "review");
        assert_eq!(rest, "");
    }
}
