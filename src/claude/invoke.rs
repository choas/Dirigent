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
use super::done_hook::DoneHook;
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
    extra_args_vec: &[String],
    env_vars: &str,
    pre_run_script: &str,
    post_run_script: &str,
    skip_permissions: bool,
    use_pty: bool,
    mut on_log: impl FnMut(&str),
    cancel: Arc<AtomicBool>,
) -> Result<ClaudeResponse, ClaudeError> {
    // Run pre-run script first so it can modify .Dirigent/.env before we read it.
    run_lifecycle_script(pre_run_script, "pre-run", project_root, &mut on_log, true)?;

    let result = if use_pty {
        invoke_pty(
            prompt,
            project_root,
            model,
            cli_path,
            extra_args,
            extra_args_vec,
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
            extra_args_vec,
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
    extra_args_vec: &[String],
    env_vars: &str,
    skip_permissions: bool,
    on_log: &mut dyn FnMut(&str),
    cancel: Arc<AtomicBool>,
) -> Result<ClaudeResponse, ClaudeError> {
    // Give the PTY a wide, tall terminal. Claude's TUI lays its output out to
    // the terminal width: at the library's default 120 cols a wide multi-column
    // table no longer fits, so the renderer collapses every column to its header
    // width and drops the cell contents — the user sees an empty bordered table
    // with no rows. A real terminal (via `claude --resume`) is far wider and
    // renders the same table in full. Match that here so wide tables and other
    // width-sensitive layouts survive capture. egui re-wraps prose to the panel
    // width on display, so the extra width only benefits fixed-width layouts.
    const PTY_ROWS: u16 = 50;
    const PTY_COLS: u16 = 220;
    let mut builder = ClaudeCode::builder()
        .cwd(project_root)
        .pty_size(PTY_ROWS, PTY_COLS);
    builder = builder.binary(resolve_claude_binary(cli_path)?);
    if !model.is_empty() {
        builder = builder.model(model);
    }
    let mut all_args: Vec<String> = Vec::new();
    if !extra_args.is_empty() {
        all_args = shlex::split(extra_args)
            .unwrap_or_else(|| extra_args.split_whitespace().map(String::from).collect());
    }
    all_args.extend_from_slice(extra_args_vec);
    if !all_args.is_empty() {
        builder = builder.extra_args(all_args);
    }
    if skip_permissions {
        builder = builder.permission_mode(claude_pty::PermissionMode::BypassPermissions);
    }
    // Compose envs in precedence order — see `compose_pty_envs` for the full
    // contract. The vec is consumed by `builder.envs(...)`, which forwards
    // every entry to `portable_pty::CommandBuilder::env`; that call inserts
    // into a BTreeMap keyed by name, so later entries OVERWRITE earlier ones.
    // The headless path achieves the same precedence via `cmd.env(...)` on
    // `std::process::Command`, which also overwrites — see `invoke_headless`.
    builder = builder.envs(compose_pty_envs(env_vars, project_root, on_log));

    let done_hook = DoneHook::install(project_root);
    if done_hook.is_some() {
        on_log("⏎ Stop hook installed\n");
    }

    let mut session = builder.open().map_err(|e| match e {
        claude_pty::Error::BinaryNotFound => ClaudeError::NotFound,
        claude_pty::Error::Spawn(msg) | claude_pty::Error::Io(msg) => {
            ClaudeError::SpawnFailed(std::io::Error::other(msg))
        }
    })?;

    let sentinel = done_hook.as_ref().map(|h| h.sentinel_path());
    let session_id = extract_session_id(extra_args_vec);
    let state = consume_pty_events(
        &mut session,
        prompt,
        &cancel,
        on_log,
        sentinel,
        session_id.as_deref(),
    );
    drop(done_hook);

    Ok(ClaudeResponse {
        stdout: state.response,
        metrics: Default::default(),
    })
}

/// Extract the Claude session id from the extra CLI args.
///
/// The caller injects either `--session-id <uuid>` (fresh run) or
/// `--resume <uuid>` (continuation) into `extra_args_vec`; this returns the
/// value following whichever flag appears so it can be surfaced in the live
/// log when the run completes.
fn extract_session_id(extra_args_vec: &[String]) -> Option<String> {
    let mut iter = extra_args_vec.iter();
    while let Some(arg) = iter.next() {
        if arg == "--session-id" || arg == "--resume" {
            return iter.next().filter(|v| !v.is_empty()).cloned();
        }
    }
    None
}

/// Resolve the Claude binary to an absolute, currently-existing executable.
///
/// A configured `cli_path` is tried first, but if it is stale (e.g. the binary
/// was moved or removed since it was auto-detected) we fall back to resolving
/// `claude` from PATH so the run still works. Only when nothing resolves do we
/// return [`ClaudeError::NotFound`], which surfaces a clear "configure path in
/// Settings" message instead of a cryptic spawn ENOENT.
fn resolve_claude_binary(cli_path: &str) -> Result<String, ClaudeError> {
    if !cli_path.is_empty() {
        if let Some(path) = crate::settings::resolve_in_path(cli_path) {
            return Ok(path);
        }
    }
    crate::settings::resolve_in_path("claude").ok_or(ClaudeError::NotFound)
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
    extra_args_vec: &[String],
    env_vars: &str,
    skip_permissions: bool,
    on_log: &mut dyn FnMut(&str),
    cancel: Arc<AtomicBool>,
) -> Result<ClaudeResponse, ClaudeError> {
    let claude_bin = resolve_claude_binary(cli_path)?;

    let mut cmd = Command::new(&claude_bin);
    cmd.arg("-p").arg(prompt);
    if skip_permissions {
        cmd.arg("--dangerously-skip-permissions");
    }
    if !model.is_empty() {
        cmd.arg("--model").arg(model);
    }
    if !extra_args.is_empty() {
        let args = shlex::split(extra_args)
            .unwrap_or_else(|| extra_args.split_whitespace().map(String::from).collect());
        for arg in args {
            if !arg.is_empty() {
                cmd.arg(arg);
            }
        }
    }
    for arg in extra_args_vec {
        if !arg.is_empty() {
            cmd.arg(arg);
        }
    }
    // Env precedence (must match `compose_pty_envs` for the PTY path so a
    // secret rotated in `.Dirigent/.env` cannot silently behave differently
    // between modes): Settings `env_vars` first, then `.Dirigent/.env`.
    // `Command::env` overwrites on duplicate keys, so `.Dirigent/.env` wins.
    apply_env_vars(&mut cmd, env_vars, on_log);
    apply_dirigent_env(&mut cmd, project_root, on_log);

    let mut child = cmd
        .current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(ClaudeError::SpawnFailed)?;

    let stdout_handle = child
        .stdout
        .take()
        .ok_or_else(|| ClaudeError::SpawnFailed(std::io::Error::other("stdout was not piped")))?;
    let stderr_handle = child
        .stderr
        .take()
        .ok_or_else(|| ClaudeError::SpawnFailed(std::io::Error::other("stderr was not piped")))?;
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
    // Check `cancel` before `done`: if cancellation stopped the stdout reader
    // and the main thread set `done` before this watchdog woke from sleep,
    // checking `done` first would let it exit without killing, leaving
    // `reap_child` to wait on a still-running process.
    thread::spawn(move || loop {
        if cancel.load(Ordering::Relaxed) {
            if let Ok(mut c) = child.lock() {
                let _ = c.kill();
            }
            return;
        }
        if done.load(Ordering::Relaxed) {
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

/// Build the env list passed to the PTY-launched Claude TUI.
///
/// Order matters — `portable_pty::CommandBuilder::env` (used by `claude_pty`
/// when consuming this vec) inserts into a map keyed by variable name, so
/// later entries OVERWRITE earlier ones. The effective precedence is:
///
///   1. `FORCE_COLOR=1` — base default (pins Claude's chalk/Ink to 16-color
///      ANSI so diff lines emit 31/32/91/92 that `ansi::DiffAnsiOverrides`
///      can remap to the Settings-page diff colors).
///   2. Settings `env_vars` (resolved from the parent process environment)
///      — may override `FORCE_COLOR` if the user explicitly forwards it.
///   3. `.Dirigent/.env` — wins over everything above.
///
/// The headless path (`invoke_headless`) applies the same two user-supplied
/// sources in the same order via `std::process::Command::env`, which has the
/// same last-write-wins semantics, so a secret rotated in `.Dirigent/.env`
/// behaves identically on both paths.
fn compose_pty_envs(
    env_vars: &str,
    project_root: &Path,
    on_log: &mut dyn FnMut(&str),
) -> Vec<(String, String)> {
    let mut envs: Vec<(String, String)> = vec![("FORCE_COLOR".to_string(), "1".to_string())];
    envs.extend(resolve_env_pairs(env_vars, on_log));
    envs.extend(load_dirigent_env_pairs(project_root, on_log));
    envs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::{OsStr, OsString};
    use std::sync::{Mutex, MutexGuard};

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    /// RAII guard that sets a process env var on creation and restores the
    /// previous value (or removes it) on drop — even if the test panics.
    /// Holds a process-wide lock to serialize env mutations across parallel tests.
    struct EnvVarGuard {
        key: &'static str,
        prev: Option<OsString>,
        _lock: MutexGuard<'static, ()>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let prev = std::env::var_os(key);
            // SAFETY: test-only; the guard ensures restore-on-drop.
            unsafe { std::env::set_var(key, value) };
            Self {
                key,
                prev,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.prev {
                // SAFETY: test-only restore of the previous value.
                Some(v) => unsafe { std::env::set_var(self.key, v) },
                None => unsafe { std::env::remove_var(self.key) },
            }
            // _lock is dropped after this, releasing the mutex
        }
    }

    /// Collapse a list of `(key, value)` env pairs the same way the underlying
    /// command builders do: insert in order, last-write wins.
    fn effective_envs(pairs: &[(String, String)]) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for (k, v) in pairs {
            map.insert(k.clone(), v.clone());
        }
        map
    }

    /// Snapshot the env vars set on a `std::process::Command` so we can
    /// compare against the PTY composition.
    fn cmd_envs(cmd: &Command) -> HashMap<OsString, OsString> {
        cmd.get_envs()
            .filter_map(|(k, v)| v.map(|v| (k.to_owned(), v.to_owned())))
            .collect()
    }

    /// Regression: a secret rotated in `.Dirigent/.env` MUST win on both the
    /// headless and PTY paths. If precedence diverges, the same prompt could
    /// behave with a stale token under one transport and a fresh token under
    /// the other — a security and ops footgun.
    #[test]
    fn dirigent_env_overrides_settings_env_vars_on_both_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path();
        std::fs::create_dir_all(project_root.join(".Dirigent")).unwrap();
        std::fs::write(
            project_root.join(".Dirigent").join(".env"),
            "DIRIGENT_PRECEDENCE_KEY=from_dirigent_env\n",
        )
        .unwrap();

        let _guard = EnvVarGuard::set("DIRIGENT_PRECEDENCE_KEY", "from_process_env");
        let settings_env_vars = "DIRIGENT_PRECEDENCE_KEY\n";

        let mut headless_cmd = Command::new("true");
        apply_env_vars(&mut headless_cmd, settings_env_vars, &mut |_| {});
        apply_dirigent_env(&mut headless_cmd, project_root, &mut |_| {});
        let headless = cmd_envs(&headless_cmd);

        let pty_pairs = compose_pty_envs(settings_env_vars, project_root, &mut |_| {});
        let pty = effective_envs(&pty_pairs);

        assert_eq!(
            headless.get(OsStr::new("DIRIGENT_PRECEDENCE_KEY")),
            Some(&OsString::from("from_dirigent_env")),
            "headless path: .Dirigent/.env must win over Settings env_vars",
        );
        assert_eq!(
            pty.get("DIRIGENT_PRECEDENCE_KEY"),
            Some(&"from_dirigent_env".to_string()),
            "PTY path: .Dirigent/.env must win over Settings env_vars",
        );
        assert_eq!(
            headless.get(OsStr::new("DIRIGENT_PRECEDENCE_KEY")).cloned(),
            pty.get("DIRIGENT_PRECEDENCE_KEY")
                .cloned()
                .map(OsString::from),
            "headless and PTY paths must agree on the resolved value",
        );
    }

    /// A var that is ONLY in Settings `env_vars` (not in `.Dirigent/.env`)
    /// should be forwarded by both paths with the same value.
    #[test]
    fn settings_only_env_var_forwarded_on_both_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path();
        // No .Dirigent/.env on purpose — Settings should be the only source.

        let _guard = EnvVarGuard::set("DIRIGENT_SETTINGS_ONLY_KEY", "settings_value");
        let settings_env_vars = "DIRIGENT_SETTINGS_ONLY_KEY\n";

        let mut headless_cmd = Command::new("true");
        apply_env_vars(&mut headless_cmd, settings_env_vars, &mut |_| {});
        apply_dirigent_env(&mut headless_cmd, project_root, &mut |_| {});
        let headless = cmd_envs(&headless_cmd);

        let pty = effective_envs(&compose_pty_envs(
            settings_env_vars,
            project_root,
            &mut |_| {},
        ));

        assert_eq!(
            headless.get(OsStr::new("DIRIGENT_SETTINGS_ONLY_KEY")),
            Some(&OsString::from("settings_value")),
        );
        assert_eq!(
            pty.get("DIRIGENT_SETTINGS_ONLY_KEY"),
            Some(&"settings_value".to_string()),
        );
    }

    /// PTY path seeds `FORCE_COLOR=1` so chalk emits 16-color ANSI, but a
    /// user-supplied `FORCE_COLOR` (via Settings env_vars or `.Dirigent/.env`)
    /// MUST be able to override it. This documents the order in
    /// `compose_pty_envs` and pins it against accidental reordering.
    #[test]
    fn user_force_color_overrides_pty_default() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path();
        std::fs::create_dir_all(project_root.join(".Dirigent")).unwrap();
        std::fs::write(
            project_root.join(".Dirigent").join(".env"),
            "FORCE_COLOR=3\n",
        )
        .unwrap();

        let pty = effective_envs(&compose_pty_envs("", project_root, &mut |_| {}));
        assert_eq!(pty.get("FORCE_COLOR"), Some(&"3".to_string()));
    }
}
