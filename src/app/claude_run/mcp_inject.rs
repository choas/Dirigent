//! Injecting Dirigent's own MCP server into a run, so the agent can read and
//! move cues while it works.

use crate::settings;

pub(super) fn should_inject_dirigent_mcp(prompt: &str) -> bool {
    prompt.to_lowercase().contains("dirigent")
}

pub(super) fn write_dirigent_mcp_config(
    project_root: &std::path::Path,
    mcp_bin: &str,
    db_path: &std::path::Path,
) -> std::io::Result<std::path::PathBuf> {
    let dir = project_root.join(".Dirigent");
    std::fs::create_dir_all(&dir)?;
    let config_path = dir.join("mcp-config.json");
    let escaped_bin = mcp_bin.replace('\\', "\\\\").replace('"', "\\\"");
    let escaped_db = db_path
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let config = format!(
        r#"{{"mcpServers":{{"dirigent":{{"type":"stdio","command":"{}","args":["{}"]}}}}}}"#,
        escaped_bin, escaped_db,
    );
    crate::util::fs::atomic_write(&config_path, config.as_bytes())?;
    Ok(config_path)
}

pub(super) fn resolve_dirigent_db(
    project_root: &std::path::Path,
    settings: &settings::Settings,
) -> Option<std::path::PathBuf> {
    let local_db = project_root.join(".Dirigent").join("Dirigent.db");
    if local_db.exists() {
        return Some(local_db);
    }
    if !settings.dirigent_mcp_db_path.is_empty() {
        let path = std::path::PathBuf::from(&settings.dirigent_mcp_db_path);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

pub(super) fn dirigent_mcp_args(
    project_root: &std::path::Path,
    settings: &settings::Settings,
    db_path: &std::path::Path,
) -> Vec<String> {
    let mcp_bin = dirigent_mcp_bin(settings);
    if !mcp_bin_exists(&mcp_bin) {
        log::warn!(
            "Dirigent MCP server binary '{}' not found — \
             install with: cd dirigent-mcp-server && npm install && npm run build && npm link",
            mcp_bin,
        );
        return Vec::new();
    }
    match write_dirigent_mcp_config(project_root, &mcp_bin, db_path) {
        Ok(config_path) => {
            log::info!(
                "Dirigent MCP server injected: {} {}",
                mcp_bin,
                db_path.display()
            );
            vec![
                "--mcp-config".to_string(),
                config_path.display().to_string(),
            ]
        }
        Err(e) => {
            log::warn!("Failed to write Dirigent MCP config: {e}");
            Vec::new()
        }
    }
}

pub(super) fn dirigent_mcp_bin(settings: &settings::Settings) -> String {
    if settings.dirigent_mcp_server_path.is_empty() {
        "dirigent-mcp".to_string()
    } else {
        settings.dirigent_mcp_server_path.clone()
    }
}

pub(super) fn mcp_bin_exists(mcp_bin: &str) -> bool {
    which::which(mcp_bin).is_ok() || std::path::Path::new(mcp_bin).exists()
}
