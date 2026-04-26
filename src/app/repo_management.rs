use std::collections::HashSet;
use std::path::PathBuf;

use super::{start_fs_watcher, DirigentApp, NavigationHistory};
use crate::db::Database;
use crate::file_tree::FileTree;
use crate::git;
use crate::settings;

impl DirigentApp {
    pub(super) fn switch_repo(&mut self, new_root: PathBuf) {
        // Cancel all running tasks — they belong to the old repo.
        self.cancel_all_tasks();
        self.run_queue.clear();
        self.follow_up_queue.clear();
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
        // Offer to initialize git if not a repository
        if git2::Repository::discover(&new_root).is_err() {
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
        self.file_tree = FileTree::scan(&self.project_root).ok();
        self.fs_changed
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self._fs_watcher = start_fs_watcher(&self.project_root, &self.fs_changed, &self.egui_ctx);
        self.archived_cue_limit = 10;
        self.confirm_delete_archived = false;
        self.reload_cues();
        self.git.info = git::read_git_info(&self.project_root);
        self.git.dirty_files = git::get_dirty_files(&self.project_root);
        self.git.ahead_of_remote = git::get_ahead_of_remote(&self.project_root);
        self.viewer.tabs.clear();
        self.viewer.active_tab = None;
        self.viewer.nav_history = NavigationHistory::new();
        self.viewer.quick_open_active = false;
        self.viewer.quick_open_query.clear();
        self.viewer.quick_open_selected = 0;
        self.git.commit_history_limit = 10;
        let limit = self.git.commit_history_limit.max(self.git.ahead_of_remote);
        self.git.commit_history = git::read_commit_history(&self.project_root, limit);
        self.git.commit_history_total = git::count_commits(&self.project_root);
        self.git.history_cache_key = (String::new(), 0);
        self.expanded_dirs = HashSet::new();
        self.diff_review = None;
        self.prompt_history_query = String::new();
        self.prompt_history_results = Vec::new();
        self.prompt_history_active = false;
        self.git.worktrees = git::list_worktrees(&self.project_root).unwrap_or_default();
        self.cached_total_cost = self.db.total_cost().unwrap_or(0.0);
        self.cached_prompt_input.clear();
        self.cached_prompt_hints.clear();
        self.cached_prompt_suggestions.clear();
        self.cached_lines_with_cues = None;
        self.cue_warnings.clear();
        // Reload cues and all cue-derived caches (archived counts, labels, activity, etc.)
        self.reload_cues();

        // Load project-specific settings if the new repo has them,
        // carrying over recent_repos from the current session.
        let recent_repos = self.settings.recent_repos.clone();
        self.settings = settings::load_settings(&self.project_root);
        self.settings.recent_repos = recent_repos;
        let path_str = new_root.to_string_lossy().to_string();
        settings::add_recent_repo(&mut self.settings, &path_str);
        settings::save_settings(&self.project_root, &self.settings);
        // Persist to global list so every app launch remembers this project.
        settings::add_global_recent_project(&path_str);
        self.needs_theme_apply = true;

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

    pub(super) fn reload_worktrees(&mut self) {
        self.git.worktrees = git::list_worktrees(&self.project_root).unwrap_or_default();
        // Refresh archived DBs list from main worktree
        if let Ok(main_path) = git::main_worktree_path(&self.project_root) {
            self.git.archived_dbs = git::list_archived_dbs(&main_path);
        }
    }
}
