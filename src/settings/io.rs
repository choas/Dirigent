use std::path::Path;

use super::app_settings::Settings;
use super::commands::default_commands;
use super::playbook::default_playbook;

/// Resolve the full path for a CLI tool.
///
/// macOS `.app` bundles inherit a minimal PATH, so a plain
/// `which` won't find tools installed via Homebrew, npm, etc.  We therefore:
///   1. Try a login-shell `which` to pick up the user's full PATH.
///   2. Fall back to a plain `which` (works when launched from a terminal).
///   3. Probe well-known installation directories as a last resort.
fn which(name: &str) -> Option<String> {
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

pub(crate) fn load_settings(project_root: &Path) -> Settings {
    let path = project_root.join(".Dirigent").join("settings.json");
    let mut settings = match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Settings::default(),
    };
    // Auto-detect CLI paths on first launch (when paths are empty)
    if settings.claude_cli_path.is_empty() {
        if let Some(path) = which("claude") {
            settings.claude_cli_path = path;
        }
    }
    if settings.opencode_cli_path.is_empty() {
        if let Some(path) = which("opencode") {
            settings.opencode_cli_path = path;
        }
    }
    // Append any new default plays that aren't already in the user's playbook
    for default_play in default_playbook() {
        if !settings
            .playbook
            .iter()
            .any(|p| p.name == default_play.name)
        {
            settings.playbook.push(default_play);
        }
    }
    // Append any new default commands that aren't already defined
    for default_cmd in default_commands() {
        if !settings.commands.iter().any(|c| c.name == default_cmd.name) {
            settings.commands.push(default_cmd);
        }
    }
    // Normalize LSP env vars: flatten any entries with embedded newlines
    // (legacy corruption from join/split mismatch).
    for server in &mut settings.lsp_servers {
        if server.env.iter().any(|s| s.contains('\n')) {
            server.env = server
                .env
                .iter()
                .flat_map(|s| s.split('\n'))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    settings
}

pub(crate) fn save_settings(project_root: &Path, settings: &Settings) {
    let dir = project_root.join(".Dirigent");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("settings.json");
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(path, json);
    }
}
