use std::io::{BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use claude_pty::ClaudeCode;

use super::cli::{
    apply_dirigent_env, apply_env_vars, load_dirigent_env_pairs, resolve_env_pairs_with,
};
use super::done_hook::DoneHook;
use super::stream::consume_pty_events;
use crate::ai_backend::{BackendError, BackendRequest, BackendResponse};
use crate::ai_process;

/// Invoke the Claude Code CLI, send the prompt, and stream live progress to
/// `on_log`. When `req.use_pty` is true (default), the interactive TUI is
/// launched under a PTY via `claude_pty`; the trust-folder dialog is
/// auto-accepted and the prompt is sent on the first `❯` indicator. When
/// false, Claude is invoked in headless `-p <prompt>` mode with piped
/// stdout/stderr.
///
/// Pre/post-run hooks and the final cancel mapping are owned by the
/// `AiBackend` driver — the pre-run hook runs before this function, so it can
/// still rewrite `.Dirigent/.env` before we read it.
pub(crate) fn run_claude_cli(
    req: &BackendRequest<'_>,
    on_log: &mut dyn FnMut(&str),
    cancel: Arc<AtomicBool>,
) -> Result<BackendResponse, BackendError> {
    if req.use_pty {
        invoke_pty(req, on_log, cancel)
    } else {
        invoke_headless(req, on_log, cancel)
    }
}

/// Draft a commit message from `diff` by invoking the Claude CLI headlessly.
///
/// This is the fallback path for the Commit dialog's "Generate" button when the
/// Fast LLM is not configured: it runs `claude -p <prompt>` (no PTY, no
/// lifecycle scripts, no tools), captures stdout, and extracts the message from
/// the `<commit-message>` block the prompt asks Claude to emit. Returns a
/// human-readable error string on failure so the dialog can surface it.
pub(crate) fn summarize_commit_message_via_cli(
    diff: &str,
    project_root: &Path,
    model: &str,
    cli_path: &str,
    extra_args: &str,
    env_vars: &str,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    // Cap the diff so a huge change set doesn't blow past the model's context.
    let trimmed: String = diff.chars().take(12_000).collect();
    let prompt = format!(
        "Summarize the following git diff as a commit message. Start with a \
         concise subject line in imperative mood (no more than 72 characters), \
         then a blank line, then a short body of 2-6 lines (plain sentences or \
         bullet points) explaining what changed and why. Wrap the commit message \
         in <commit-message> and </commit-message> tags and output nothing \
         else.\n\n{trimmed}"
    );

    let req = BackendRequest {
        prompt: &prompt,
        project_root,
        model,
        cli_path,
        extra_args,
        extra_args_vec: &[],
        env_vars,
        pre_run_script: "",
        post_run_script: "",
        // No hook scripts to run, so the trust level is immaterial.
        hook_trust: crate::ai_backend::HookScriptTrust::Trusted,
        skip_permissions: false,
        use_pty: false,
        read_only: true,
    };
    let response = run_claude_cli(&req, &mut |_| {}, cancel.clone())
        .map_err(|e| e.to_string())?
        .stdout;
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".to_string());
    }

    // Prefer the tagged block; fall back to the first non-empty output line so a
    // model that ignores the tag instruction still yields something usable.
    let message = super::prompt::extract_commit_message(&response).or_else(|| {
        response
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(|l| l.trim_matches('"').to_string())
    });
    match message {
        Some(m) if !m.trim().is_empty() => Ok(m.trim().to_string()),
        _ => Err("CLI returned an empty commit message".to_string()),
    }
}

/// PTY path: launch Claude's interactive TUI via `claude_pty` and drive it.
fn invoke_pty(
    req: &BackendRequest<'_>,
    on_log: &mut dyn FnMut(&str),
    cancel: Arc<AtomicBool>,
) -> Result<BackendResponse, BackendError> {
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
        .cwd(req.project_root)
        .pty_size(PTY_ROWS, PTY_COLS);
    builder = builder.binary(resolve_claude_binary(req.cli_path)?);
    if !req.model.is_empty() {
        builder = builder.model(req.model);
    }
    let mut all_args: Vec<String> = Vec::new();
    if !req.extra_args.is_empty() {
        all_args = shlex::split(req.extra_args).unwrap_or_else(|| {
            req.extra_args
                .split_whitespace()
                .map(String::from)
                .collect()
        });
    }
    all_args.extend_from_slice(req.extra_args_vec);
    // Read the session id from the *full* argv: `--session-id`/`--resume` may
    // arrive via the parsed `extra_args` string, not just `extra_args_vec`. If
    // we missed it here, Claude would run session-scoped while the hook fell
    // back to an unconditional command, reopening the concurrent-run overlap.
    let session_id = extract_session_id(&all_args);
    if !all_args.is_empty() {
        builder = builder.extra_args(all_args);
    }
    if req.skip_permissions {
        builder = builder.permission_mode(claude_pty::PermissionMode::BypassPermissions);
    }
    // Compose envs in precedence order — see `compose_pty_envs` for the full
    // contract. The vec is consumed by `builder.envs(...)`, which forwards
    // every entry to `portable_pty::CommandBuilder::env`; that call inserts
    // into a BTreeMap keyed by name, so later entries OVERWRITE earlier ones.
    // The headless path achieves the same precedence via `cmd.env(...)` on
    // `std::process::Command`, which also overwrites — see `invoke_headless`.
    builder = builder.envs(compose_pty_envs(req.env_vars, req.project_root, on_log));

    // The PTY path still detects the end of a turn without the hook (idle
    // heuristics), so a failed install is a warning here, not an error.
    let done_hook = match DoneHook::install(req.project_root, session_id.as_deref()) {
        Ok(hook) => {
            on_log("⏎ Stop hook installed\n");
            Some(hook)
        }
        Err(e) => {
            on_log(&format!("\u{26A0} Stop hook not installed: {e:#}\n"));
            None
        }
    };

    let mut session = builder.open().map_err(|e| match e {
        claude_pty::Error::BinaryNotFound => BackendError::NotFound { backend: "claude" },
        claude_pty::Error::Spawn(msg) | claude_pty::Error::Io(msg) => BackendError::SpawnFailed {
            backend: "claude",
            source: std::io::Error::other(msg),
        },
    })?;
    let state = consume_pty_events(
        &mut session,
        req.prompt,
        &cancel,
        on_log,
        super::stream::PtyConsumeOptions {
            done_hook: done_hook.as_ref(),
            session_id: session_id.as_deref(),
            skip_permissions: req.skip_permissions,
            // Auto-accepting Claude Code's trust-folder dialog activates the
            // repository's own .claude/settings.json (hooks, MCP servers,
            // permissions). Only do that for a repository the user explicitly
            // trusted in Dirigent — checked here rather than inferred, because
            // a freshly cloned repo with no .Dirigent/settings.json is *not*
            // trusted.
            repo_trusted: crate::settings::repo_settings_trusted(req.project_root),
        },
    );
    drop(done_hook);

    // The TUI reports no cost or turn count, so those stay `None` — Dirigent
    // records "unknown", never a fabricated $0.00. Duration comes from the last
    // observed output rather than the driver's wall clock, which would also
    // include the shutdown drain.
    let duration_ms = state.metadata.last_output_ms;
    Ok(BackendResponse {
        stdout: state.response,
        duration_ms,
        metadata: Some(state.metadata),
        ..BackendResponse::default()
    })
}

/// Extract the Claude session id from the full CLI argv.
///
/// Either `--session-id <uuid>` (fresh run) or `--resume <uuid>` (continuation)
/// may appear, regardless of whether it came from the parsed `extra_args`
/// string or `extra_args_vec`; this returns the value following whichever flag
/// appears first so the hook and live log see the same session as the PTY.
fn extract_session_id(args: &[String]) -> Option<String> {
    let mut iter = args.iter();
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
/// return [`BackendError::NotFound`], which surfaces a clear "configure path
/// in Settings" message instead of a cryptic spawn ENOENT.
fn resolve_claude_binary(cli_path: &str) -> Result<String, BackendError> {
    if !cli_path.is_empty()
        && let Some(path) = crate::settings::resolve_in_path(cli_path)
    {
        return Ok(path);
    }
    crate::settings::resolve_in_path("claude").ok_or(BackendError::NotFound { backend: "claude" })
}

/// Headless path: spawn `claude -p <prompt>` with piped stdout/stderr,
/// forward both streams to `on_log`, and accumulate stdout as the response.
fn invoke_headless(
    req: &BackendRequest<'_>,
    on_log: &mut dyn FnMut(&str),
    cancel: Arc<AtomicBool>,
) -> Result<BackendResponse, BackendError> {
    let claude_bin = resolve_claude_binary(req.cli_path)?;

    let mut cmd = Command::new(&claude_bin);
    cmd.arg("-p").arg(req.prompt);
    if req.skip_permissions {
        cmd.arg("--dangerously-skip-permissions");
    }
    if !req.model.is_empty() {
        cmd.arg("--model").arg(req.model);
    }
    if !req.extra_args.is_empty() {
        let args = shlex::split(req.extra_args).unwrap_or_else(|| {
            req.extra_args
                .split_whitespace()
                .map(String::from)
                .collect()
        });
        for arg in args {
            if !arg.is_empty() {
                cmd.arg(arg);
            }
        }
    }
    for arg in req.extra_args_vec {
        if !arg.is_empty() {
            cmd.arg(arg);
        }
    }
    // Env precedence (must match `compose_pty_envs` for the PTY path so a
    // secret rotated in `.Dirigent/.env` cannot silently behave differently
    // between modes): Settings `env_vars` first, then `.Dirigent/.env`.
    // `Command::env` overwrites on duplicate keys, so `.Dirigent/.env` wins.
    apply_env_vars(&mut cmd, req.env_vars, on_log);
    apply_dirigent_env(&mut cmd, req.project_root, on_log);

    let mut child = cmd
        .current_dir(req.project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| BackendError::SpawnFailed {
            backend: "claude",
            source,
        })?;

    let (stdout_handle, stderr_handle) =
        ai_process::take_child_streams(&mut child).map_err(|source| BackendError::SpawnFailed {
            backend: "claude",
            source,
        })?;
    let child = Arc::new(Mutex::new(child));

    let done = Arc::new(AtomicBool::new(false));
    let watchdog = ai_process::spawn_cancel_watchdog(&child, &cancel, &done);
    let stderr_thread = ai_process::spawn_stderr_to_string(stderr_handle);

    let response = read_stdout_to_log(stdout_handle, &cancel, on_log);

    done.store(true, Ordering::Relaxed);
    let _ = watchdog.join();
    let _ = ai_process::wait_for_child(&child);
    let stderr = stderr_thread.join().unwrap_or_default();
    if !stderr.is_empty() {
        on_log(&stderr);
    }

    Ok(BackendResponse {
        stdout: response,
        ..BackendResponse::default()
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
    for line in crate::ai_process::bounded_lines(reader) {
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
    compose_pty_envs_with(env_vars, project_root, on_log, |name| {
        std::env::var(name).ok()
    })
}

/// [`compose_pty_envs`] with an injectable lookup for the Settings `env_vars`
/// names, so tests don't have to mutate the process environment.
fn compose_pty_envs_with(
    env_vars: &str,
    project_root: &Path,
    on_log: &mut dyn FnMut(&str),
    lookup: impl Fn(&str) -> Option<String>,
) -> Vec<(String, String)> {
    let mut envs: Vec<(String, String)> = vec![("FORCE_COLOR".to_string(), "1".to_string())];
    envs.extend(resolve_env_pairs_with(env_vars, on_log, lookup));
    envs.extend(load_dirigent_env_pairs(project_root, on_log));
    envs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::cli::{apply_env_vars_with, fake_env};
    use std::collections::HashMap;
    use std::ffi::{OsStr, OsString};

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

        let process_env = [("DIRIGENT_PRECEDENCE_KEY", "from_process_env")];
        let settings_env_vars = "DIRIGENT_PRECEDENCE_KEY\n";

        let mut headless_cmd = Command::new("true");
        apply_env_vars_with(
            &mut headless_cmd,
            settings_env_vars,
            &mut |_| {},
            fake_env(&process_env),
        );
        apply_dirigent_env(&mut headless_cmd, project_root, &mut |_| {});
        let headless = cmd_envs(&headless_cmd);

        let pty_pairs = compose_pty_envs_with(
            settings_env_vars,
            project_root,
            &mut |_| {},
            fake_env(&process_env),
        );
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

        let process_env = [("DIRIGENT_SETTINGS_ONLY_KEY", "settings_value")];
        let settings_env_vars = "DIRIGENT_SETTINGS_ONLY_KEY\n";

        let mut headless_cmd = Command::new("true");
        apply_env_vars_with(
            &mut headless_cmd,
            settings_env_vars,
            &mut |_| {},
            fake_env(&process_env),
        );
        apply_dirigent_env(&mut headless_cmd, project_root, &mut |_| {});
        let headless = cmd_envs(&headless_cmd);

        let pty = effective_envs(&compose_pty_envs_with(
            settings_env_vars,
            project_root,
            &mut |_| {},
            fake_env(&process_env),
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
