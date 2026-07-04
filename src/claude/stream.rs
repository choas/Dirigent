use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use claude_pty::{Event, ExitStatus, PermissionDialogKind, PollEvent, Session, SessionState};

use super::done_hook::DoneHook;
use super::types::ClaudeRunMetadata;

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const SENTINEL_GRACE: Duration = Duration::from_secs(3);
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// State accumulated while consuming PTY events.
pub(super) struct PtyResult {
    pub response: String,
    pub metadata: ClaudeRunMetadata,
}

/// Close the session while forwarding the lines Claude emits during shutdown.
///
/// Triggering the exit sends `Ctrl-C`, which cuts an in-progress turn short and
/// makes Claude Code print an "Interrupted · What should Claude do instead?"
/// line (also written to the session transcript). The plain `close_gracefully`
/// discards those drained events, so the line was invisible in Dirigent and
/// only reappeared when the session was later opened with `claude --resume`.
/// Draining forwards it to `on_log` so it shows up live instead.
fn graceful_close(session: &mut Session, timeout: Duration, on_log: &mut dyn FnMut(&str)) {
    let _ = session.close_gracefully_draining(timeout, |event| {
        if let Event::FilteredLine { ansi, .. } = event {
            on_log(ansi);
            on_log("\n");
        }
    });
}

struct PtyConsumeState<'a> {
    result: PtyResult,
    prompt_sent: bool,
    start_time: Instant,
    last_event_time: Instant,
    sentinel_seen: Option<Instant>,
    waiting_for_permission: bool,
    saw_filtered_line: bool,
    survey_answered: bool,
    resume_choice_answered: bool,
    done_sentinel: Option<PathBuf>,
    done_hook: Option<&'a DoneHook>,
    skip_permissions: bool,
}

impl<'a> PtyConsumeState<'a> {
    fn new(done_hook: Option<&'a DoneHook>, skip_permissions: bool) -> Self {
        Self {
            result: PtyResult {
                response: String::new(),
                metadata: ClaudeRunMetadata::default(),
            },
            prompt_sent: false,
            start_time: Instant::now(),
            last_event_time: Instant::now(),
            sentinel_seen: None,
            waiting_for_permission: false,
            saw_filtered_line: false,
            survey_answered: false,
            resume_choice_answered: false,
            done_sentinel: done_hook.map(|hook| hook.sentinel_path().to_path_buf()),
            done_hook,
            skip_permissions,
        }
    }

    fn handle_cancel(
        &mut self,
        session: &mut Session,
        cancel: &AtomicBool,
        on_log: &mut dyn FnMut(&str),
    ) -> bool {
        if !cancel.load(Ordering::Relaxed) {
            return false;
        }
        on_log("\n⚠ Run cancelled.\n");
        self.result.metadata.completion_reason = Some("cancelled".to_string());
        graceful_close(session, Duration::from_secs(2), on_log);
        true
    }

    fn handle_ready_event(
        &mut self,
        session: &mut Session,
        prompt: &str,
        on_log: &mut dyn FnMut(&str),
        event: Event,
    ) -> bool {
        self.mark_output_seen();
        match event {
            Event::FilteredLine {
                ref text, ref ansi, ..
            } => self.handle_filtered_line(session, on_log, text, ansi),
            Event::PtyActivity { .. } => on_log("\0"),
            Event::ParserWarning { ref message } => {
                self.result.metadata.parser_warnings.push(message.clone());
            }
            Event::SessionStateChanged {
                state: session_state,
            } => self.handle_session_state(session_state),
            Event::PermissionDialogDetected { ref dialog } => {
                self.handle_permission_dialog(session, on_log, dialog);
            }
            Event::TuiToolConfirmation { .. } => {
                self.handle_tool_confirmation(session, on_log);
            }
            Event::TuiPrompt => return self.handle_tui_prompt(session, prompt, on_log),
            Event::TuiScreen {
                ref lines,
                ref lines_ansi,
                ..
            } => self.handle_tui_screen(session, on_log, lines, lines_ansi),
            Event::LibDone => return true,
            Event::LibError { ref message } => {
                on_log(&format!("error: {}\n", message));
                self.result.metadata.completion_reason = Some("pty_error".to_string());
                return true;
            }
            _ => {}
        }
        false
    }

    fn mark_output_seen(&mut self) {
        self.last_event_time = Instant::now();
        let elapsed_ms = self.start_time.elapsed().as_millis() as u64;
        self.result
            .metadata
            .first_output_ms
            .get_or_insert(elapsed_ms);
        self.result.metadata.last_output_ms = Some(elapsed_ms);
    }

    fn handle_filtered_line(
        &mut self,
        session: &Session,
        on_log: &mut dyn FnMut(&str),
        text: &str,
        ansi: &str,
    ) {
        self.saw_filtered_line = true;
        on_log(ansi);
        on_log("\n");
        self.append_response_line(text);
        self.answer_rating_if_needed(session, on_log, text);
        self.answer_resume_choice_if_needed(session, on_log, text);
    }

    fn append_response_line(&mut self, line: &str) {
        self.result.response.push_str(line);
        self.result.response.push('\n');
    }

    fn answer_rating_if_needed(
        &mut self,
        session: &Session,
        on_log: &mut dyn FnMut(&str),
        line: &str,
    ) {
        if !self.survey_answered && is_session_rating_prompt(line) {
            answer_session_rating(session, on_log);
            self.survey_answered = true;
        }
    }

    /// Auto-confirm the `--resume` resume-choice dialog. Gated on
    /// `!prompt_sent` so quoted text in later session output cannot trigger a
    /// stray keypress.
    fn answer_resume_choice_if_needed(
        &mut self,
        session: &Session,
        on_log: &mut dyn FnMut(&str),
        line: &str,
    ) {
        if !self.prompt_sent && !self.resume_choice_answered && is_resume_choice_prompt(line) {
            answer_resume_choice(session, on_log);
            self.resume_choice_answered = true;
        }
    }

    fn handle_session_state(&mut self, session_state: SessionState) {
        if session_state == SessionState::WaitingForPermission {
            self.waiting_for_permission = true;
        }
    }

    fn handle_permission_dialog(
        &mut self,
        session: &Session,
        on_log: &mut dyn FnMut(&str),
        dialog: &claude_pty::PermissionDialog,
    ) {
        let summary = permission_dialog_summary(dialog);
        self.result
            .metadata
            .permission_summaries
            .push(summary.clone());
        match dialog.kind {
            PermissionDialogKind::TrustFolder => {
                on_log(&format!("\n⏎ Permission: {summary} — accepted\n"));
                let _ = session.confirm_permission_option();
                self.waiting_for_permission = false;
            }
            PermissionDialogKind::ToolUse if self.skip_permissions => {
                on_log(&format!("\n⏎ Permission: {summary} — accepted by policy\n"));
                let _ = session.confirm_permission_option();
                self.waiting_for_permission = false;
            }
            _ => {
                on_log(&format!("\n⚠ Waiting for permission: {summary}\n"));
                self.result.metadata.completion_reason = Some("waiting_for_permission".to_string());
            }
        }
    }

    fn handle_tool_confirmation(&mut self, session: &Session, on_log: &mut dyn FnMut(&str)) {
        if self.skip_permissions {
            let _ = session.confirm_permission_option();
        } else {
            self.waiting_for_permission = true;
            on_log("\n⚠ Waiting for permission.\n");
        }
    }

    fn handle_tui_prompt(
        &mut self,
        session: &mut Session,
        prompt: &str,
        on_log: &mut dyn FnMut(&str),
    ) -> bool {
        if !self.prompt_sent {
            return self.send_initial_prompt(session, prompt, on_log);
        }
        if self.done_sentinel.is_none() {
            self.result.metadata.completion_reason = Some("prompt_returned".to_string());
            graceful_close(session, Duration::from_secs(5), on_log);
            return true;
        }
        false
    }

    fn send_initial_prompt(
        &mut self,
        session: &mut Session,
        prompt: &str,
        on_log: &mut dyn FnMut(&str),
    ) -> bool {
        if let Err(e) = session.send_prompt(prompt) {
            on_log(&format!("\n⚠ Failed to send prompt: {e}\n"));
            self.result.metadata.completion_reason = Some("prompt_send_failed".to_string());
            return true;
        }
        self.result.metadata.prompt_submitted_ms =
            Some(self.start_time.elapsed().as_millis() as u64);
        self.prompt_sent = true;
        false
    }

    fn handle_tui_screen(
        &mut self,
        session: &Session,
        on_log: &mut dyn FnMut(&str),
        lines: &[String],
        lines_ansi: &[String],
    ) {
        if self.saw_filtered_line {
            return;
        }
        for (plain, ansi) in lines.iter().zip(lines_ansi.iter()) {
            self.handle_tui_screen_line(session, on_log, plain, ansi);
        }
    }

    fn handle_tui_screen_line(
        &mut self,
        session: &Session,
        on_log: &mut dyn FnMut(&str),
        plain: &str,
        ansi: &str,
    ) {
        if plain.trim().is_empty() || is_tui_chrome_line(plain) {
            on_log("\0");
            return;
        }
        on_log(ansi);
        on_log("\n");
        self.append_response_line(plain);
        self.answer_rating_if_needed(session, on_log, plain);
        self.answer_resume_choice_if_needed(session, on_log, plain);
    }

    fn handle_pending(&mut self, session: &mut Session, on_log: &mut dyn FnMut(&str)) -> bool {
        if !self.prompt_sent {
            return false;
        }
        self.capture_stop_hook_if_ready(on_log);
        if self.sentinel_seen.is_some() {
            return self.finish_after_sentinel_grace(session, on_log);
        }
        if self.waiting_for_permission {
            return false;
        }
        self.finish_after_idle_timeout(session, on_log)
    }

    fn capture_stop_hook_if_ready(&mut self, on_log: &mut dyn FnMut(&str)) {
        let Some(sentinel) = &self.done_sentinel else {
            return;
        };
        if self.sentinel_seen.is_some() || !sentinel.exists() {
            return;
        }
        self.sentinel_seen = Some(Instant::now());
        self.result.metadata.completion_reason = Some("stop_hook".to_string());
        self.capture_stop_hook_payload(on_log);
        on_log("\n⏎ Stop hook fired — draining remaining output…\n");
    }

    fn capture_stop_hook_payload(&mut self, on_log: &mut dyn FnMut(&str)) {
        let Some(hook) = self.done_hook else {
            return;
        };
        let Some(summary) = hook.read_summary() else {
            return;
        };
        if let Some(last) = summary.last_assistant_message.as_ref() {
            self.result.response = format!("{}\n", last.trim_end());
        }
        self.result.metadata.stop_hook = Some(summary);
        on_log(&format!(
            "\n⏎ Stop hook payload captured: {}\n",
            hook.payload_path().display()
        ));
    }

    fn finish_after_sentinel_grace(
        &mut self,
        session: &mut Session,
        on_log: &mut dyn FnMut(&str),
    ) -> bool {
        let seen_at = self.sentinel_seen.expect("sentinel already checked");
        let since_sentinel = seen_at.elapsed();
        let since_event = self.last_event_time.elapsed();
        if since_sentinel < SENTINEL_GRACE && since_event < Duration::from_secs(1) {
            return false;
        }
        on_log("\n⏎ Grace period done — exiting.\n");
        graceful_close(session, Duration::from_secs(5), on_log);
        true
    }

    fn finish_after_idle_timeout(
        &mut self,
        session: &mut Session,
        on_log: &mut dyn FnMut(&str),
    ) -> bool {
        if self.last_event_time.elapsed() < IDLE_TIMEOUT {
            return false;
        }
        on_log("\n⚠ No PTY events for 120 s — exiting to avoid stall.\n");
        self.result.metadata.completion_reason = Some("idle_timeout".to_string());
        graceful_close(session, Duration::from_secs(5), on_log);
        true
    }
}

fn permission_dialog_summary(dialog: &claude_pty::PermissionDialog) -> String {
    format!(
        "{:?}: {}{}",
        dialog.kind,
        dialog.title,
        dialog
            .path_or_command
            .as_ref()
            .map(|s| format!(" — {s}"))
            .unwrap_or_default()
    )
}

/// Consume events from a PTY session, forwarding screen output to `on_log`.
///
/// Auto-accepts all confirmation dialogs (trust-folder and tool permissions),
/// auto-confirms the `--resume` "Resume from summary" choice dialog, and sends
/// `prompt` on the first [`TuiPrompt`](Event::TuiPrompt). The loop
/// exits when:
///
/// 1. The `done_sentinel` file appears (Claude Code `Stop` hook fired).
/// 2. A second `TuiPrompt` arrives *and no `done_sentinel` is configured*
///    (fallback heuristic) — triggers [`graceful_exit`]. When a sentinel is
///    active this heuristic is ignored: the underlying library emits a second
///    `TuiPrompt` after only a few seconds of PTY silence, which Claude
///    routinely produces *mid-task* (slow tool, network/rate-limit stall,
///    extended thinking). Acting on it would `Ctrl-C` Claude prematurely, so
///    the sentinel — the authoritative end-of-turn signal — wins instead.
/// 3. The session ends (`LibDone`).
/// 4. No PTY events arrive for 120 s after the prompt is submitted (idle
///    timeout) — triggers [`graceful_exit`]. This is a safety net for both
///    paths: without a sentinel it bounds the no-output case, and with a
///    sentinel it guards against a `Stop` hook that never fired.
pub(super) fn consume_pty_events(
    session: &mut Session,
    prompt: &str,
    cancel: &AtomicBool,
    on_log: &mut dyn FnMut(&str),
    done_hook: Option<&DoneHook>,
    session_id: Option<&str>,
    skip_permissions: bool,
) -> PtyResult {
    let mut state = PtyConsumeState::new(done_hook, skip_permissions);

    loop {
        if state.handle_cancel(session, cancel, on_log) {
            break;
        }

        match session.poll_event() {
            PollEvent::Ready(event) => {
                if state.handle_ready_event(session, prompt, on_log, event) {
                    break;
                }
            }
            PollEvent::Pending => {
                if state.handle_pending(session, on_log) {
                    break;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            PollEvent::Closed => {
                on_log("\n⚠ PTY session closed unexpectedly.\n");
                state.result.metadata.completion_reason = Some("pty_closed".to_string());
                break;
            }
        }
    }

    let elapsed = state.start_time.elapsed();
    let secs = elapsed.as_secs();
    let response_lines = state.result.response.lines().count();
    let session_suffix = match session_id {
        Some(id) if !id.is_empty() => format!(" — session {id}"),
        _ => String::new(),
    };
    if response_lines > 0 {
        on_log(&format!(
            "\nDone {secs}s ({response_lines} lines){session_suffix}\n"
        ));
    } else {
        on_log(&format!("\nDone {secs}s{session_suffix}\n"));
    }

    report_exit_status(session, state.prompt_sent, &state.result.response, on_log);

    state.result
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

// ── TUI chrome filtering ────────────────────────────────────────────

/// Detect Claude Code TUI status/action bar lines that should not appear
/// in the captured output. These contain interactive hints like
/// "(shift+tab to cycle) · esc to interrupt".
fn is_tui_chrome_line(line: &str) -> bool {
    line.contains("esc to interrupt") || line.contains("shift+tab to cycle")
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

// ── Stopped-early detection ─────────────────────────────────────────

/// Detect whether a run was stopped before the AI finished its task.
///
/// Looks at the log for the "Stop hook fired" sentinel exit, then checks
/// whether the last substantive output was mid-tool-use (e.g. tool output
/// lines starting with `⎿` or tool headers `⏺`) rather than a natural
/// language conclusion. Returns `true` when the run appears truncated.
pub(crate) fn detect_stopped_early(log: &str) -> bool {
    if !log.contains("Stop hook fired") {
        return false;
    }
    let last_substantive = log.lines().rev().map(|l| l.trim()).find(|l| {
        !l.is_empty() && !l.starts_with("Done ") && !l.starts_with("⏎") && !l.starts_with("⚠")
    });
    match last_substantive {
        Some(line) => {
            line.starts_with('⎿')
                || line.starts_with('⏺')
                || line.starts_with("Running")
                || line.starts_with("  ")
        }
        None => false,
    }
}

// ── Session-rating survey auto-answer ───────────────────────────────

/// Detect Claude Code's optional end-of-session rating prompt, e.g.:
///
/// ```text
/// ● How is Claude doing this session? (optional)
///   1: Bad    2: Fine   3: Good   0: Dismiss
/// ```
///
/// This is an interactive question Claude routinely emits in the TUI; left
/// unanswered it can hold the PTY open. We match the distinctive question text
/// so Dirigent can answer it automatically and let the run finish.
fn is_session_rating_prompt(line: &str) -> bool {
    line.contains("How is Claude doing this session")
}

/// Answer the session-rating survey by selecting option `3` (Good).
///
/// The survey is a single-key selection (like the permission-option picker), so
/// writing the digit `3` chooses "Good" and dismisses the prompt; no Enter is
/// needed.
fn answer_session_rating(session: &Session, on_log: &mut dyn FnMut(&str)) {
    match session.send_key(b"3") {
        Ok(()) => on_log("\n⏎ Session rating prompt — answered 3 (Good)\n"),
        Err(e) => on_log(&format!(
            "\n⚠ Failed to answer session rating prompt: {e}\n"
        )),
    }
}

// ── Resume-choice dialog auto-answer ────────────────────────────────

/// Detect Claude Code's resume-choice dialog, shown when `--resume` targets an
/// old or token-heavy session, e.g.:
///
/// ```text
/// This session is 1d 12h old and 166.2k tokens.
///
/// Resuming the full session will consume a substantial portion of your
/// usage limits. We recommend resuming from a summary.
///
/// ❯ 1. Resume from summary (recommended)
///   2. Resume full session as-is
///   3. Don't ask me again
///
/// Enter to confirm · Esc to cancel
/// ```
///
/// The dialog appears at session startup, before the TUI prompt, so it is not
/// caught by `claude_pty`'s permission-dialog detection and would block the
/// PTY forever (the idle timeout only arms after the prompt is sent). Callers
/// gate on `!prompt_sent` so quoted text in later session output (e.g. Claude
/// reading this very file) cannot trigger a stray keypress.
fn is_resume_choice_prompt(line: &str) -> bool {
    line.contains("Resume from summary")
        || line.contains("Resuming the full session will consume")
}

/// Answer the resume-choice dialog by confirming the pre-selected option 1
/// ("Resume from summary (recommended)") with Enter, matching the dialog's
/// "Enter to confirm" footer — the same way the trust-folder dialog is
/// accepted.
fn answer_resume_choice(session: &Session, on_log: &mut dyn FnMut(&str)) {
    match session.send_key(b"\r") {
        Ok(()) => on_log("\n⏎ Resume prompt — confirmed 'Resume from summary (recommended)'\n"),
        Err(e) => on_log(&format!("\n⚠ Failed to answer resume prompt: {e}\n")),
    }
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
    fn session_rating_prompt_detected() {
        assert!(is_session_rating_prompt(
            "● How is Claude doing this session? (optional)"
        ));
        assert!(is_session_rating_prompt("How is Claude doing this session"));
        assert!(!is_session_rating_prompt(
            "  1: Bad    2: Fine   3: Good   0: Dismiss"
        ));
        assert!(!is_session_rating_prompt("How are you doing today?"));
        assert!(!is_session_rating_prompt(""));
    }

    #[test]
    fn resume_choice_prompt_detected() {
        assert!(is_resume_choice_prompt(
            "❯ 1. Resume from summary (recommended)"
        ));
        assert!(is_resume_choice_prompt(
            "Resuming the full session will consume a substantial portion of \
             your usage limits. We recommend resuming from a summary."
        ));
        assert!(!is_resume_choice_prompt("2. Resume full session as-is"));
        assert!(!is_resume_choice_prompt(
            "This session is 1d 12h old and 166.2k tokens."
        ));
        assert!(!is_resume_choice_prompt("Enter to confirm · Esc to cancel"));
        assert!(!is_resume_choice_prompt(""));
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

    #[test]
    fn detect_stopped_early_mid_tool_use() {
        let log = "⏺ Bash(sed -n '384,470p' file.mjs)\n\
                   ⎿  function useKeyPress(\n\
                   ⎿  Running…\n\
                   \n\
                   ⏎ Stop hook fired — draining remaining output…\n\
                   \n\
                   ⏎ Grace period done — exiting.\n\
                   \n\
                   Done 324s (122 lines)\n";
        assert!(detect_stopped_early(log));
    }

    #[test]
    fn detect_stopped_early_normal_completion() {
        let log = "⏺ Read file.rs\n\
                   ⎿  contents…\n\
                   I've reviewed the file and everything looks good.\n\
                   \n\
                   ⏎ Stop hook fired — draining remaining output…\n\
                   ⏎ Grace period done — exiting.\n\
                   Done 60s (10 lines)\n";
        assert!(!detect_stopped_early(log));
    }

    #[test]
    fn detect_stopped_early_no_stop_hook() {
        let log = "Some output\nDone 30s (5 lines)\n";
        assert!(!detect_stopped_early(log));
    }

    #[test]
    fn tui_chrome_line_detected() {
        assert!(is_tui_chrome_line(
            "─────────────────────ns on (shift+tab to cycle) · esc to interrupt"
        ));
        assert!(is_tui_chrome_line(
            "2 actions on (shift+tab to cycle) · esc to interrupt"
        ));
        assert!(!is_tui_chrome_line("⏺ Read file.rs"));
        assert!(!is_tui_chrome_line("Normal output text"));
    }
}
