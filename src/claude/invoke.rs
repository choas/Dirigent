use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use claude_pty::ClaudeCode;

use super::cli::{load_dirigent_env_pairs, resolve_env_pairs, run_lifecycle_script};
use super::stream::consume_pty_events;
use super::types::{ClaudeError, ClaudeResponse};

/// Invoke the Claude Code TUI under a PTY, send `prompt`, and stream live
/// progress to `on_log`. Returns the accumulated response text.
///
/// The interactive TUI is launched (no `--print` / `--output-format`), the
/// trust-folder dialog is auto-accepted, and the prompt is sent on the first
/// `❯` indicator. Screen output is forwarded to `on_log` as it arrives.
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
    run_lifecycle_script(pre_run_script, "pre-run", project_root, &mut on_log, true)?;

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

    let state = consume_pty_events(&mut session, prompt, &cancel, &mut on_log);

    run_lifecycle_script(
        post_run_script,
        "post-run",
        project_root,
        &mut on_log,
        false,
    )?;

    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(ClaudeError::Cancelled);
    }

    Ok(ClaudeResponse {
        stdout: state.response,
        metrics: Default::default(),
    })
}
