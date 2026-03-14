use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::claude;

const WATCHDOG_POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug)]
pub(crate) enum OpenCodeError {
    NotFound,
    SpawnFailed(std::io::Error),
    Cancelled,
}

impl std::error::Error for OpenCodeError {}

impl std::fmt::Display for OpenCodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenCodeError::NotFound => write!(f, "opencode CLI not found on PATH"),
            OpenCodeError::SpawnFailed(e) => write!(f, "failed to spawn opencode: {e}"),
            OpenCodeError::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct OpenCodeResponse {
    pub stdout: String,
    pub edited_files: Vec<String>,
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

pub(crate) fn invoke_opencode_streaming(
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
) -> Result<OpenCodeResponse, OpenCodeError> {
    use std::io::{BufRead, Read};
    use std::process::Stdio;

    let opencode_bin = if cli_path.is_empty() {
        "opencode"
    } else {
        cli_path
    };

    let which_result = Command::new("which").arg(opencode_bin).output();
    match which_result {
        Ok(output) if !output.status.success() => return Err(OpenCodeError::NotFound),
        Err(_) => return Err(OpenCodeError::NotFound),
        _ => {}
    }

    let mut cmd = Command::new(opencode_bin);
    cmd.arg("run").arg(prompt).arg("--format").arg("json");
    if !model.is_empty() {
        cmd.arg("--model").arg(model);
    }
    for arg in extra_args.split_whitespace() {
        if !arg.is_empty() {
            cmd.arg(arg);
        }
    }

    // Apply user-supplied environment variables (KEY=VALUE per line)
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

    // Run pre-run script
    if !pre_run_script.trim().is_empty() {
        on_log(&format!("\u{25B6} pre-run: {}\n", pre_run_script.trim()));
        let pre_result = Command::new("sh")
            .arg("-c")
            .arg(pre_run_script.trim())
            .current_dir(project_root)
            .output();
        match pre_result {
            Ok(output) => {
                if !output.stdout.is_empty() {
                    on_log(&String::from_utf8_lossy(&output.stdout));
                }
                if !output.stderr.is_empty() {
                    on_log(&String::from_utf8_lossy(&output.stderr));
                }
                if !output.status.success() {
                    let msg = format!("pre-run script failed (exit {})", output.status);
                    on_log(&format!("\u{2717} {}\n", msg));
                    return Err(OpenCodeError::SpawnFailed(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        msg,
                    )));
                }
            }
            Err(e) => {
                on_log(&format!("\u{2717} pre-run script error: {}\n", e));
                return Err(OpenCodeError::SpawnFailed(e));
            }
        }
    }

    let mut child = cmd
        .current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(OpenCodeError::SpawnFailed)?;

    let stderr_handle = child.stderr.take().expect("stderr must be piped");
    let stdout_handle = child.stdout.take().expect("stdout must be piped");

    let child = Arc::new(Mutex::new(child));

    let done = Arc::new(AtomicBool::new(false));
    let watchdog = {
        let child = Arc::clone(&child);
        let cancel = Arc::clone(&cancel);
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

    let stderr_thread = std::thread::spawn(move || {
        let mut s = String::new();
        std::io::BufReader::new(stderr_handle)
            .read_to_string(&mut s)
            .ok();
        s
    });

    let reader = std::io::BufReader::new(stdout_handle);
    let mut final_result = String::new();
    let mut edited_files: Vec<String> = Vec::new();

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

        let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match event_type {
            "text" => {
                let text = event
                    .get("part")
                    .and_then(|p| p.get("text"))
                    .and_then(|t| t.as_str());
                if let Some(text) = text {
                    on_log(text);
                    on_log("\n");
                } else {
                    on_log(&format!(
                        "[DEBUG] text event but no text found: {:?}\n",
                        event
                    ));
                }
            }
            "tool_use" | "tool" => {
                let name = event
                    .get("part")
                    .and_then(|p| p.get("tool").or_else(|| p.get("name")))
                    .and_then(|n| n.as_str())
                    .unwrap_or("?");
                let input = event
                    .get("part")
                    .and_then(|p| p.get("input"))
                    .cloned()
                    .unwrap_or_default();
                let name_lower = name.to_ascii_lowercase();
                let is_file_tool = matches!(name, "Write" | "Edit" | "Bash" | "Task")
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
                    );
                if is_file_tool {
                    // Check common field names for file paths
                    let path = input
                        .get("file_path")
                        .or_else(|| input.get("path"))
                        .or_else(|| input.get("file"))
                        .and_then(|p| p.as_str());
                    if let Some(path) = path {
                        if !edited_files.contains(&path.to_string()) {
                            edited_files.push(path.to_string());
                        }
                    } else if let Some(command) = input.get("command").and_then(|c| c.as_str()) {
                        if name_lower == "bash" {
                            on_log(&format!(
                                "\u{2192} {} {}\n",
                                name,
                                command.lines().next().unwrap_or("")
                            ));
                        }
                    }
                }
                let detail =
                    if let Some(file_path) = input.get("file_path").and_then(|p| p.as_str()) {
                        format!(" {}", file_path)
                    } else if let Some(command) = input.get("command").and_then(|c| c.as_str()) {
                        format!(" $ {}", command.lines().next().unwrap_or(""))
                    } else if let Some(grep) = input.get("pattern").and_then(|p| p.as_str()) {
                        format!(" \"{}\"", grep)
                    } else {
                        String::new()
                    };
                if !detail.is_empty() {
                    on_log(&format!("\u{2192} {}{}\n", name, detail));
                }
            }
            "step_finish" => {
                let reason = event
                    .get("part")
                    .and_then(|p| p.get("reason"))
                    .and_then(|r| r.as_str())
                    .unwrap_or("");
                let cost = event
                    .get("part")
                    .and_then(|p| p.get("cost"))
                    .and_then(|c| c.as_f64())
                    .unwrap_or(0.0);
                let tokens = event.get("part").and_then(|p| p.get("tokens"));
                let duration = tokens
                    .and_then(|t| t.get("total"))
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0);
                if reason == "stop" {
                    if let Some(text) = event
                        .get("part")
                        .and_then(|p| p.get("text"))
                        .and_then(|t| t.as_str())
                    {
                        final_result = text.to_string();
                    }
                }
                on_log(&format!(
                    "\n\u{2713} Done ({:.1}s, ${:.4})\n",
                    duration as f64 / 1_000_000.0,
                    cost
                ));
            }
            "error" => {
                if let Some(error_msg) = event.get("error").and_then(|e| e.get("message")) {
                    on_log(&format!("\nError: {}\n", error_msg));
                }
            }
            _ => {}
        }
    }

    done.store(true, Ordering::Relaxed);
    let _ = watchdog.join();

    match child.lock() {
        Ok(mut c) => {
            let _ = c.wait();
        }
        Err(poisoned) => {
            let _ = poisoned.into_inner().wait();
        }
    }
    let stderr = stderr_thread.join().unwrap_or_default();

    if cancel.load(Ordering::Relaxed) {
        return Err(OpenCodeError::Cancelled);
    }

    if final_result.is_empty() && !stderr.is_empty() {
        on_log(&format!("\nError: {}\n", stderr));
    }

    // Run post-run script
    if !post_run_script.trim().is_empty() {
        on_log(&format!("\u{25B6} post-run: {}\n", post_run_script.trim()));
        let post_result = Command::new("sh")
            .arg("-c")
            .arg(post_run_script.trim())
            .current_dir(project_root)
            .output();
        match post_result {
            Ok(output) => {
                if !output.stdout.is_empty() {
                    on_log(&String::from_utf8_lossy(&output.stdout));
                }
                if !output.stderr.is_empty() {
                    on_log(&String::from_utf8_lossy(&output.stderr));
                }
                if !output.status.success() {
                    on_log(&format!(
                        "\u{2717} post-run script failed (exit {})\n",
                        output.status
                    ));
                }
            }
            Err(e) => {
                on_log(&format!("\u{2717} post-run script error: {}\n", e));
            }
        }
    }

    Ok(OpenCodeResponse {
        stdout: final_result,
        edited_files,
    })
}

pub(crate) fn build_prompt(
    cue_text: &str,
    file_path: &str,
    line_number: usize,
    line_number_end: Option<usize>,
    images: &[String],
) -> String {
    claude::build_prompt(cue_text, file_path, line_number, line_number_end, images)
}

pub(crate) fn build_reply_prompt(
    original_cue: &str,
    file_path: &str,
    line_number: usize,
    line_number_end: Option<usize>,
    previous_diff: &str,
    reply: &str,
    images: &[String],
) -> String {
    claude::build_reply_prompt(
        original_cue,
        file_path,
        line_number,
        line_number_end,
        previous_diff,
        reply,
        images,
    )
}

pub(crate) fn parse_diff_from_response(response: &str) -> Option<String> {
    claude::parse_diff_from_response(response)
}
