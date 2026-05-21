use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use claude_pty::{Event, ExitStatus, PollEvent, Session};

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const IDLE_EXIT_SECS: u64 = 10;

/// State accumulated while consuming PTY events.
pub(super) struct PtyResult {
    pub response: String,
}

/// Consume events from a PTY session, forwarding screen output to `on_log`.
///
/// Auto-accepts all confirmation dialogs (trust-folder and tool permissions)
/// and sends `prompt` on the first `TuiPrompt`. When a second prompt appears
/// (Claude finished and is waiting for new input), the idle-exit timer fires,
/// or the session ends (`LibDone`), the loop exits gracefully.
pub(super) fn consume_pty_events(
    session: &mut Session,
    prompt: &str,
    cancel: &AtomicBool,
    on_log: &mut dyn FnMut(&str),
) -> PtyResult {
    let mut state = PtyResult {
        response: String::new(),
    };
    let mut prompt_sent = false;
    let start_time = Instant::now();
    let mut last_event_time = Instant::now();

    loop {
        if cancel.load(Ordering::Relaxed) {
            on_log("\n⚠ Run cancelled.\n");
            let _ = session.kill();
            break;
        }

        match session.poll_event() {
            PollEvent::Ready(event) => {
                last_event_time = Instant::now();
                match event {
                    Event::TuiToolConfirmation { .. } => {
                        let _ = session.write_raw(b"\r");
                    }
                    Event::TuiPrompt => {
                        if !prompt_sent {
                            std::thread::sleep(Duration::from_millis(300));
                            if let Err(e) = session.write_raw(prompt.as_bytes()) {
                                on_log(&format!("\n⚠ Failed to send prompt: {e}\n"));
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(200));
                            if let Err(e) = session.write_raw(b"\r") {
                                on_log(&format!("\n⚠ Failed to submit prompt: {e}\n"));
                                break;
                            }
                            prompt_sent = true;
                        } else {
                            graceful_exit(session);
                            break;
                        }
                    }
                    Event::TuiScreen {
                        ref lines,
                        ref lines_ansi,
                        ..
                    } => {
                        for (plain, ansi) in lines.iter().zip(lines_ansi.iter()) {
                            if plain.trim().is_empty() {
                                // Drop the empty line from the visible log
                                // but emit a heartbeat sentinel so the
                                // activity strip still ticks for every PTY
                                // line. `\0` is stripped on the receiver
                                // side before display.
                                on_log("\0");
                                continue;
                            }
                            on_log(ansi);
                            on_log("\n");
                            state.response.push_str(plain);
                            state.response.push('\n');
                        }
                    }
                    Event::LibDone => break,
                    Event::LibError { ref message } => {
                        on_log(&format!("error: {}\n", message));
                        break;
                    }
                    _ => {}
                }
            }
            PollEvent::Pending => {
                if prompt_sent && last_event_time.elapsed() >= Duration::from_secs(IDLE_EXIT_SECS) {
                    on_log(&format!(
                        "\n⚠ No output for {}s — session timed out.\n",
                        IDLE_EXIT_SECS,
                    ));
                    graceful_exit(session);
                    break;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            PollEvent::Closed => {
                on_log("\n⚠ PTY session closed unexpectedly.\n");
                break;
            }
        }
    }

    let elapsed = start_time.elapsed();
    let secs = elapsed.as_secs();
    let response_lines = state.response.lines().count();
    if response_lines > 0 {
        on_log(&format!("\nDone {secs}s ({response_lines} lines)\n"));
    } else {
        on_log(&format!("\nDone {secs}s\n"));
    }

    report_exit_status(session, prompt_sent, &state.response, on_log);

    state
}

/// End the Claude TUI cleanly: Ctrl-C twice (first interrupts the current
/// operation, second requests exit), drain events for up to 5 s waiting for
/// `LibDone`, then fall back to a hard kill.
fn graceful_exit(session: &mut Session) {
    let _ = session.write_raw(b"\x03");
    std::thread::sleep(Duration::from_millis(500));
    let _ = session.write_raw(b"\x03");

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match session.poll_event() {
            PollEvent::Ready(Event::LibDone) => return,
            PollEvent::Ready(_) => {}
            PollEvent::Pending => std::thread::sleep(POLL_INTERVAL),
            PollEvent::Closed => return,
        }
    }
    let _ = session.kill();
}

fn report_exit_status(
    session: &Session,
    prompt_sent: bool,
    response: &str,
    on_log: &mut dyn FnMut(&str),
) {
    let status: Option<ExitStatus> = session.try_wait();
    let has_response = !response.trim().is_empty();

    match status {
        Some(s) if s.success() => {
            if prompt_sent && !has_response {
                on_log("\n⚠ Claude exited successfully but produced no output.\n");
            }
        }
        Some(s) => {
            on_log(&format!("\n⚠ Claude process exited: {}\n", s));
        }
        None => {
            if !prompt_sent {
                on_log("\n⚠ Claude process still running — prompt was never sent.\n");
            } else if !has_response {
                on_log("\n⚠ Claude process still running — no output received yet.\n");
            }
        }
    }
}

// ── OpenCode log filtering (used by the opencode provider) ─────────

/// Filter OpenCode stderr: drop DEBUG noise, keep WARN/ERROR, delegate INFO
/// and non-structured lines to `handle_non_json_line` for consistent formatting.
pub(crate) fn filter_opencode_log_line(line: &str, on_log: &mut dyn FnMut(&str)) {
    if !is_opencode_log_line(line) {
        handle_non_json_line(line, on_log);
        return;
    }
    if line.starts_with("DEBUG") {
        return;
    }
    if line.starts_with("INFO") {
        handle_non_json_line(line, on_log);
        return;
    }
    on_log(line);
    on_log("\n");
}

fn handle_non_json_line(line: &str, on_log: &mut dyn FnMut(&str)) {
    if !is_opencode_log_line(line) {
        on_log(line);
        on_log("\n");
        return;
    }
    if line.starts_with("WARN") || line.starts_with("ERROR") {
        on_log(line);
        on_log("\n");
        return;
    }
    if let Some(formatted) = format_opencode_service(line) {
        on_log(&formatted);
    }
}

fn format_opencode_service(line: &str) -> Option<String> {
    if line.contains("service=llm") {
        let model = extract_kv(line, "modelID").unwrap_or("?");
        let provider = extract_kv(line, "providerID").unwrap_or("?");
        return Some(format!("\u{2192} {} ({})\n", model, provider));
    }
    if line.contains("service=permission") {
        let perm = extract_kv(line, "permission").unwrap_or("?");
        let pattern = extract_kv(line, "pattern").unwrap_or("?");
        return Some(format!("\u{2192} {} \u{2014} {}\n", perm, pattern));
    }
    if line.contains("service=format") {
        return extract_kv(line, "file").map(|f| format!("\u{2192} format: {}\n", f));
    }
    if line.contains("service=session") && line.contains("created") {
        return extract_kv(line, "slug").map(|s| format!("\u{2192} session: {}\n", s));
    }
    if line.contains("service=vcs") {
        return extract_kv(line, "branch").map(|b| format!("\u{2192} branch: {}\n", b));
    }
    if line.contains("service=lsp") {
        return extract_kv(line, "method").map(|m| format!("\u{2192} lsp: {}\n", m));
    }
    if line.contains("service=session.prompt") {
        if line.contains("exiting loop") {
            return Some("\u{2192} loop done\n".to_string());
        }
        return extract_kv(line, "step").map(|s| format!("\u{2192} step {}\n", s));
    }
    None
}

fn is_opencode_log_line(line: &str) -> bool {
    let rest = if let Some(r) = line.strip_prefix("INFO") {
        r
    } else if let Some(r) = line.strip_prefix("DEBUG") {
        r
    } else if let Some(r) = line.strip_prefix("WARN") {
        r
    } else if let Some(r) = line.strip_prefix("ERROR") {
        r
    } else {
        return false;
    };
    let bytes = rest.trim_start().as_bytes();
    bytes.len() >= 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[0..4].iter().all(|b| b.is_ascii_digit())
}

fn extract_kv<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    for token in line.split_whitespace() {
        if let Some(val) = token
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix('='))
        {
            return Some(val);
        }
    }
    None
}

// ── Plan path extraction ────────────────────────────────────────────

/// Extract a Claude Code plan file path from a log that contains "ExitPlanMode".
/// Looks for a "→ Write ~/.claude/plans/..." line preceding "ExitPlanMode".
/// Returns the expanded absolute path (~ replaced with the home directory).
pub(crate) fn extract_plan_path(log: &str) -> Option<String> {
    if !log.contains("ExitPlanMode") {
        return None;
    }
    let lines: Vec<&str> = log.lines().collect();
    let exit_idx = lines.iter().rposition(|l| l.contains("ExitPlanMode"))?;
    let start = exit_idx.saturating_sub(10);
    for i in (start..exit_idx).rev() {
        let line = lines[i].trim();
        let rest = match line.strip_prefix("\u{2192} Write ") {
            Some(r) => r,
            None => continue,
        };
        if rest.contains(".claude/plans/") {
            let path = rest.trim();
            if let Some(suffix) = path.strip_prefix("~/") {
                if let Some(home) = dirs::home_dir() {
                    return Some(home.join(suffix).to_string_lossy().to_string());
                }
            }
            return Some(path.to_string());
        }
    }
    None
}

// ── Usage limit & question detection ────────────────────────────────

/// Detect a usage-limit / hard rate-limit message in Claude output.
/// Returns the first matching line (trimmed) if found.
pub(crate) fn detect_usage_limit(text: &str) -> Option<&str> {
    const PATTERNS: &[&str] = &[
        "out of extra usage",
        "out of usage",
        "usage limit",
        "token limit reached",
    ];
    for line in text.lines() {
        let lower = line.to_lowercase();
        if PATTERNS.iter().any(|p| lower.contains(p)) {
            return Some(line.trim());
        }
    }
    None
}

/// Check whether the CLI response text contains a question directed at the user.
pub(crate) fn response_has_question(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    for line in trimmed.lines().rev().take(20) {
        let line = line.trim();
        if line.ends_with('?') && line.len() > 5 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_plan_path_typical_log() {
        let log = "Now let me write the final plan file.\n\
                    \u{2192} Write ~/.claude/plans/binary-tinkering-gray.md\n\
                    \u{2192} ToolSearch\n\
                    \u{2192} ExitPlanMode\n";
        let result = extract_plan_path(log);
        assert!(result.is_some());
        let path = result.unwrap();
        assert!(path.ends_with(".claude/plans/binary-tinkering-gray.md"));
        assert!(!path.starts_with('~'));
    }

    #[test]
    fn extract_plan_path_no_exit_plan_mode() {
        let log = "\u{2192} Write ~/.claude/plans/foo.md\n\u{2192} ToolSearch\n";
        assert!(extract_plan_path(log).is_none());
    }

    #[test]
    fn extract_plan_path_no_write_line() {
        let log = "some text\n\u{2192} ExitPlanMode\n";
        assert!(extract_plan_path(log).is_none());
    }

    #[test]
    fn extract_plan_path_write_too_far_above() {
        let mut log = "\u{2192} Write ~/.claude/plans/old.md\n".to_string();
        for _ in 0..12 {
            log.push_str("some other line\n");
        }
        log.push_str("\u{2192} ExitPlanMode\n");
        assert!(extract_plan_path(&log).is_none());
    }

    #[test]
    fn response_has_question_detects_questions() {
        assert!(response_has_question(
            "I noticed a few things.\nWhich approach would you prefer?"
        ));
        assert!(response_has_question(
            "Should I refactor the module or just fix the bug?"
        ));
        assert!(response_has_question("Could you clarify what you mean?"));
    }

    #[test]
    fn response_has_question_ignores_non_questions() {
        assert!(!response_has_question("Done. No changes needed."));
        assert!(!response_has_question("The code looks correct as written."));
        assert!(!response_has_question(""));
        assert!(!response_has_question("   "));
        assert!(!response_has_question("ok?"));
    }

    #[test]
    fn detect_usage_limit_matches_all_patterns() {
        let cases: &[(&str, Option<&str>)] = &[
            (
                "You are out of extra usage",
                Some("You are out of extra usage"),
            ),
            ("Out of usage", Some("Out of usage")),
            (
                "Your usage limit has been reached",
                Some("Your usage limit has been reached"),
            ),
            ("Token limit reached.", Some("Token limit reached.")),
            ("  Out of usage  ", Some("Out of usage")),
            (
                "You are out of usage for today",
                Some("You are out of usage for today"),
            ),
            ("all good\nretrying in 5s\n", None),
            ("Everything is fine", None),
            ("rate_limit_event retry", None),
            ("", None),
        ];
        for &(input, expected) in cases {
            assert_eq!(detect_usage_limit(input), expected, "input: {input:?}");
        }
    }
}
