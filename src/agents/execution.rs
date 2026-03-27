use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::diagnostics::{parse_cargo_diagnostics, parse_generic_diagnostics, Diagnostic};
use super::types::{AgentConfig, AgentKind, AgentStatus};

// ---------------------------------------------------------------------------
// Agent result (sent from worker thread back to main)
// ---------------------------------------------------------------------------

pub(crate) struct AgentResult {
    pub kind: AgentKind,
    pub cue_id: Option<i64>,
    pub status: AgentStatus,
    pub output: String,
    pub diagnostics: Vec<Diagnostic>,
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Run an agent (called from a worker thread)
// ---------------------------------------------------------------------------

/// Execute a single agent command. This is meant to be called from a spawned
/// thread — it blocks until the command finishes or times out.
///
/// `shell_init` is an optional shell snippet (from settings) prepended to the
/// command so that macOS GUI apps can source profiles, set PATH, JAVA_HOME, etc.
pub(crate) fn run_agent(
    config: &AgentConfig,
    project_root: &Path,
    shell_init: &str,
    cue_id: Option<i64>,
    prompt: &str,
    tx: &mpsc::Sender<AgentResult>,
    cancel: &Arc<AtomicBool>,
) {
    let start = Instant::now();
    let kind = config.kind;
    let timeout = Duration::from_secs(config.timeout_secs);

    let effective_cmd = prepend_shell_init(shell_init, &config.command);

    let cwd = match resolve_working_dir(project_root, &config.working_dir) {
        Ok(p) => p,
        Err(msg) => {
            let _ = tx.send(make_error_result(kind, cue_id, msg, &start));
            return;
        }
    };

    if let Err(msg) = run_before_hook(config, shell_init, prompt, &cwd, cancel) {
        let _ = tx.send(make_error_result(kind, cue_id, msg, &start));
        return;
    }

    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&effective_cmd)
        .current_dir(&cwd)
        .env("PROMPT", prompt)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Inject .Dirigent/.env overrides so agent commands use dev credentials.
    crate::claude::apply_dirigent_env(&mut cmd, project_root);

    // On Unix, create a new process group so we can kill the entire tree on timeout
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Failed to execute command: {}", e);
            let _ = tx.send(make_error_result(kind, cue_id, msg, &start));
            return;
        }
    };

    let result = wait_with_timeout(&mut child, timeout, cancel);
    let duration_ms = start.elapsed().as_millis() as u64;

    let agent_result = match result {
        WaitResult::Completed(output) => build_completed_result(kind, cue_id, &output, duration_ms),
        WaitResult::TimedOut => {
            kill_process_tree(&child);
            let _ = child.kill();
            let _ = child.wait();
            AgentResult {
                kind,
                cue_id,
                status: AgentStatus::Error,
                output: format!(
                    "Agent timed out after {}s (limit: {}s)",
                    duration_ms / 1000,
                    config.timeout_secs
                ),
                diagnostics: Vec::new(),
                duration_ms,
            }
        }
        WaitResult::Cancelled => {
            kill_process_tree(&child);
            let _ = child.kill();
            let _ = child.wait();
            AgentResult {
                kind,
                cue_id,
                status: AgentStatus::Error,
                output: "Cancelled by user".to_string(),
                diagnostics: Vec::new(),
                duration_ms,
            }
        }
    };
    let _ = tx.send(agent_result);
}

/// Prepend an optional shell init snippet to a command string.
fn prepend_shell_init(shell_init: &str, command: &str) -> String {
    if shell_init.trim().is_empty() {
        command.to_string()
    } else {
        format!("{}\n{}", shell_init.trim(), command)
    }
}

/// Resolve and validate the working directory for an agent.
/// Returns `Err(message)` if the path escapes the project root.
fn resolve_working_dir(
    project_root: &Path,
    working_dir: &str,
) -> Result<std::path::PathBuf, String> {
    if working_dir.trim().is_empty() {
        return Ok(project_root.to_path_buf());
    }
    let candidate = project_root.join(working_dir.trim());
    let resolved = candidate.canonicalize().unwrap_or(candidate.clone());
    let root_resolved = project_root
        .canonicalize()
        .unwrap_or(project_root.to_path_buf());
    if !resolved.starts_with(&root_resolved) {
        return Err(format!(
            "working_dir '{}' escapes project root",
            working_dir
        ));
    }
    Ok(candidate)
}

/// Execute the before_run hook if configured. Returns `Err(message)` on failure.
/// Uses spawn + timeout/cancellation logic consistent with the main agent process.
fn run_before_hook(
    config: &AgentConfig,
    shell_init: &str,
    prompt: &str,
    cwd: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    if config.before_run.trim().is_empty() {
        return Ok(());
    }
    let before_effective = prepend_shell_init(shell_init, &config.before_run);
    let timeout = Duration::from_secs(config.timeout_secs);

    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&before_effective)
        .current_dir(cwd)
        .env("PROMPT", prompt)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("before_run failed to execute: {}", e))?;

    let result = wait_with_timeout(&mut child, timeout, cancel);

    match result {
        WaitResult::Completed(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let combined = if stderr.is_empty() { stdout } else { stderr };
            Err(format!(
                "before_run failed (exit {}):\n{}",
                output.status, combined
            ))
        }
        WaitResult::Completed(_) => Ok(()),
        WaitResult::TimedOut => {
            kill_process_tree(&child);
            let _ = child.kill();
            let _ = child.wait();
            Err(format!(
                "before_run timed out after {}s",
                config.timeout_secs
            ))
        }
        WaitResult::Cancelled => {
            kill_process_tree(&child);
            let _ = child.kill();
            let _ = child.wait();
            Err("before_run cancelled by user".to_string())
        }
    }
}

/// Build an error `AgentResult` with the given message.
fn make_error_result(
    kind: AgentKind,
    cue_id: Option<i64>,
    output: String,
    start: &Instant,
) -> AgentResult {
    AgentResult {
        kind,
        cue_id,
        status: AgentStatus::Error,
        output,
        diagnostics: Vec::new(),
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

/// Build the `AgentResult` for a completed (non-timeout, non-cancelled) process.
fn build_completed_result(
    kind: AgentKind,
    cue_id: Option<i64>,
    output: &std::process::Output,
    duration_ms: u64,
) -> AgentResult {
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = if stderr.is_empty() {
        stdout.clone()
    } else if stdout.is_empty() {
        stderr.clone()
    } else {
        format!("{}\n{}", stdout, stderr)
    };

    let status = match kind {
        AgentKind::Lint | AgentKind::Format if output.status.success() => AgentStatus::Passed,
        AgentKind::Lint | AgentKind::Format => AgentStatus::Failed,
        _ if output.status.success() => AgentStatus::Passed,
        _ => AgentStatus::Failed,
    };

    let diagnostics = match kind {
        AgentKind::Lint | AgentKind::Build | AgentKind::Test | AgentKind::Custom(_) => {
            let cargo_diags = parse_cargo_diagnostics(&stdout);
            if cargo_diags.is_empty() {
                parse_generic_diagnostics(&combined)
            } else {
                cargo_diags
            }
        }
        _ => Vec::new(),
    };

    AgentResult {
        kind,
        cue_id,
        status,
        output: combined,
        diagnostics,
        duration_ms,
    }
}

enum WaitResult {
    Completed(std::process::Output),
    TimedOut,
    Cancelled,
}

/// Wait for a child process with a timeout, polling every 100ms.
/// Drains stdout/stderr in background threads to avoid pipe buffer deadlocks.
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
    cancel: &Arc<AtomicBool>,
) -> WaitResult {
    use std::io::Read;

    // Spawn threads to drain stdout/stderr so the pipe buffers don't fill up
    // and block the child process (classic pipe deadlock).
    let stdout_handle = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out.read_to_end(&mut buf);
            buf
        })
    });
    let stderr_handle = child.stderr.take().map(|mut err| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = err.read_to_end(&mut buf);
            buf
        })
    });

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = stdout_handle
                    .and_then(|h| h.join().ok())
                    .unwrap_or_default();
                let stderr = stderr_handle
                    .and_then(|h| h.join().ok())
                    .unwrap_or_default();
                return WaitResult::Completed(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if cancel.load(Ordering::Relaxed) {
                    return WaitResult::Cancelled;
                }
                if start.elapsed() >= timeout {
                    return WaitResult::TimedOut;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => {
                return WaitResult::TimedOut;
            }
        }
    }
}

/// Kill the entire process tree (process group) on Unix, or just the child on other platforms.
fn kill_process_tree(child: &std::process::Child) {
    let pid = child.id();
    #[cfg(unix)]
    {
        // Kill the entire process group (negative PID)
        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
        }
        // Give processes a moment to clean up, then force kill
        std::thread::sleep(Duration::from_millis(500));
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid; // suppress unused warning
                     // On non-Unix, just kill the direct child (best effort)
                     // child.kill() requires &mut, so we can't call it here
    }
}
