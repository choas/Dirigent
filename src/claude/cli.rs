use std::path::Path;
use std::process::Command;

use super::types::ClaudeError;

/// Resolve the Claude binary path and verify it exists on PATH.
pub(super) fn resolve_claude_binary(cli_path: &str) -> Result<&str, ClaudeError> {
    let claude_bin = if cli_path.is_empty() {
        "claude"
    } else {
        cli_path
    };
    let which_result = Command::new("which").arg(claude_bin).output();
    match which_result {
        Ok(output) if !output.status.success() => Err(ClaudeError::NotFound),
        Err(_) => Err(ClaudeError::NotFound),
        _ => Ok(claude_bin),
    }
}

/// Build the `Command` with prompt, flags, extra args, and env vars.
pub(super) fn build_claude_command(
    claude_bin: &str,
    prompt: &str,
    model: &str,
    extra_args: &str,
    env_vars: &str,
) -> Command {
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
    append_extra_args(&mut cmd, extra_args);
    apply_env_vars(&mut cmd, env_vars);
    cmd
}

/// Append whitespace-separated extra arguments to the command.
fn append_extra_args(cmd: &mut Command, extra_args: &str) {
    for arg in extra_args.split_whitespace() {
        if !arg.is_empty() {
            cmd.arg(arg);
        }
    }
}

/// Apply KEY=VALUE environment variables (one per line, # comments allowed).
fn apply_env_vars(cmd: &mut Command, env_vars: &str) {
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
}

/// Run a lifecycle script (pre-run or post-run).
///
/// Returns `Err` for pre-run failures (abort the run), logs but ignores
/// post-run failures when `fail_on_error` is false.
pub(super) fn run_lifecycle_script(
    script: &str,
    label: &str,
    project_root: &Path,
    on_log: &mut dyn FnMut(&str),
    fail_on_error: bool,
) -> Result<(), ClaudeError> {
    let trimmed = script.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    on_log(&format!("\u{25B6} {}: {}\n", label, trimmed));
    match Command::new("sh")
        .arg("-c")
        .arg(trimmed)
        .current_dir(project_root)
        .output()
    {
        Ok(output) => handle_script_output(&output, label, on_log, fail_on_error),
        Err(e) => handle_script_error(e, label, on_log, fail_on_error),
    }
}

/// Process successful script execution output.
fn handle_script_output(
    output: &std::process::Output,
    label: &str,
    on_log: &mut dyn FnMut(&str),
    fail_on_error: bool,
) -> Result<(), ClaudeError> {
    if !output.stdout.is_empty() {
        on_log(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        on_log(&String::from_utf8_lossy(&output.stderr));
    }
    if output.status.success() {
        return Ok(());
    }
    let msg = format!("{} script failed (exit {})", label, output.status);
    on_log(&format!("\u{2717} {}\n", msg));
    if fail_on_error {
        return Err(ClaudeError::SpawnFailed(std::io::Error::other(msg)));
    }
    Ok(())
}

/// Handle a script spawn error.
fn handle_script_error(
    e: std::io::Error,
    label: &str,
    on_log: &mut dyn FnMut(&str),
    fail_on_error: bool,
) -> Result<(), ClaudeError> {
    on_log(&format!("\u{2717} {} script error: {}\n", label, e));
    if fail_on_error {
        return Err(ClaudeError::SpawnFailed(e));
    }
    Ok(())
}
