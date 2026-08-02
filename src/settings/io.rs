use std::path::Path;

use super::app_settings::Settings;
use super::commands::default_commands;
use super::playbook::default_playbook;
use super::trust;

/// Resolve the full path for a CLI tool.
///
/// macOS `.app` bundles inherit a minimal PATH, so a plain
/// `which` won't find tools installed via Homebrew, npm, etc.  We therefore:
///   1. Try a login-shell `which` to pick up the user's full PATH.
///   2. Fall back to a plain `which` (works when launched from a terminal).
///   3. Probe well-known installation directories as a last resort.
///
/// `name` may be a bare command name (`claude`) or an absolute path. For an
/// absolute path, the login/plain `which` lookups verify it is still present
/// and executable, so a stale stored path resolves to `None` instead of being
/// blindly spawned.
pub(crate) fn resolve_in_path(name: &str) -> Option<String> {
    // Reject names with shell metacharacters to prevent command injection.
    // `name` may originate from deserialized settings.json which could be
    // attacker-controlled (e.g. a malicious .Dirigent/settings.json in a repo).
    //
    // Absolute / path-like names (containing '/') are filesystem paths and may
    // legitimately contain spaces (e.g. "/Applications/Claude Code.app/...").
    // Spaces are not a command-injection vector on their own, so we permit them
    // for paths while keeping the stricter allowlist for bare command names.
    let is_path = name.contains('/');
    if name.is_empty()
        || !name.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/') || (is_path && c == ' ')
        })
    {
        return None;
    }

    // 1. Login shell
    let login = std::process::Command::new("/bin/zsh")
        .args(["-l", "-c", &format!("which {name}")])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());
    if login.is_some() {
        return login;
    }

    // 2. Plain which (limited PATH, but works from terminal launches).
    let plain = std::process::Command::new("which")
        .arg(name)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());
    if plain.is_some() {
        return plain;
    }

    // 3. Well-known paths (Homebrew, npm global, user-local).
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        format!("/opt/homebrew/bin/{name}"),
        format!("/usr/local/bin/{name}"),
        format!("{home}/.local/bin/{name}"),
        format!("{home}/.npm-global/bin/{name}"),
        format!("{home}/.nvm/current/bin/{name}"),
    ];
    for p in &candidates {
        if std::path::Path::new(p).is_file() {
            return Some(p.clone());
        }
    }

    None
}

/// Validate `agent_shell_init` against common dangerous patterns.
/// The field is intentionally shell code (e.g. `source ~/.nvm/nvm.sh`), so we
/// cannot restrict by character set.  Instead we reject known dangerous commands
/// that have no legitimate place in an init snippet.
fn validate_shell_init(init: &str) -> bool {
    if init.is_empty() {
        return true;
    }
    let lower = init.to_ascii_lowercase();
    // Reject network exfiltration / code download patterns.
    let blocked = [
        "curl ",
        "curl\t",
        "wget ",
        "wget\t",
        "nc ",
        "nc\t",
        "ncat ",
        "python -c",
        "python3 -c",
        "ruby -e",
        "perl -e",
        "php -r",
        "bash -i",
        "zsh -i",
        "/dev/tcp/",
        "/dev/udp/",
        "mkfifo",
        "telnet ",
        "xterm ",
    ];
    for pat in &blocked {
        if lower.contains(pat) {
            return false;
        }
    }
    true
}

/// Result of [`load_settings`]: the settings to apply now, plus — when the
/// repository's file carries execution-capable fields the user has not yet
/// approved — a pending trust request for the approval dialog.
pub(crate) struct SettingsLoad {
    pub settings: Settings,
    pub pending_trust: Option<PendingTrust>,
}

/// An unapproved execution-capable field set from `.Dirigent/settings.json`.
pub(crate) struct PendingTrust {
    /// Content hash of the execution-capable fields awaiting approval.
    pub hash: String,
    /// (field name, pretty-printed JSON value) pairs shown in the approval dialog.
    pub fields: Vec<(String, String)>,
    /// The settings as they would apply once the user trusts the repository.
    pub trusted_settings: Settings,
}

pub(crate) fn load_settings(project_root: &Path) -> SettingsLoad {
    let path = project_root.join(".Dirigent").join("settings.json");
    let file_value: Option<serde_json::Value> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok());
    let check = trust::evaluate(project_root, file_value.as_ref());

    if check.trusted {
        let mut settings = settings_from_value(file_value);
        finalize_settings(project_root, &mut settings);
        if normalize_lsp_env_vars(&mut settings) {
            save_settings_after_env_migration(project_root, &settings);
        }
        return SettingsLoad {
            settings,
            pending_trust: None,
        };
    }

    // Untrusted repository: honor only the repo-scoped safe fields (theme,
    // fonts, layout, ...). Every execution-capable field falls back to its
    // default until the user approves it, and `--dangerously-skip-permissions`
    // is forced off. The untrusted file is never written back to disk.
    let safe_value = file_value.as_ref().map(strip_gated_fields);
    let mut settings = settings_from_value(safe_value);
    finalize_settings(project_root, &mut settings);
    normalize_lsp_env_vars(&mut settings);
    settings.allow_dangerous_skip_permissions = false;

    let mut trusted_settings = settings_from_value(file_value);
    finalize_settings(project_root, &mut trusted_settings);
    normalize_lsp_env_vars(&mut trusted_settings);

    SettingsLoad {
        settings,
        pending_trust: Some(PendingTrust {
            hash: check.hash,
            fields: check.fields,
            trusted_settings,
        }),
    }
}

fn settings_from_value(value: Option<serde_json::Value>) -> Settings {
    value
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn strip_gated_fields(value: &serde_json::Value) -> serde_json::Value {
    match value.as_object() {
        Some(obj) => serde_json::Value::Object(
            obj.iter()
                .filter(|(k, _)| trust::is_repo_safe_field(k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
        None => serde_json::Value::Object(Default::default()),
    }
}

fn finalize_settings(project_root: &Path, settings: &mut Settings) {
    sanitize_shell_init(settings);
    detect_cli_paths(settings);
    detect_vcs_backend(project_root, settings);
    merge_default_playbook(settings);
    merge_default_commands(settings);
}

fn sanitize_shell_init(settings: &mut Settings) {
    if !validate_shell_init(&settings.agent_shell_init) {
        log::warn!(
            "[settings] rejecting agent_shell_init with blocked command pattern: {:?}",
            settings.agent_shell_init
        );
        settings.agent_shell_init = String::new();
    }
}

fn detect_cli_paths(settings: &mut Settings) {
    fill_cli_path(&mut settings.claude_cli_path, "claude");
    fill_cli_path(&mut settings.opencode_cli_path, "opencode");
    fill_cli_path(&mut settings.codex_cli_path, "codex");
    fill_cli_path(&mut settings.jj_cli_path, "jj");
}

fn fill_cli_path(slot: &mut String, command: &str) {
    if slot.is_empty() {
        *slot = resolve_in_path(command).unwrap_or_default();
    }
}

fn detect_vcs_backend(project_root: &Path, settings: &mut Settings) {
    if project_root.join(".jj").is_dir() {
        settings.vcs_backend = crate::settings::VcsBackend::Jj;
    }
}

fn merge_default_playbook(settings: &mut Settings) {
    for default_play in default_playbook() {
        if !settings
            .playbook
            .iter()
            .any(|p| p.name == default_play.name)
        {
            settings.playbook.push(default_play);
        }
    }
}

fn merge_default_commands(settings: &mut Settings) {
    for default_cmd in default_commands() {
        if !settings.commands.iter().any(|c| c.name == default_cmd.name) {
            settings.commands.push(default_cmd);
        }
    }
}

fn normalize_lsp_env_vars(settings: &mut Settings) -> bool {
    let mut env_changed = false;
    for server in &mut settings.lsp_servers {
        if server.env.iter().any(|s| s.contains('\n')) {
            server.env = server
                .env
                .iter()
                .flat_map(|s| s.split('\n'))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            env_changed = true;
        }
    }
    env_changed
}

fn save_settings_after_env_migration(project_root: &Path, settings: &Settings) {
    if let Err(e) = save_settings(project_root, settings) {
        log::warn!("Failed to save settings after env migration: {e}");
    }
}

pub(crate) fn save_settings(project_root: &Path, settings: &Settings) -> Result<(), String> {
    let dir = project_root.join(".Dirigent");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create settings dir: {}", e))?;
    let path = dir.join("settings.json");
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Cannot serialize settings: {}", e))?;
    crate::util::fs::atomic_write(&path, json.as_bytes())
        .map_err(|e| format!("Cannot write settings: {}", e))?;
    // Everything the app saves has already passed the trust gate (or was
    // edited by the user in the UI), so record its execution-capable field
    // hash — otherwise the app's own writes would re-trigger the trust prompt.
    if let Ok(value) = serde_json::to_value(settings) {
        trust::record_settings_state(project_root, &value);
    }
    Ok(())
}
