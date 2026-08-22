use std::path::Path;
use std::process::{Child, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::ai_backend::HookScriptTrust;

const WATCHDOG_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Largest single line accepted from a child process's stdout/stderr.
///
/// `BufRead::lines()` grows its buffer until it finds a newline, so one
/// newline-less blob — a base64 image in a tool result, a minified bundle
/// echoed by a build, a hostile MCP server — allocates without bound in a
/// worker thread. Provider stream lines are JSON events; a megabyte is far
/// past any legitimate one.
pub(crate) const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Largest total stderr retained for a single run. The full text is only used
/// for error reporting, so keeping the first few MiB loses nothing that
/// matters.
pub(crate) const MAX_STDERR_BYTES: usize = 4 * 1024 * 1024;

/// Marker appended to a line that hit [`MAX_LINE_BYTES`].
const LINE_TRUNCATED_MARKER: &str = "… [line truncated by Dirigent]";

/// Iterator over lines with a hard per-line byte cap.
///
/// Drop-in replacement for [`std::io::BufRead::lines`] that stops accumulating
/// at `max_line` bytes, skips the rest of the over-long line, and marks the
/// result so the truncation is visible rather than silent. Invalid UTF-8 is
/// replaced rather than erroring — the same lossy behaviour the callers'
/// `String::from_utf8_lossy` already relied on.
pub(crate) struct BoundedLines<R> {
    reader: R,
    max_line: usize,
}

/// Wrap `reader` in a per-line-bounded iterator.
pub(crate) fn bounded_lines<R: std::io::BufRead>(reader: R) -> BoundedLines<R> {
    BoundedLines {
        reader,
        max_line: MAX_LINE_BYTES,
    }
}

impl<R: std::io::BufRead> Iterator for BoundedLines<R> {
    type Item = std::io::Result<String>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut buf: Vec<u8> = Vec::new();
        let mut truncated = false;
        let mut saw_any = false;

        loop {
            let available = match self.reader.fill_buf() {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Some(Err(e)),
            };
            if available.is_empty() {
                // EOF: emit a trailing partial line, then stop.
                if !saw_any {
                    return None;
                }
                break;
            }
            saw_any = true;

            let (chunk, consumed, done) = match available.iter().position(|b| *b == b'\n') {
                Some(idx) => (&available[..idx], idx + 1, true),
                None => (available, available.len(), false),
            };
            let room = self.max_line.saturating_sub(buf.len());
            if chunk.len() > room {
                buf.extend_from_slice(&chunk[..room]);
                truncated = true;
            } else {
                buf.extend_from_slice(chunk);
            }
            self.reader.consume(consumed);
            if done {
                break;
            }
        }

        let mut line = String::from_utf8_lossy(&buf).into_owned();
        if line.ends_with('\r') {
            line.pop();
        }
        if truncated {
            line.push_str(LINE_TRUNCATED_MARKER);
        }
        Some(Ok(line))
    }
}

/// Append to a capped accumulator, adding a one-time notice when the cap is
/// reached. Returns `false` once the buffer is full and nothing more is kept.
fn push_capped(buf: &mut String, text: &str, cap: usize) -> bool {
    const CAP_NOTICE: &str = "\n[output truncated by Dirigent — size limit reached]\n";
    if buf.len() >= cap {
        return false;
    }
    if buf.len() + text.len() <= cap {
        buf.push_str(text);
        return true;
    }
    let room = cap - buf.len();
    let mut end = room;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    buf.push_str(&text[..end]);
    buf.push_str(CAP_NOTICE);
    false
}

pub(crate) fn spawn_cancel_watchdog(
    child: &Arc<Mutex<Child>>,
    cancel: &Arc<AtomicBool>,
    done: &Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    let child = Arc::clone(child);
    let cancel = Arc::clone(cancel);
    let done = Arc::clone(done);
    // Check `cancel` before `done`: if cancellation stopped the stdout reader
    // and the main thread set `done` before this watchdog woke from sleep,
    // checking `done` first would let it exit without killing, leaving
    // `wait_for_child` to block on a still-running process.
    std::thread::spawn(move || loop {
        if cancel.load(Ordering::Relaxed) {
            kill_child(&child);
            return;
        }
        if done.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(WATCHDOG_POLL_INTERVAL);
    })
}

pub(crate) fn wait_for_child(child: &Arc<Mutex<Child>>) -> std::io::Result<ExitStatus> {
    match child.lock() {
        Ok(mut child) => child.wait(),
        Err(poisoned) => poisoned.into_inner().wait(),
    }
}

/// Take the piped stdout/stderr handles from a freshly spawned child.
/// When a pipe is missing (it shouldn't be — both were requested as
/// `Stdio::piped()`), the child is killed and reaped and an error is
/// returned instead of panicking.
pub(crate) fn take_child_streams(
    child: &mut Child,
) -> std::io::Result<(std::process::ChildStdout, std::process::ChildStderr)> {
    match (child.stdout.take(), child.stderr.take()) {
        (Some(stdout), Some(stderr)) => Ok((stdout, stderr)),
        (stdout, _) => {
            let _ = child.kill();
            let _ = child.wait();
            let missing = if stdout.is_none() { "stdout" } else { "stderr" };
            Err(std::io::Error::other(format!("{missing} was not piped")))
        }
    }
}

pub(crate) fn kill_child(child: &Arc<Mutex<Child>>) {
    match child.lock() {
        Ok(mut child) => {
            let _ = child.kill();
        }
        Err(poisoned) => {
            let _ = poisoned.into_inner().kill();
        }
    }
}

pub(crate) fn with_locked_log<F, R>(
    on_log: &Arc<Mutex<F>>,
    context: &str,
    body: impl FnOnce(&mut F) -> R,
) -> R
where
    F: FnMut(&str),
{
    let mut log = on_log.lock().unwrap_or_else(|e| {
        log::error!("Mutex poisoned while acquiring on_log for {context}: {e:?}");
        e.into_inner()
    });
    body(&mut *log)
}

pub(crate) fn run_hook_with_locked_log<F>(
    on_log: &Arc<Mutex<F>>,
    label: &str,
    script: &str,
    trust: HookScriptTrust,
    project_root: &Path,
    fail_on_error: bool,
) -> Result<(), std::io::Error>
where
    F: FnMut(&str),
{
    with_locked_log(on_log, label, |log| {
        run_hook_script(label, script, trust, project_root, log, fail_on_error)
    })
}

pub(crate) fn spawn_stderr_collector<F, R, H>(
    stderr_handle: R,
    on_log: Arc<Mutex<F>>,
    mut handle_line: H,
) -> std::thread::JoinHandle<String>
where
    F: FnMut(&str) + Send + 'static,
    R: std::io::Read + Send + 'static,
    H: FnMut(&str, &mut F) + Send + 'static,
{
    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(stderr_handle);
        let mut full_stderr = String::new();
        // Both the per-line and the total size are bounded: a child that dumps
        // megabytes of stderr (or one enormous newline-less line) must not be
        // able to grow this worker's memory without limit.
        for line in bounded_lines(reader) {
            match line {
                Ok(line) => {
                    push_capped(&mut full_stderr, &line, MAX_STDERR_BYTES);
                    push_capped(&mut full_stderr, "\n", MAX_STDERR_BYTES);
                    with_locked_log(&on_log, "stderr", |log| handle_line(&line, log));
                }
                Err(e) => {
                    let msg = format!("stderr read error: {e}\n");
                    with_locked_log(&on_log, "stderr error", |log| log(&msg));
                    push_capped(&mut full_stderr, &msg, MAX_STDERR_BYTES);
                    break;
                }
            }
        }
        full_stderr
    })
}

/// Spawn a thread that silently drains stderr into a string, so the child's
/// pipe buffer cannot fill and stall the process. Used by backends that log
/// stderr only after the run completes (Claude headless).
pub(crate) fn spawn_stderr_to_string<R>(stderr_handle: R) -> std::thread::JoinHandle<String>
where
    R: std::io::Read + Send + 'static,
{
    std::thread::spawn(move || {
        use std::io::Read;
        let mut raw = Vec::new();
        // `.take()` bounds the read: the pipe still drains (the reader below
        // keeps consuming past the cap), but only the first MAX_STDERR_BYTES
        // are retained.
        let mut reader = std::io::BufReader::new(stderr_handle);
        let _ = std::io::Read::by_ref(&mut reader)
            .take(MAX_STDERR_BYTES as u64)
            .read_to_end(&mut raw);
        let mut buf = String::from_utf8_lossy(&raw).into_owned();
        // Drain whatever is left so the child never stalls on a full pipe.
        let mut discarded = 0u64;
        if let Ok(n) = std::io::copy(&mut reader, &mut std::io::sink()) {
            discarded = n;
        }
        if discarded > 0 {
            buf.push_str(&format!(
                "\n[stderr truncated by Dirigent — {discarded} further bytes discarded]\n"
            ));
        }
        buf
    })
}

pub(crate) fn log_empty_result_stderr<F>(on_log: &Arc<Mutex<F>>, final_result: &str, stderr: &str)
where
    F: FnMut(&str),
{
    if final_result.is_empty() && !stderr.is_empty() {
        with_locked_log(on_log, "empty-result error", |log| {
            log(&format!("\nError: {}\n", stderr));
        });
    }
}

pub(crate) fn is_file_related_tool(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "write"
            | "edit"
            | "bash"
            | "execute_bash"
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

pub(crate) fn extract_file_path_from_input(input: &serde_json::Value) -> Option<String> {
    input
        .get("file_path")
        .or_else(|| input.get("path"))
        .or_else(|| input.get("file"))
        .and_then(|p| p.as_str())
        .map(|s| s.to_string())
}

pub(crate) fn build_tool_detail(input: &serde_json::Value, include_pattern: bool) -> String {
    if let Some(file_path) = input.get("file_path").and_then(|p| p.as_str()) {
        format!(" {}", file_path)
    } else if let Some(command) = input.get("command").and_then(|c| c.as_str()) {
        format!(" $ {}", command.lines().next().unwrap_or(""))
    } else if include_pattern {
        input
            .get("pattern")
            .and_then(|p| p.as_str())
            .map(|pattern| format!(" \"{}\"", pattern))
            .unwrap_or_default()
    } else {
        String::new()
    }
}

/// The one pre/post-run hook policy, shared by all four backends.
///
/// Hook scripts come from `.Dirigent/settings.json`, which a repository can
/// ship. `settings::trust` already gates every execution-capable field behind
/// an explicit per-repository approval, and [`HookScriptTrust`] carries that
/// decision here:
///
/// * [`HookScriptTrust::ProjectLocal`] — the script comes from settings the
///   user has not approved. It is skipped with a visible warning and is
///   **never** fatal, even for the pre-run hook: refusing to execute an
///   unapproved script must not also disable the backend (which is exactly
///   what Codex's old per-backend policy did, silently aborting every run
///   whenever any hook was configured).
/// * [`HookScriptTrust::Trusted`] — the user approved this configuration, so
///   the script runs with the shell semantics the feature documents.
///
/// Before this, the same configuration got two different treatments depending
/// on the backend: `sh -c` for Claude/Gemini/OpenCode, blanket refusal for
/// Codex.
pub(crate) fn run_hook_script(
    label: &str,
    script: &str,
    trust: HookScriptTrust,
    project_root: &Path,
    on_log: &mut dyn FnMut(&str),
    fail_on_error: bool,
) -> Result<(), std::io::Error> {
    if script.trim().is_empty() {
        return Ok(());
    }
    if trust != HookScriptTrust::Trusted {
        let msg = format!(
            "{label} skipped: hook scripts from unapproved project-local settings are not executed"
        );
        log::warn!("[hook] {msg}: {:?}", script.trim());
        on_log(&format!("\u{26A0} {msg}\n"));
        return Ok(());
    }
    on_log(&format!("\u{25B6} {}: {}\n", label, script.trim()));
    let result = std::process::Command::new("sh")
        .arg("-c")
        .arg(script.trim())
        .current_dir(project_root)
        .output();
    match result {
        Ok(output) => handle_hook_output(label, output, on_log, fail_on_error),
        Err(e) => handle_hook_spawn_error(label, e, on_log, fail_on_error),
    }
}

fn handle_hook_output(
    label: &str,
    output: std::process::Output,
    on_log: &mut dyn FnMut(&str),
    fail_on_error: bool,
) -> Result<(), std::io::Error> {
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
            return Err(std::io::Error::other(msg));
        }
    }
    Ok(())
}

fn handle_hook_spawn_error(
    label: &str,
    error: std::io::Error,
    on_log: &mut dyn FnMut(&str),
    fail_on_error: bool,
) -> Result<(), std::io::Error> {
    on_log(&format!("\u{2717} {} script error: {}\n", label, error));
    if fail_on_error {
        Err(error)
    } else {
        Ok(())
    }
}

pub(crate) fn stream_json_payload(line: &str) -> Option<&str> {
    if let Some(data) = line.strip_prefix("data:") {
        let payload = data.trim();
        (!payload.is_empty()).then_some(payload)
    } else if should_skip_stream_line(line) {
        None
    } else {
        let payload = line.trim();
        (!payload.is_empty()).then_some(payload)
    }
}

fn should_skip_stream_line(line: &str) -> bool {
    line.is_empty()
        || line.starts_with("event:")
        || line.starts_with("id:")
        || line.starts_with("retry:")
        || line.starts_with(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonblocking_hook_failure_is_logged() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut log = String::new();

        let result = run_hook_script(
            "post-run",
            "exit 7",
            HookScriptTrust::Trusted,
            temp_dir.path(),
            &mut |text| {
                log.push_str(text);
            },
            false,
        );

        assert!(result.is_ok());
        assert!(log.contains("post-run script failed"));
    }

    #[test]
    fn blocking_hook_failure_returns_error() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let mut log = String::new();

        let result = run_hook_script(
            "pre-run",
            "exit 7",
            HookScriptTrust::Trusted,
            temp_dir.path(),
            &mut |text| {
                log.push_str(text);
            },
            true,
        );

        assert!(result.is_err());
        assert!(log.contains("pre-run script failed"));
    }

    // -- hook script trust (one policy for all four backends) --

    /// An unapproved script must not execute — and must not abort the run
    /// either. The old Codex-only policy returned `HookRejected` from the
    /// pre-run hook, which disabled the whole backend whenever any hook was
    /// configured.
    #[test]
    fn untrusted_pre_run_hook_is_skipped_without_failing_the_run() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let marker = temp_dir.path().join("marker");
        let mut log = String::new();

        let result = run_hook_script(
            "pre-run",
            &format!("touch {}", marker.display()),
            HookScriptTrust::ProjectLocal,
            temp_dir.path(),
            &mut |text| {
                log.push_str(text);
            },
            true,
        );

        assert!(result.is_ok(), "an unapproved hook must not abort the run");
        assert!(!marker.exists(), "the script must not have been executed");
        assert!(log.contains("pre-run skipped"), "log was: {log}");
    }

    #[test]
    fn untrusted_post_run_hook_is_skipped() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let marker = temp_dir.path().join("marker");
        let mut log = String::new();

        run_hook_script(
            "post-run",
            &format!("touch {}", marker.display()),
            HookScriptTrust::ProjectLocal,
            temp_dir.path(),
            &mut |text| {
                log.push_str(text);
            },
            false,
        )
        .expect("skipping is never an error");

        assert!(!marker.exists());
        assert!(log.contains("post-run skipped"), "log was: {log}");
    }

    /// An approved script runs — the case Codex used to refuse outright.
    #[test]
    fn trusted_hook_executes() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let marker = temp_dir.path().join("marker");
        let mut log = String::new();

        run_hook_script(
            "pre-run",
            &format!("touch {}", marker.display()),
            HookScriptTrust::Trusted,
            temp_dir.path(),
            &mut |text| {
                log.push_str(text);
            },
            true,
        )
        .expect("trusted hook should run");

        assert!(marker.exists(), "trusted hook did not execute; log: {log}");
    }

    // -- bounded line reading --

    #[test]
    fn bounded_lines_reads_normal_lines() {
        let input = b"alpha\nbeta\r\ngamma".to_vec();
        let lines: Vec<String> = bounded_lines(std::io::Cursor::new(input))
            .map(|l| l.expect("line"))
            .collect();
        assert_eq!(lines, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn bounded_lines_on_empty_input_yields_nothing() {
        let lines: Vec<String> = bounded_lines(std::io::Cursor::new(Vec::new()))
            .map(|l| l.expect("line"))
            .collect();
        assert!(lines.is_empty());
    }

    #[test]
    fn bounded_lines_preserves_empty_lines() {
        let lines: Vec<String> = bounded_lines(std::io::Cursor::new(b"a\n\nb\n".to_vec()))
            .map(|l| l.expect("line"))
            .collect();
        assert_eq!(lines, vec!["a", "", "b"]);
    }

    /// A newline-less blob must not grow the buffer without bound: it is
    /// truncated at the cap, marked, and the rest of the line is discarded so
    /// the next real line still parses.
    #[test]
    fn bounded_lines_truncates_an_oversized_line() {
        let mut input = vec![b'x'; MAX_LINE_BYTES * 2];
        input.push(b'\n');
        input.extend_from_slice(b"next\n");

        let lines: Vec<String> = bounded_lines(std::io::Cursor::new(input))
            .map(|l| l.expect("line"))
            .collect();

        assert_eq!(lines.len(), 2, "the following line must still be readable");
        assert!(lines[0].starts_with("xxxx"));
        assert!(
            lines[0].ends_with(LINE_TRUNCATED_MARKER),
            "truncation must be visible, not silent"
        );
        assert!(
            lines[0].len() <= MAX_LINE_BYTES + LINE_TRUNCATED_MARKER.len(),
            "line kept {} bytes, cap is {MAX_LINE_BYTES}",
            lines[0].len()
        );
        assert_eq!(lines[1], "next");
    }

    #[test]
    fn push_capped_stops_at_the_limit_and_says_so() {
        let mut buf = String::new();
        assert!(push_capped(&mut buf, "hello", 32));
        assert!(!push_capped(&mut buf, &"y".repeat(100), 32));
        assert!(buf.len() < 100, "buffer grew past the cap: {}", buf.len());
        assert!(buf.contains("truncated by Dirigent"));
        // Once full, further writes are dropped entirely.
        let len_before = buf.len();
        assert!(!push_capped(&mut buf, "more", 32));
        assert_eq!(buf.len(), len_before);
    }

    /// Multi-byte characters must never be split mid-codepoint by the cap.
    #[test]
    fn push_capped_respects_char_boundaries() {
        let mut buf = String::new();
        push_capped(&mut buf, &"é".repeat(50), 15);
        assert!(buf.starts_with('é'));
    }
}
