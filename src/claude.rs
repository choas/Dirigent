use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const WATCHDOG_POLL_INTERVAL: Duration = Duration::from_millis(200);

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
pub(crate) fn build_prompt(
    cue_text: &str,
    file_path: &str,
    line_number: usize,
    line_number_end: Option<usize>,
    images: &[String],
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
    if file_path.is_empty() {
        format!(
            "## Task\n\n{}{}\n\n\
             ## Instructions\n\n\
             Make the requested changes directly by editing the files. \
             Do not output a diff — use your tools to edit files in place.",
            cue_text, images_section,
        )
    } else {
        let line_ref = match line_number_end {
            Some(end) => format!("lines {}-{}", line_number, end),
            None => format!("line {}", line_number),
        };
        format!(
            "## Task\n\n{}{}\n\n\
             ## Context\n\n\
             Focus on {} in `{}`.\n\n\
             ## Instructions\n\n\
             Make the requested changes directly by editing the files. \
             Do not output a diff — use your tools to edit files in place.",
            cue_text, images_section, line_ref, file_path,
        )
    }
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
    use std::io::{BufRead, Read};
    use std::process::Stdio;

    let claude_bin = if cli_path.is_empty() {
        "claude"
    } else {
        cli_path
    };

    let which_result = Command::new("which").arg(claude_bin).output();
    match which_result {
        Ok(output) if !output.status.success() => return Err(ClaudeError::NotFound),
        Err(_) => return Err(ClaudeError::NotFound),
        _ => {}
    }

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
    // Append user-supplied extra arguments
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
                    return Err(ClaudeError::SpawnFailed(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        msg,
                    )));
                }
            }
            Err(e) => {
                on_log(&format!("\u{2717} pre-run script error: {}\n", e));
                return Err(ClaudeError::SpawnFailed(e));
            }
        }
    }

    let mut child = cmd
        .current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(ClaudeError::SpawnFailed)?;

    let stderr_handle = child.stderr.take().unwrap();
    let stdout_handle = child.stdout.take().unwrap();

    // Wrap the child handle so the cancellation watchdog can kill the process.
    let child = Arc::new(Mutex::new(child));

    // Watchdog thread: polls the cancel flag and kills the child process when set.
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
    let mut final_result = String::new();
    let mut edited_files: Vec<String> = Vec::new();

    for line_result in reader.lines() {
        // Check cancellation between lines for fast response
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
                // Non-JSON line, just log it
                on_log(&line);
                on_log("\n");
                continue;
            }
        };

        let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match event_type {
            "assistant" => {
                if let Some(content) = event
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for block in content {
                        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match block_type {
                            "text" => {
                                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                    on_log(text);
                                    on_log("\n");
                                }
                            }
                            "tool_use" => {
                                let name =
                                    block.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                                let input = block.get("input").cloned().unwrap_or_default();
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
                                    format!(" $ {}", cmd.lines().next().unwrap_or(""))
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
                                on_log(&format!("\u{2192} {}{}\n", name, detail));
                            }
                            _ => {}
                        }
                    }
                }
            }
            "result" => {
                if let Some(result) = event.get("result").and_then(|r| r.as_str()) {
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
                let turns = event.get("num_turns").and_then(|t| t.as_u64()).unwrap_or(0);
                on_log(&format!(
                    "\n\u{2713} Done ({} turns, {:.1}s, ${:.4})\n",
                    turns,
                    duration as f64 / 1000.0,
                    cost
                ));
            }
            // Silently ignore known but uninteresting event types
            "system" | "user" | "tool" => {}
            "rate_limit_event" => {
                // Show rate-limit info so the user knows why there's a pause
                let seconds = event
                    .get("retry_after_seconds")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                on_log(&format!(
                    "\u{23f3} Rate limited, retrying in {:.0}s\n",
                    seconds
                ));
            }
            _ => {
                // Log truly unknown event types for debugging
                if !event_type.is_empty() {
                    on_log(&format!("[{}]\n", event_type));
                }
            }
        }
    }

    // Signal the watchdog to stop and wait for it
    done.store(true, Ordering::Relaxed);
    let _ = watchdog.join();

    // Reap the child process (works whether it exited naturally or was killed)
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
        return Err(ClaudeError::Cancelled);
    }

    // If we didn't get a result from stream events, stderr might have the error
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

    Ok(ClaudeResponse {
        stdout: final_result,
        edited_files,
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
        if lines[i].starts_with("--- ") && i + 1 < lines.len() && lines[i + 1].starts_with("+++ ") {
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

#[cfg(test)]
mod tests {
    use super::*;

    // -- build_prompt --

    #[test]
    fn build_prompt_global_cue() {
        let prompt = build_prompt("Add tests", "", 0, None, &[]);
        assert!(prompt.contains("Add tests"));
        assert!(!prompt.contains("Focus on"));
    }

    #[test]
    fn build_prompt_with_file_single_line() {
        let prompt = build_prompt("Fix bug", "src/main.rs", 42, None, &[]);
        assert!(prompt.contains("Fix bug"));
        assert!(prompt.contains("line 42"));
        assert!(prompt.contains("`src/main.rs`"));
    }

    #[test]
    fn build_prompt_with_file_line_range() {
        let prompt = build_prompt("Refactor", "lib.rs", 10, Some(20), &[]);
        assert!(prompt.contains("lines 10-20"));
        assert!(prompt.contains("`lib.rs`"));
    }

    #[test]
    fn build_prompt_with_images() {
        let images = vec![
            "/tmp/screenshot.png".to_string(),
            "/tmp/design.jpg".to_string(),
        ];
        let prompt = build_prompt("Implement this design", "", 0, None, &images);
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
        let prompt = build_prompt("Fix the bug", "src/main.rs", 42, None, &[]);
        assert_eq!(extract_user_text_from_prompt(&prompt), "Fix the bug");
    }

    #[test]
    fn extract_task_from_global_prompt() {
        let prompt = build_prompt("Add tests", "", 0, None, &[]);
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
