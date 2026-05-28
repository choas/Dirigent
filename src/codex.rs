use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::claude;

const WATCHDOG_POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug)]
pub(crate) enum CodexError {
    NotFound,
    SpawnFailed(std::io::Error),
    StreamReadError(std::io::Error),
    Cancelled,
    NonZeroExit(std::process::ExitStatus),
    InvalidExtraArgs(String),
}

impl std::fmt::Display for CodexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodexError::NotFound => write!(f, "codex CLI not found on PATH"),
            CodexError::SpawnFailed(e) => write!(f, "failed to spawn codex: {e}"),
            CodexError::StreamReadError(e) => write!(f, "failed to read codex stdout: {e}"),
            CodexError::Cancelled => write!(f, "cancelled"),
            CodexError::InvalidExtraArgs(args) => {
                write!(f, "failed to parse extra_args (unmatched quote?): {args}")
            }
            CodexError::NonZeroExit(status) => write!(f, "codex exited with {status}"),
        }
    }
}

impl std::error::Error for CodexError {}

#[derive(Debug, Clone)]
pub(crate) struct CodexResponse {
    pub stdout: String,
    pub edited_files: Vec<String>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    pub num_turns: Option<u64>,
}

pub(crate) struct CodexRunConfig<'a> {
    pub model: &'a str,
    pub cli_path: &'a str,
    pub extra_args: &'a str,
    pub env_vars: &'a str,
    pub pre_run_script: &'a str,
    pub post_run_script: &'a str,
    pub skip_permissions: bool,
}

#[derive(Default)]
struct StreamMetrics {
    cost_usd: Option<f64>,
    duration_ms: Option<u64>,
    num_turns: Option<u64>,
}

fn run_hook_script(
    label: &str,
    script: &str,
    project_root: &Path,
    on_log: &mut impl FnMut(&str),
    fail_on_error: bool,
) -> Result<(), CodexError> {
    if script.trim().is_empty() {
        return Ok(());
    }
    on_log(&format!("▶ {}: {}\n", label, script.trim()));
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
            if !output.status.success() && fail_on_error {
                return Err(CodexError::SpawnFailed(std::io::Error::other(format!(
                    "{} script failed (exit {})",
                    label, output.status
                ))));
            }
        }
        Err(e) => {
            if fail_on_error {
                return Err(CodexError::SpawnFailed(e));
            }
        }
    }
    Ok(())
}

fn resolve_codex_bin(cli_path: &str) -> Result<PathBuf, CodexError> {
    let bin = if cli_path.is_empty() {
        "codex"
    } else {
        cli_path
    };
    which::which(bin).map_err(|_| CodexError::NotFound)
}

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

fn process_event_stream(
    stdout_handle: impl std::io::Read,
    cancel: &AtomicBool,
    on_log: &mut impl FnMut(&str),
) -> Result<(String, Vec<String>, StreamMetrics), std::io::Error> {
    use std::io::BufRead;

    let reader = std::io::BufReader::new(stdout_handle);
    let mut final_result = String::new();
    let mut edited_files = Vec::new();
    let mut metrics = StreamMetrics::default();

    for line_result in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let line = line_result?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Ok(event) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(text) = extract_text_from_event(&event) {
                on_log(text);
                on_log("\n");
                final_result.push_str(text);
                final_result.push('\n');
            }
            if let Some(path) = event
                .get("path")
                .and_then(|p| p.as_str())
                .or_else(|| event.get("file_path").and_then(|p| p.as_str()))
            {
                if !edited_files.iter().any(|f| f == path) {
                    edited_files.push(path.to_string());
                }
            }
            if let Some(cost) = event.get("cost_usd").and_then(|c| c.as_f64()) {
                metrics.cost_usd = Some(cost);
            }
            if let Some(duration_ms) = event.get("duration_ms").and_then(|d| d.as_u64()) {
                metrics.duration_ms = Some(duration_ms);
            }
            if is_turn_complete_event(&event) {
                metrics.num_turns = Some(metrics.num_turns.unwrap_or(0).saturating_add(1));
            }
            continue;
        }

        on_log(&format!("{}\n", line));
        final_result.push_str(line);
        final_result.push('\n');
    }

    Ok((final_result, edited_files, metrics))
}

fn stream_stderr(
    stderr_handle: impl std::io::Read,
    cancel: Arc<AtomicBool>,
) -> Result<String, std::io::Error> {
    use std::io::BufRead;
    let reader = std::io::BufReader::new(stderr_handle);
    let mut all = String::new();
    for line_result in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let line = line_result?;
        if line.trim().is_empty() {
            continue;
        }
        all.push_str(&line);
        all.push('\n');
    }
    Ok(all)
}

pub(crate) fn invoke_codex_streaming(
    prompt: &str,
    project_root: &Path,
    config: &CodexRunConfig<'_>,
    mut on_log: impl FnMut(&str),
    cancel: Arc<AtomicBool>,
) -> Result<CodexResponse, CodexError> {
    let codex_bin = resolve_codex_bin(config.cli_path)?;
    run_hook_script(
        "Pre-run script",
        config.pre_run_script,
        project_root,
        &mut on_log,
        true,
    )?;

    let mut cmd = Command::new(codex_bin);
    cmd.arg("exec").arg("--json");
    if config.skip_permissions {
        cmd.arg("--yolo");
    }
    if !config.model.is_empty() {
        cmd.arg("--model").arg(config.model);
    }
    if !config.extra_args.trim().is_empty() {
        let args = shlex::split(config.extra_args)
            .ok_or_else(|| CodexError::InvalidExtraArgs(config.extra_args.to_string()))?;
        for arg in args {
            cmd.arg(arg);
        }
    }
    cmd.arg(prompt)
        .current_dir(project_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let yolo_note = if config.skip_permissions {
        "--yolo "
    } else {
        ""
    };
    let model_note = if config.model.is_empty() {
        String::new()
    } else {
        format!("--model {} ", config.model)
    };
    on_log(&format!(
        "▶ codex exec --json {}{}<prompt>\n",
        yolo_note, model_note
    ));

    claude::apply_env_vars(&mut cmd, config.env_vars, &mut on_log);
    claude::apply_dirigent_env(&mut cmd, project_root, &mut on_log);

    let child = cmd.spawn().map_err(CodexError::SpawnFailed)?;
    let child = Arc::new(Mutex::new(child));
    let done = Arc::new(AtomicBool::new(false));
    let watchdog = spawn_watchdog(&child, &cancel, &done);

    let (final_result, edited_files, metrics, stderr, status) = {
        let mut guard = child.lock().expect("child mutex poisoned");
        let stdout_handle = guard
            .stdout
            .take()
            .ok_or_else(|| CodexError::SpawnFailed(std::io::Error::other("missing stdout")))?;
        let stderr_handle = guard
            .stderr
            .take()
            .ok_or_else(|| CodexError::SpawnFailed(std::io::Error::other("missing stderr")))?;

        let cancel_for_stderr = Arc::clone(&cancel);
        let stderr_thread =
            std::thread::spawn(move || stream_stderr(stderr_handle, cancel_for_stderr));

        let (result, files, metrics) = process_event_stream(stdout_handle, &cancel, &mut on_log)
            .map_err(CodexError::StreamReadError)?;

        let stderr_buf = stderr_thread
            .join()
            .unwrap_or_else(|_| Ok(String::new()))
            .unwrap_or_default();

        let status = guard.wait().map_err(CodexError::SpawnFailed)?;
        (result, files, metrics, stderr_buf, status)
    };

    done.store(true, Ordering::Relaxed);
    let _ = watchdog.join();

    if !stderr.is_empty() {
        on_log(&stderr);
    }

    run_hook_script(
        "Post-run script",
        config.post_run_script,
        project_root,
        &mut on_log,
        false,
    )?;

    if cancel.load(Ordering::Relaxed) {
        return Err(CodexError::Cancelled);
    }
    if !status.success() {
        return Err(CodexError::NonZeroExit(status));
    }

    Ok(CodexResponse {
        stdout: final_result,
        edited_files,
        cost_usd: metrics.cost_usd,
        duration_ms: metrics.duration_ms,
        num_turns: metrics.num_turns,
    })
}

pub(crate) fn parse_diff_from_response(_response: &str) -> Option<String> {
    None
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
