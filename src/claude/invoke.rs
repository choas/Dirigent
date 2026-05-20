use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use claude_pty::ClaudeCode;

use super::cli::{
    apply_dirigent_env, apply_env_vars, load_dirigent_env_pairs, resolve_env_pairs,
    run_lifecycle_script,
};
use super::stream::consume_pty_events;
use super::types::{ClaudeError, ClaudeResponse};

/// Invoke the Claude Code CLI, send `prompt`, and stream live progress to
/// `on_log`. When `use_pty` is true (default), the interactive TUI is launched
/// under a PTY via `claude_pty`; the trust-folder dialog is auto-accepted and
/// the prompt is sent on the first `❯` indicator. When `use_pty` is false,
/// Claude is invoked in headless `-p <prompt>` mode with piped stdout/stderr.
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
    use_pty: bool,
    mut on_log: impl FnMut(&str),
    cancel: Arc<AtomicBool>,
) -> Result<ClaudeResponse, ClaudeError> {
    run_lifecycle_script(pre_run_script, "pre-run", project_root, &mut on_log, true)?;

    let result = if use_pty {
        invoke_pty(
            prompt,
            project_root,
            model,
            cli_path,
            extra_args,
            env_vars,
            skip_permissions,
            &mut on_log,
            cancel.clone(),
        )
    } else {
        invoke_headless(
            prompt,
            project_root,
            model,
            cli_path,
            extra_args,
            env_vars,
            skip_permissions,
            &mut on_log,
            cancel.clone(),
        )
    };

    run_lifecycle_script(
        post_run_script,
        "post-run",
        project_root,
        &mut on_log,
        false,
    )?;

    if cancel.load(Ordering::Relaxed) {
        return Err(ClaudeError::Cancelled);
    }

    result
}

/// PTY path: launch Claude's interactive TUI via `claude_pty` and drive it.
#[allow(clippy::too_many_arguments)]
fn invoke_pty(
    prompt: &str,
    project_root: &Path,
    model: &str,
    cli_path: &str,
    extra_args: &str,
    env_vars: &str,
    skip_permissions: bool,
    on_log: &mut dyn FnMut(&str),
    cancel: Arc<AtomicBool>,
) -> Result<ClaudeResponse, ClaudeError> {
    let mut builder = ClaudeCode::builder().cwd(project_root);
    if !cli_path.is_empty() {
        builder = builder.binary(cli_path);
    }
    if !model.is_empty() {
        builder = builder.model(model);
    }
    if !extra_args.is_empty() {
        let args = shlex::split(extra_args).unwrap_or_else(|| {
            extra_args.split_whitespace().map(String::from).collect()
        });
        builder = builder.extra_args(args);
    }
    if skip_permissions {
        builder = builder.permission_mode(claude_pty::PermissionMode::BypassPermissions);
    }
    // Pin Claude's TUI to basic 16-color ANSI so diff lines emit standard
    // 31/32 (and 91/92) codes that `ansi::DiffAnsiOverrides` can remap to
    // the user's Settings-page diff colors. Without this, chalk/Ink detects
    // the PTY as truecolor-capable and emits 24-bit RGB for diffs, which
    // the override doesn't intercept. User env (CLI/.Dirigent/.env) is
    // appended after, so an explicit `FORCE_COLOR=…` still wins.
    let mut envs: Vec<(String, String)> =
        vec![("FORCE_COLOR".to_string(), "1".to_string())];
    envs.extend(resolve_env_pairs(env_vars));
    envs.extend(load_dirigent_env_pairs(project_root));
    builder = builder.envs(envs);

    let mut session = builder.open().map_err(|e| match e {
        claude_pty::Error::BinaryNotFound => ClaudeError::NotFound,
        claude_pty::Error::Spawn(msg) | claude_pty::Error::Io(msg) => {
            ClaudeError::SpawnFailed(std::io::Error::other(msg))
        }
    })?;

    let state = consume_pty_events(&mut session, prompt, &cancel, on_log);

    Ok(ClaudeResponse {
        stdout: state.response,
        metrics: Default::default(),
    })
}

/// Headless path: spawn `claude -p <prompt>` with piped stdout/stderr,
/// forward both streams to `on_log`, and accumulate stdout as the response.
#[allow(clippy::too_many_arguments)]
fn invoke_headless(
    prompt: &str,
    project_root: &Path,
    model: &str,
    cli_path: &str,
    extra_args: &str,
    env_vars: &str,
    skip_permissions: bool,
    on_log: &mut dyn FnMut(&str),
    cancel: Arc<AtomicBool>,
) -> Result<ClaudeResponse, ClaudeError> {
    let claude_bin = if cli_path.is_empty() { "claude" } else { cli_path };
    which::which(claude_bin).map_err(|_| ClaudeError::NotFound)?;

    let mut cmd = Command::new(claude_bin);
    cmd.arg("-p").arg(prompt);
    if skip_permissions {
        cmd.arg("--dangerously-skip-permissions");
    }
    if !model.is_empty() {
        cmd.arg("--model").arg(model);
    }
    if !extra_args.is_empty() {
        let args = shlex::split(extra_args).unwrap_or_else(|| {
            extra_args.split_whitespace().map(String::from).collect()
        });
        for arg in args {
            if !arg.is_empty() {
                cmd.arg(arg);
            }
        }
    }
    apply_env_vars(&mut cmd, env_vars);
    apply_dirigent_env(&mut cmd, project_root);

    let mut child = cmd
        .current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(ClaudeError::SpawnFailed)?;

    let stdout_handle = child.stdout.take().expect("stdout must be piped");
    let stderr_handle = child.stderr.take().expect("stderr must be piped");
    let child = Arc::new(Mutex::new(child));

    let done = Arc::new(AtomicBool::new(false));
    let watchdog = spawn_cancel_watchdog(child.clone(), cancel.clone(), done.clone());
    let stderr_thread = spawn_stderr_collector(stderr_handle);

    let response = read_stdout_to_log(stdout_handle, &cancel, on_log);

    done.store(true, Ordering::Relaxed);
    let _ = watchdog.join();
    reap_child(&child);
    let stderr = stderr_thread.join().unwrap_or_default();
    if !stderr.is_empty() {
        on_log(&stderr);
    }

    Ok(ClaudeResponse {
        stdout: response,
        metrics: Default::default(),
    })
}

/// Read stdout line-by-line, forward to `on_log`, and accumulate the response.
fn read_stdout_to_log(
    stdout: impl Read,
    cancel: &AtomicBool,
    on_log: &mut dyn FnMut(&str),
) -> String {
    let mut response = String::new();
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        match line {
            Ok(text) => {
                on_log(&text);
                on_log("\n");
                response.push_str(&text);
                response.push('\n');
            }
            Err(_) => break,
        }
    }
    response
}

/// Spawn a thread that drains stderr into a string, so the child's pipe
/// buffer cannot fill and stall the process.
fn spawn_stderr_collector(stderr: impl Read + Send + 'static) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut buf = String::new();
        let mut reader = BufReader::new(stderr);
        let _ = reader.read_to_string(&mut buf);
        buf
    })
}

/// Spawn a watchdog that kills the child when `cancel` flips, and exits
/// when `done` is set so the foreground reader can finish naturally.
fn spawn_cancel_watchdog(
    child: Arc<Mutex<std::process::Child>>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || loop {
        if done.load(Ordering::Relaxed) {
            return;
        }
        if cancel.load(Ordering::Relaxed) {
            if let Ok(mut c) = child.lock() {
                let _ = c.kill();
            }
            return;
        }
        thread::sleep(Duration::from_millis(100));
    })
}

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
