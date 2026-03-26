use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::cli::{build_claude_command, resolve_claude_binary, run_lifecycle_script};
use super::stream::{read_stream_events, spawn_stderr_reader, spawn_watchdog};
use super::types::{ClaudeError, ClaudeResponse};

/// Reap the child process (works whether it exited naturally or was killed).
fn reap_child(child: &Arc<Mutex<std::process::Child>>) {
    match child.lock() {
        Ok(mut c) => {
            let _ = c.wait();
        }
        Err(poisoned) => {
            let _ = poisoned.into_inner().wait();
        }
    }
}

/// Invoke `claude -p <prompt> --output-format stream-json` with live progress
/// streaming to a shared log buffer. Parses JSON events from stdout in real-time.
///
/// The `cancel` token allows the caller to abort the run: a watchdog thread
/// monitors the flag and kills the child process when it is set.
#[allow(clippy::too_many_arguments)]
pub(crate) fn invoke_claude_streaming(
    prompt: &str,
    project_root: &Path,
    model: &str,
    cli_path: &str,
    extra_args: &str,
    env_vars: &str,
    pre_run_script: &str,
    post_run_script: &str,
    skip_permissions: bool,
    mut on_log: impl FnMut(&str),
    cancel: Arc<AtomicBool>,
) -> Result<ClaudeResponse, ClaudeError> {
    use std::process::Stdio;

    let claude_bin = resolve_claude_binary(cli_path)?;
    let mut cmd = build_claude_command(
        claude_bin,
        prompt,
        model,
        extra_args,
        env_vars,
        skip_permissions,
    );

    run_lifecycle_script(pre_run_script, "pre-run", project_root, &mut on_log, true)?;

    let mut child = cmd
        .current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(ClaudeError::SpawnFailed)?;

    let stderr_handle = child.stderr.take().expect("stderr must be piped");
    let stdout_handle = child.stdout.take().expect("stdout must be piped");
    let child = Arc::new(Mutex::new(child));

    let (done, watchdog) = spawn_watchdog(&child, &cancel);
    let stderr_thread = spawn_stderr_reader(stderr_handle);
    let state = read_stream_events(stdout_handle, &cancel, &mut on_log);

    done.store(true, Ordering::Relaxed);
    let _ = watchdog.join();
    reap_child(&child);
    let stderr = stderr_thread.join().unwrap_or_default();

    if cancel.load(Ordering::Relaxed) {
        return Err(ClaudeError::Cancelled);
    }

    if state.final_result.is_empty() && !stderr.is_empty() {
        on_log(&format!("\nError: {}\n", stderr));
    }

    run_lifecycle_script(
        post_run_script,
        "post-run",
        project_root,
        &mut on_log,
        false,
    )?;

    Ok(ClaudeResponse {
        stdout: state.final_result,
        edited_files: state.edited_files,
        metrics: state.metrics,
    })
}
