use std::process::Child;
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
                let _ = child.lock().map(|mut c| c.kill());
                return;
            }
            std::thread::sleep(WATCHDOG_POLL_INTERVAL);
        }
    })
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
