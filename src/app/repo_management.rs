use std::collections::HashSet;
use std::path::PathBuf;

use super::{start_fs_watcher, DirigentApp, NavigationHistory};
use crate::db::Database;
use crate::file_tree::FileTree;
use crate::git;
use crate::jj;
use crate::settings::{self, VcsBackend};

impl DirigentApp {
    pub(super) fn switch_repo(&mut self, new_root: PathBuf) {
        // Cancel all running tasks — they belong to the old repo.
        self.cancel_all_tasks();
        self.run_queue.clear();
        self.follow_up_queue.clear();
        self.pending_auto_continues.clear();
        self.claude.auto_continue_count.clear();
        self.claude.auto_continue_spawn_retries.clear();
        self.scheduled_runs.clear();
        self.schedule_inputs.clear();

        // Validate that the path is an existing directory
        if !new_root.is_dir() {
            self.set_status_message(format!(
                "Cannot switch repo: not a directory: {}",
                new_root.display()
            ));
            return;
        }
        // Accept the directory if it contains a jj or git repository
        if !new_root.join(".jj").is_dir() && git2::Repository::discover(&new_root).is_err() {
            self.git_init_confirm = Some(new_root);
            return;
        }

        self.db = match Database::open(&new_root) {
            Ok(db) => db,
            Err(e) => {
                self.set_status_message(format!("Failed to open database: {}", e));
                return;
            }
        };
        self.project_root = new_root.clone();
        match FileTree::scan(&self.project_root) {
            Ok(tree) => self.file_tree = Some(tree),
            Err(e) => {
                self.file_tree = None;
                self.set_status_message(format!("Failed to scan file tree: {}", e));
            }
        }
        self.fs_changed
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self._fs_watcher = start_fs_watcher(&self.project_root, &self.fs_changed, &self.egui_ctx);
        self.archived_cue_limit = 10;
        self.confirm_delete_archived = false;

        // Load project-specific settings before any VCS calls so the correct
        // backend (git vs jj) is used from the start.
        let recent_repos = self.settings.recent_repos.clone();
        self.settings = settings::load_settings(&self.project_root);
        self.settings.recent_repos = recent_repos;
        self.ensure_jj_colocated();

        self.reload_cues();
        self.reload_git_info();
        self.viewer.tabs.clear();
        self.viewer.active_tab = None;
        self.viewer.nav_history = NavigationHistory::new();
        self.viewer.quick_open_active = false;
        self.viewer.quick_open_query.clear();
        self.viewer.quick_open_selected = 0;
        self.git.commit_history_limit = 10;
        self.git.history_cache_key = (String::new(), 0);
        self.git.active_bookmark = None;
        self.reload_commit_history();
        self.expanded_dirs = HashSet::new();
        self.git.git_view_expanded_dirs.clear();
        self.diff_review = None;
        self.prompt_history_query = String::new();
        self.prompt_history_results = Vec::new();
        self.prompt_history_active = false;
        self.reload_worktrees();
        self.cached_total_cost = self.db.total_cost().unwrap_or(0.0);
        self.cached_prompt_input.clear();
        self.cached_prompt_hints.clear();
        self.cached_prompt_suggestions.clear();
        self.cached_lines_with_cues = None;
        self.cue_warnings.clear();
        // Reload cues and all cue-derived caches (archived counts, labels, activity, etc.)
        self.reload_cues();
        let path_str = new_root.to_string_lossy().to_string();
        settings::add_recent_repo(&mut self.settings, &path_str);
        if let Err(e) = settings::save_settings(&self.project_root, &self.settings) {
            self.set_status_message(format!("Failed to save settings: {e}"));
        }
        // Persist to global list so every app launch remembers this project.
        settings::add_global_recent_project(&path_str);
        self.needs_theme_apply = true;
        self.logo_texture = None;
        #[cfg(target_os = "macos")]
        {
            crate::app::update_macos_dock_icon(&self.settings.custom_dock_icon_path);
            let folder = self
                .project_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| self.project_root.to_string_lossy().to_string());
            crate::app::set_macos_dock_name(&folder);
        }

        // Update window title to show the new folder name
        if let Some(ctx) = self.egui_ctx.get() {
            let folder = self
                .project_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| self.project_root.to_string_lossy().to_string());
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Title(format!(
                "Dirigent - {}",
                folder
            )));
        }
    }

    /// If the VCS backend is jj but no `.jj` directory exists yet (git-only
    /// repo), run `jj git init --colocate` to create a jj repo backed by git.
    pub(super) fn ensure_jj_colocated(&mut self) {
        if self.settings.vcs_backend != VcsBackend::Jj {
            return;
        }
        if self.project_root.join(".jj").is_dir() {
            return;
        }
        if !self.project_root.join(".git").exists() {
            return;
        }
        let jj_path = &self.settings.jj_cli_path;
        let mut cmd = if jj_path.is_empty() {
            std::process::Command::new("jj")
        } else {
            std::process::Command::new(jj_path)
        };
        match cmd
            .args(["git", "init", "--colocate"])
            .current_dir(&self.project_root)
            .output()
        {
            Ok(o) if o.status.success() => {
                self.set_status_message(format!(
                    "Initialized jj repo (colocated) at {}",
                    self.project_root.display()
                ));
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                self.set_status_message(format!("jj git init failed: {}", stderr.trim()));
            }
            Err(e) => {
                self.set_status_message(format!("jj git init failed: {}", e));
            }
        }
    }

    pub(super) fn reload_worktrees(&mut self) {
        self.git.worktrees = match self.settings.vcs_backend {
            VcsBackend::Jj => {
                jj::jj_list_workspaces(&self.project_root, &self.settings.jj_cli_path)
                    .unwrap_or_default()
            }
            VcsBackend::Git => git::list_worktrees(&self.project_root).unwrap_or_default(),
        };
        // Refresh archived DBs list from main worktree
        if let Ok(main_path) = git::main_worktree_path(&self.project_root) {
            self.git.archived_dbs = git::list_archived_dbs(&main_path);
        }
    }
}
