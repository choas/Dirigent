use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub enum ClaudeError {
    NotFound,
    SpawnFailed(std::io::Error),
}

impl std::fmt::Display for ClaudeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClaudeError::NotFound => write!(f, "claude CLI not found on PATH"),
            ClaudeError::SpawnFailed(e) => write!(f, "failed to spawn claude: {e}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClaudeResponse {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    /// File paths that Claude edited (from Edit/Write tool_use events).
    pub edited_files: Vec<String>,
}

/// Build a structured prompt for Claude given a cue's context.
pub fn build_prompt(
    cue_text: &str,
    file_path: &str,
    line_number: usize,
    line_number_end: Option<usize>,
) -> String {
    if file_path.is_empty() {
        format!(
            "## Task\n\n{}\n\n\
             ## Instructions\n\n\
             Make the requested changes directly by editing the files. \
             Do not output a diff — use your tools to edit files in place.",
            cue_text,
        )
    } else {
        let line_ref = match line_number_end {
            Some(end) => format!("lines {}-{}", line_number, end),
            None => format!("line {}", line_number),
        };
        format!(
            "## Task\n\n{}\n\n\
             ## Context\n\n\
             Focus on {} in `{}`.\n\n\
             ## Instructions\n\n\
             Make the requested changes directly by editing the files. \
             Do not output a diff — use your tools to edit files in place.",
            cue_text, line_ref, file_path,
        )
    }
}

/// Invoke `claude -p <prompt>` in the given project directory.
pub fn invoke_claude(
    prompt: &str,
    project_root: &Path,
    model: &str,
) -> Result<ClaudeResponse, ClaudeError> {
    let which_result = Command::new("which").arg("claude").output();
    match which_result {
        Ok(output) if !output.status.success() => return Err(ClaudeError::NotFound),
        Err(_) => return Err(ClaudeError::NotFound),
        _ => {}
    }

    let mut cmd = Command::new("claude");
    cmd.arg("-p").arg(prompt);
    if !model.is_empty() {
        cmd.arg("--model").arg(model);
    }
    let output = cmd
        .current_dir(project_root)
        .output()
        .map_err(ClaudeError::SpawnFailed)?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code();

    Ok(ClaudeResponse {
        stdout,
        stderr,
        exit_code,
        edited_files: Vec::new(),
    })
}

/// Invoke `claude -p <prompt> --output-format stream-json` with live progress
/// streaming to a shared log buffer. Parses JSON events from stdout in real-time.
pub fn invoke_claude_streaming(
    prompt: &str,
    project_root: &Path,
    model: &str,
    log: Arc<Mutex<String>>,
) -> Result<ClaudeResponse, ClaudeError> {
    use std::io::{BufRead, Read};
    use std::process::Stdio;

    let which_result = Command::new("which").arg("claude").output();
    match which_result {
        Ok(output) if !output.status.success() => return Err(ClaudeError::NotFound),
        Err(_) => return Err(ClaudeError::NotFound),
        _ => {}
    }

    let mut cmd = Command::new("claude");
    cmd.arg("-p")
        .arg(prompt)
        .arg("--verbose")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--dangerously-skip-permissions");
    if !model.is_empty() {
        cmd.arg("--model").arg(model);
    }

    let mut child = cmd
        .current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(ClaudeError::SpawnFailed)?;

    let stderr_handle = child.stderr.take().unwrap();
    let stdout_handle = child.stdout.take().unwrap();

    // Collect stderr in background (for error messages)
    let stderr_thread = std::thread::spawn(move || {
        let mut s = String::new();
        std::io::BufReader::new(stderr_handle)
            .read_to_string(&mut s)
            .ok();
        s
    });

    // Parse stream-json events from stdout in real-time
    let reader = std::io::BufReader::new(stdout_handle);
    let log_clone = Arc::clone(&log);
    let mut final_result = String::new();
    let mut edited_files: Vec<String> = Vec::new();

    for line in reader.lines().flatten() {
        let event: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                // Non-JSON line, just log it
                if let Ok(mut log) = log_clone.lock() {
                    log.push_str(&line);
                    log.push('\n');
                }
                continue;
            }
        };

        let event_type = event
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match event_type {
            "assistant" => {
                if let Some(content) = event
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for block in content {
                        let block_type =
                            block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match block_type {
                            "text" => {
                                if let Some(text) =
                                    block.get("text").and_then(|t| t.as_str())
                                {
                                    // Show a truncated preview of assistant text
                                    let preview: String =
                                        text.chars().take(200).collect();
                                    if let Ok(mut log) = log_clone.lock() {
                                        log.push_str(&preview);
                                        if text.len() > 200 {
                                            log.push_str("...");
                                        }
                                        log.push('\n');
                                    }
                                }
                            }
                            "tool_use" => {
                                let name = block
                                    .get("name")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("?");
                                let input =
                                    block.get("input").cloned().unwrap_or_default();
                                // Track files edited by Claude
                                if matches!(name, "Edit" | "Write" | "NotebookEdit") {
                                    if let Some(path) =
                                        input.get("file_path").and_then(|p| p.as_str())
                                    {
                                        if !edited_files.contains(&path.to_string()) {
                                            edited_files.push(path.to_string());
                                        }
                                    }
                                }
                                let detail = if let Some(cmd) =
                                    input.get("command").and_then(|c| c.as_str())
                                {
                                    format!(
                                        " $ {}",
                                        cmd.lines().next().unwrap_or("")
                                    )
                                } else if let Some(path) =
                                    input.get("file_path").and_then(|p| p.as_str())
                                {
                                    format!(" {}", path)
                                } else if let Some(pattern) =
                                    input.get("pattern").and_then(|p| p.as_str())
                                {
                                    format!(" \"{}\"", pattern)
                                } else {
                                    String::new()
                                };
                                if let Ok(mut log) = log_clone.lock() {
                                    log.push_str(&format!(
                                        "\u{2192} {}{}\n",
                                        name, detail
                                    ));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            "result" => {
                if let Some(result) =
                    event.get("result").and_then(|r| r.as_str())
                {
                    final_result = result.to_string();
                }
                // Show cost and duration
                let cost = event
                    .get("cost_usd")
                    .and_then(|c| c.as_f64())
                    .unwrap_or(0.0);
                let duration = event
                    .get("duration_ms")
                    .and_then(|d| d.as_u64())
                    .unwrap_or(0);
                let turns = event
                    .get("num_turns")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0);
                if let Ok(mut log) = log_clone.lock() {
                    log.push_str(&format!(
                        "\n\u{2713} Done ({} turns, {:.1}s, ${:.4})\n",
                        turns,
                        duration as f64 / 1000.0,
                        cost
                    ));
                }
            }
            "system" => {
                if let Some(subtype) =
                    event.get("subtype").and_then(|s| s.as_str())
                {
                    if let Ok(mut log) = log_clone.lock() {
                        log.push_str(&format!("[{}]\n", subtype));
                    }
                }
            }
            _ => {
                // Log unknown event types briefly
                if !event_type.is_empty() {
                    if let Ok(mut log) = log_clone.lock() {
                        log.push_str(&format!("[{}]\n", event_type));
                    }
                }
            }
        }
    }

    let status = child.wait().map_err(ClaudeError::SpawnFailed)?;
    let stderr = stderr_thread.join().unwrap_or_default();

    // If we didn't get a result from stream events, stderr might have the error
    if final_result.is_empty() && !stderr.is_empty() {
        if let Ok(mut log) = log.lock() {
            log.push_str(&format!("\nError: {}\n", stderr));
        }
    }

    Ok(ClaudeResponse {
        stdout: final_result,
        stderr,
        exit_code: status.code(),
        edited_files,
    })
}

/// Parse diff content from a Claude response.
pub fn parse_diff_from_response(response: &str) -> Option<String> {
    if let Some(diff) = extract_fenced_diff(response) {
        return Some(diff);
    }
    extract_unified_diff(response)
}

fn extract_fenced_diff(response: &str) -> Option<String> {
    let mut blocks = Vec::new();
    let mut current_lines: Vec<&str> = Vec::new();
    let mut in_block = false;

    for line in response.lines() {
        let trimmed = line.trim_start();
        if !in_block && trimmed.starts_with("```diff") {
            in_block = true;
            current_lines.clear();
        } else if in_block && trimmed.starts_with("```") {
            if !current_lines.is_empty() {
                blocks.push(current_lines.join("\n"));
            }
            in_block = false;
            current_lines.clear();
        } else if in_block {
            current_lines.push(line);
        }
    }

    if blocks.is_empty() {
        return None;
    }

    let mut diffs = Vec::new();
    for block in &blocks {
        if let Some(clean_diff) = extract_unified_diff(block) {
            diffs.push(clean_diff);
        }
    }

    if diffs.is_empty() {
        None
    } else {
        let mut result = diffs.join("\n");
        if !result.ends_with('\n') {
            result.push('\n');
        }
        Some(result)
    }
}

fn extract_unified_diff(response: &str) -> Option<String> {
    let lines: Vec<&str> = response.lines().collect();
    let mut result: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if lines[i].starts_with("--- ")
            && i + 1 < lines.len()
            && lines[i + 1].starts_with("+++ ")
        {
            result.push(lines[i]);
            i += 1;
            result.push(lines[i]);
            i += 1;

            while i < lines.len() {
                let line = lines[i];
                if line.starts_with("@@ ")
                    || line.starts_with('+')
                    || line.starts_with('-')
                    || line.starts_with(' ')
                {
                    result.push(line);
                    i += 1;
                } else if line.starts_with("--- ")
                    && i + 1 < lines.len()
                    && lines[i + 1].starts_with("+++ ")
                {
                    break;
                } else {
                    break;
                }
            }
        } else {
            i += 1;
        }
    }

    if result.is_empty() {
        None
    } else {
        let mut text = result.join("\n");
        if !text.ends_with('\n') {
            text.push('\n');
        }
        Some(fix_hunk_headers(&text))
    }
}

fn fix_hunk_headers(diff_text: &str) -> String {
    let lines: Vec<&str> = diff_text.lines().collect();
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if !lines[i].starts_with("@@ ") {
            result.push(lines[i].to_string());
            i += 1;
            continue;
        }

        let header = lines[i];
        let (old_start, new_start, tail) = parse_hunk_header(header);

        let mut old_count = 0usize;
        let mut new_count = 0usize;
        let mut j = i + 1;
        while j < lines.len() {
            let line = lines[j];
            if line.starts_with("@@ ")
                || (line.starts_with("--- ")
                    && j + 1 < lines.len()
                    && lines[j + 1].starts_with("+++ "))
            {
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

        let new_header = format!(
            "@@ -{},{} +{},{} @@{}",
            old_start, old_count, new_start, new_count, tail
        );
        result.push(new_header);
        i += 1;
    }

    let mut text = result.join("\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
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
