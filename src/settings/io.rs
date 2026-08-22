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
    /// Set when the settings file existed but could not be used, so every
    /// preference silently fell back to its default. The caller surfaces this
    /// in the status bar — the failure mode it describes (all configuration
    /// gone, no explanation) is otherwise indistinguishable from a bug.
    pub warning: Option<String>,
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
    let (file_value, mut warning) = read_settings_file(&path);
    let check = trust::evaluate(project_root, file_value.as_ref());

    if check.trusted {
        let (mut settings, deser_warning) = settings_from_value(file_value, &path);
        warning = warning.or(deser_warning);
        finalize_settings(project_root, &mut settings);
        if normalize_lsp_env_vars(&mut settings) {
            save_settings_after_env_migration(project_root, &settings);
        }
        return SettingsLoad {
            settings,
            pending_trust: None,
            warning,
        };
    }

    // Untrusted repository: honor only the repo-scoped safe fields (theme,
    // fonts, layout, ...). Every execution-capable field falls back to its
    // default until the user approves it, and `--dangerously-skip-permissions`
    // is forced off. The untrusted file is never written back to disk.
    let safe_value = file_value.as_ref().map(strip_gated_fields);
    let (mut settings, deser_warning) = settings_from_value(safe_value, &path);
    warning = warning.or(deser_warning);
    finalize_settings(project_root, &mut settings);
    normalize_lsp_env_vars(&mut settings);
    settings.allow_dangerous_skip_permissions = false;
    // Belt and braces: the hook-script fields are gated, so `strip_gated_fields`
    // has already blanked them — but marking the whole configuration untrusted
    // means the backends refuse to execute hooks even if a future field escapes
    // the strip.
    settings.hooks_trusted = false;

    // Same file, same error — the warning was already recorded above.
    let (mut trusted_settings, _) = settings_from_value(file_value, &path);
    finalize_settings(project_root, &mut trusted_settings);
    normalize_lsp_env_vars(&mut trusted_settings);

    SettingsLoad {
        settings,
        pending_trust: Some(PendingTrust {
            hash: check.hash,
            fields: check.fields,
            trusted_settings,
        }),
        warning,
    }
}

/// Read and parse `settings.json`, returning the parsed value plus a warning
/// when the file exists but is unusable.
///
/// An unparseable file is renamed to `settings.json.bak-<timestamp>` before
/// defaults take over. Without that, the next `save_settings` overwrites the
/// broken file with defaults and whatever the user had configured — possibly a
/// hand-edited file with one stray comma — is gone for good.
fn read_settings_file(path: &Path) -> (Option<serde_json::Value>, Option<String>) {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (None, None),
        Err(e) => {
            let msg = format!(
                "Could not read {}: {e}. Using default settings.",
                filename(path)
            );
            log::warn!("[settings] {msg}");
            return (None, Some(msg));
        }
    };
    match serde_json::from_str(&contents) {
        Ok(value) => (Some(value), None),
        Err(e) => {
            let backup = backup_broken_settings(path);
            let msg = match backup {
                Some(name) => format!(
                    "{} is not valid JSON ({e}). It was saved as {name} and default settings are in use.",
                    filename(path)
                ),
                None => format!(
                    "{} is not valid JSON ({e}). Default settings are in use — fix or remove the file to keep your configuration.",
                    filename(path)
                ),
            };
            log::warn!("[settings] {msg}");
            (None, Some(msg))
        }
    }
}

/// Rename the unusable settings file out of the way. Returns the backup file
/// name on success.
fn backup_broken_settings(path: &Path) -> Option<String> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let backup = path.with_file_name(format!("{}.bak-{stamp}", filename(path)));
    match std::fs::rename(path, &backup) {
        Ok(()) => Some(filename(&backup)),
        Err(e) => {
            log::warn!("[settings] could not back up {}: {e}", path.display());
            None
        }
    }
}

fn filename(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Deserialize `Settings` from an already-parsed JSON value.
///
/// Valid JSON that does not fit the schema (a string where a number belongs,
/// say) fails the *whole* deserialize, so this reports rather than silently
/// resetting. The file is left alone here — it is valid JSON the user can fix.
fn settings_from_value(
    value: Option<serde_json::Value>,
    path: &Path,
) -> (Settings, Option<String>) {
    let Some(value) = value else {
        return (Settings::default(), None);
    };
    match serde_json::from_value(value) {
        Ok(settings) => (settings, None),
        Err(e) => {
            let msg = format!(
                "{} could not be applied ({e}). Default settings are in use.",
                filename(path)
            );
            log::warn!("[settings] {msg}");
            (Settings::default(), Some(msg))
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write_settings(dir: &Path, contents: &str) -> std::path::PathBuf {
        let d = dir.join(".Dirigent");
        std::fs::create_dir_all(&d).expect("create .Dirigent");
        let path = d.join("settings.json");
        std::fs::write(&path, contents).expect("write settings");
        path
    }

    #[test]
    fn missing_settings_file_is_not_a_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (value, warning) = read_settings_file(&dir.path().join("settings.json"));
        assert!(value.is_none());
        assert!(warning.is_none(), "unexpected warning: {warning:?}");
    }

    #[test]
    fn corrupt_settings_file_is_backed_up_and_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_settings(dir.path(), "{ \"theme\": \"Dracula\", }}} not json");

        let (value, warning) = read_settings_file(&path);

        assert!(value.is_none());
        let warning = warning.expect("corrupt settings must produce a warning");
        assert!(warning.contains("not valid JSON"), "got: {warning}");
        assert!(
            !path.exists(),
            "the corrupt file must be moved aside, not left to be overwritten"
        );

        let backups: Vec<_> = std::fs::read_dir(path.parent().expect("parent"))
            .expect("read dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("settings.json.bak-"))
            .collect();
        assert_eq!(backups.len(), 1, "expected exactly one backup: {backups:?}");
    }

    #[test]
    fn valid_settings_file_parses_without_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_settings(dir.path(), r#"{"font_size": 15.0}"#);
        let (value, warning) = read_settings_file(&path);
        assert!(value.is_some());
        assert!(warning.is_none(), "unexpected warning: {warning:?}");
        assert!(path.exists(), "a valid file must be left alone");
    }

    /// Valid JSON that does not fit the schema is reported rather than
    /// silently resetting every preference — but the file is left in place,
    /// since the user can fix it by hand.
    #[test]
    fn schema_mismatch_is_reported_without_backup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_settings(dir.path(), r#"{"font_size": "not a number"}"#);
        let (value, read_warning) = read_settings_file(&path);
        assert!(read_warning.is_none(), "JSON itself is valid");

        let (settings, warning) = settings_from_value(value, &path);
        let warning = warning.expect("schema mismatch must produce a warning");
        assert!(warning.contains("could not be applied"), "got: {warning}");
        assert_eq!(settings.font_size, Settings::default().font_size);
        assert!(path.exists(), "valid JSON must not be moved aside");
    }
}
