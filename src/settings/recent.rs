use std::path::PathBuf;

use super::app_settings::Settings;

pub(crate) fn add_recent_repo(settings: &mut Settings, path: &str) {
    settings.recent_repos.retain(|p| p != path);
    settings.recent_repos.insert(0, path.to_string());
    settings.recent_repos.truncate(10);
}

// ---------------------------------------------------------------------------
// Global recent-projects list (persisted across all projects / app launches)
// ---------------------------------------------------------------------------

/// Returns the path to the global recent-projects file:
/// `~/Library/Application Support/Dirigent/recent_projects.json`
fn global_recent_projects_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join("Library/Application Support/Dirigent/recent_projects.json"))
}

/// Load the global list of recently opened project paths.
pub(crate) fn load_global_recent_projects() -> Vec<String> {
    let path = match global_recent_projects_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Persist the global list of recently opened project paths.
pub(crate) fn save_global_recent_projects(projects: &[String]) {
    let path = match global_recent_projects_path() {
        Some(p) => p,
        None => return,
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(projects) {
        let _ = std::fs::write(path, json);
    }
}

/// Add a project path to the global recent list and persist it.
pub(crate) fn add_global_recent_project(path: &str) {
    let mut projects = load_global_recent_projects();
    projects.retain(|p| p != path);
    projects.insert(0, path.to_string());
    projects.truncate(20);
    save_global_recent_projects(&projects);
}
