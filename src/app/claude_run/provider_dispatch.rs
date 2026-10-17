//! Turning a cue plus the user's settings into one provider invocation:
//! `[command]` prefix resolution, prompt assembly, and the per-provider
//! model / CLI path / args / hook-script configuration.

use super::mcp_inject::{dirigent_mcp_args, resolve_dirigent_db, should_inject_dirigent_mcp};
use crate::claude;
use crate::settings::{self, CliProvider, CueCommand};

/// Configuration extracted from Settings for a specific CLI provider.
pub(super) struct ProviderConfig {
    pub(super) model: String,
    pub(super) cli_path: String,
    /// The `herdr` binary; only the Herdr provider uses it (its `cli_path` is
    /// the Claude CLI launched inside the pane).
    pub(super) herdr_cli_path: String,
    pub(super) extra_args: String,
    /// Structured args that must not go through shlex::split (e.g. --mcp-config
    /// paths). Passed directly as individual Command::arg entries.
    pub(super) extra_args_vec: Vec<String>,
    pub(super) env_vars: String,
    pub(super) pre_run_script: String,
    pub(super) post_run_script: String,
    /// Trust level for the two scripts above, derived from whether the user
    /// approved this repository's execution-capable settings.
    pub(super) hook_trust: crate::ai_backend::HookScriptTrust,
    pub(super) skip_permissions: bool,
    pub(super) use_pty: bool,
}

/// Resolve a `[command]` prefix from cue text, returning the effective prompt text
/// and the matched command config (if any).
pub(super) fn resolve_command_prefix(
    cue_text: &str,
    commands: &[CueCommand],
) -> (String, Option<CueCommand>) {
    let parsed = claude::parse_command_prefix(cue_text);
    let (cmd_name, rest) = match parsed {
        Some((name, rest)) => (name, rest),
        None => return (cue_text.to_string(), None),
    };
    match commands.iter().find(|c| c.name == cmd_name) {
        Some(cmd) => {
            let expanded = cmd.prompt.replace("{task}", rest);
            (expanded, Some(cmd.clone()))
        }
        None => (cue_text.to_string(), None),
    }
}

/// For PR-sourced cues, append a hint so the AI can fetch more context.
/// The hint is added at prompt time (not stored in cue text) to avoid
/// triggering spurious text-changed updates on existing cues.
pub(super) fn build_pr_hint_text(text: &str, source_ref: Option<&str>) -> String {
    if let Some(sref) = source_ref
        && let Some(pr_num) = sref
            .strip_prefix("pr")
            .and_then(|s| s.split(':').next())
            .and_then(|n| n.parse::<u64>().ok())
    {
        return format!(
            "{}\n\n[Hint: use `gh pr view {} --comments` to read the full PR discussion for additional context.]",
            text, pr_num,
        );
    }
    text.to_string()
}

/// Build provider-specific configuration from settings, with optional command overrides.
/// When the prompt mentions "Dirigent" and the provider is Claude, the Dirigent MCP
/// server is conditionally injected via `--mcp-config` so Claude gains cue-management tools.
pub(super) fn build_provider_config(
    settings: &settings::Settings,
    provider: &CliProvider,
    matched_command: &Option<CueCommand>,
    prompt: &str,
    project_root: &std::path::Path,
) -> ProviderConfig {
    let pf = settings.provider_fields(provider);
    let model = pf.model.to_string();
    let cli_path = pf.cli_path.to_string();
    let mut extra_args = pf.extra_args.to_string();
    let env_vars = pf.env_vars.to_string();
    let pre_run_script = matched_command
        .as_ref()
        .filter(|cmd| !cmd.pre_agent.is_empty())
        .map(|cmd| cmd.pre_agent.clone())
        .unwrap_or_else(|| pf.pre_run_script.to_string());
    let post_run_script = matched_command
        .as_ref()
        .filter(|cmd| !cmd.post_agent.is_empty())
        .map(|cmd| cmd.post_agent.clone())
        .unwrap_or_else(|| pf.post_run_script.to_string());
    append_command_cli_args(&mut extra_args, matched_command);
    let extra_args_vec = provider_extra_args_vec(settings, provider, prompt, project_root);
    ProviderConfig {
        model,
        cli_path,
        herdr_cli_path: settings.herdr_cli_path.clone(),
        extra_args,
        extra_args_vec,
        env_vars,
        pre_run_script,
        post_run_script,
        hook_trust: settings.hook_trust(),
        skip_permissions: settings.allow_dangerous_skip_permissions,
        use_pty: settings.claude_use_pty,
    }
}

fn append_command_cli_args(extra_args: &mut String, matched_command: &Option<CueCommand>) {
    let Some(cmd) = matched_command.as_ref() else {
        return;
    };
    let cmd_args = cmd.cli_args.trim();
    if cmd_args.is_empty() {
        return;
    }
    if extra_args.is_empty() {
        *extra_args = cmd_args.to_string();
    } else {
        *extra_args = format!("{} {}", extra_args, cmd_args);
    }
}

pub(super) fn provider_extra_args_vec(
    settings: &settings::Settings,
    provider: &CliProvider,
    prompt: &str,
    project_root: &std::path::Path,
) -> Vec<String> {
    if *provider != CliProvider::Claude || !should_inject_dirigent_mcp(prompt) {
        return Vec::new();
    }
    let Some(db_path) = resolve_dirigent_db(project_root, settings) else {
        return Vec::new();
    };
    dirigent_mcp_args(project_root, settings, &db_path)
}
