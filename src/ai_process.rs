use std::path::Path;
use std::process::{Child, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const WATCHDOG_POLL_INTERVAL: Duration = Duration::from_millis(200);

pub(crate) fn spawn_cancel_watchdog(
    child: &Arc<Mutex<Child>>,
    cancel: &Arc<AtomicBool>,
    done: &Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    let child = Arc::clone(child);
    let cancel = Arc::clone(cancel);
    let done = Arc::clone(done);
    std::thread::spawn(move || {
        while !done.load(Ordering::Relaxed) {
            if cancel.load(Ordering::Relaxed) {
                kill_child(&child);
                return;
            }
            std::thread::sleep(WATCHDOG_POLL_INTERVAL);
        }
    })
}

pub(crate) fn wait_for_child(child: &Arc<Mutex<Child>>) -> std::io::Result<ExitStatus> {
    match child.lock() {
        Ok(mut child) => child.wait(),
        Err(poisoned) => poisoned.into_inner().wait(),
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
    project_root: &Path,
    fail_on_error: bool,
) -> Result<(), std::io::Error>
where
    F: FnMut(&str),
{
    with_locked_log(on_log, label, |log| {
        run_hook_script(label, script, project_root, log, fail_on_error)
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
        use std::io::BufRead;

        let reader = std::io::BufReader::new(stderr_handle);
        let mut full_stderr = String::new();
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    full_stderr.push_str(&line);
                    full_stderr.push('\n');
                    with_locked_log(&on_log, "stderr", |log| handle_line(&line, log));
                }
                Err(e) => {
                    let msg = format!("stderr read error: {e}\n");
                    with_locked_log(&on_log, "stderr error", |log| log(&msg));
                    full_stderr.push_str(&msg);
                    break;
                }
            }
        }
        full_stderr
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

pub(crate) fn run_hook_script(
    label: &str,
    script: &str,
    project_root: &Path,
    on_log: &mut dyn FnMut(&str),
    fail_on_error: bool,
) -> Result<(), std::io::Error> {
    if script.trim().is_empty() {
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
            temp_dir.path(),
            &mut |text| {
                log.push_str(text);
            },
            true,
        );

        assert!(result.is_err());
        assert!(log.contains("pre-run script failed"));
    }
}
