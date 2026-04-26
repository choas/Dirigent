use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use eframe::egui;

use super::symbols;
use super::{
    DirigentApp, ELAPSED_REPAINT, FS_RESCAN_DEBOUNCE, LOG_SYNC_INTERVAL, REPAINT_FAST, REPAINT_SLOW,
};
use crate::git;
use crate::settings;

impl DirigentApp {
    /// Re-read settings from disk (the file may have been changed externally by Claude Code).
    /// Only applies the new settings (and triggers a theme re-apply) if they actually differ
    /// from the current in-memory state, to avoid unnecessary theme resets on unrelated FS changes.
    pub(super) fn reload_settings_from_disk(&mut self) {
        let new_settings = settings::load_settings(&self.project_root);

        // Compare ignoring in-memory-only fields (recent_repos may differ from disk)
        let mut current_cmp = self.settings.clone();
        let mut new_cmp = new_settings.clone();
        current_cmp.recent_repos = Vec::new();
        new_cmp.recent_repos = Vec::new();

        let current_json = serde_json::to_string(&current_cmp).unwrap_or_default();
        let new_json = serde_json::to_string(&new_cmp).unwrap_or_default();

        if current_json == new_json {
            return;
        }

        let recent_repos = self.settings.recent_repos.clone();
        self.settings = new_settings;
        self.settings.recent_repos = recent_repos;
        self.needs_theme_apply = true;
    }

    /// Handle filesystem changes: rescan file tree, reload tabs, trigger agents.
    pub(super) fn handle_fs_changes(&mut self) {
        let fs_ready = self.fs_changed.load(Ordering::Relaxed)
            && self.last_fs_rescan.elapsed() >= FS_RESCAN_DEBOUNCE;
        if !fs_ready {
            return;
        }
        self.fs_changed.store(false, Ordering::Relaxed);
        self.last_fs_rescan = std::time::Instant::now();
        self.reload_file_tree();
        self.git.dirty_files = git::get_dirty_files(&self.project_root);
        self.git.ahead_of_remote = git::get_ahead_of_remote(&self.project_root);
        self.reload_settings_from_disk();
        self.reload_open_tabs();
        self.trigger_agents_for(&crate::agents::AgentTrigger::OnFileChange, None, "");
    }

    /// Reload all open tabs from disk, notify LSP for changed files, and refresh
    /// in-file search. Returns changed and failed paths.
    pub(super) fn reload_open_tabs_and_notify_lsp(&mut self) -> super::types::TabReloadResult {
        let mut changed = Vec::new();
        let mut failed = Vec::new();
        for tab in &mut self.viewer.tabs {
            match tab.reload_from_disk() {
                Ok(true) => changed.push(tab.file_path.clone()),
                Ok(false) => {}
                Err(e) => failed.push((tab.file_path.clone(), e.to_string())),
            }
        }
        if self.settings.lsp_enabled {
            for path in &changed {
                self.lsp.notify_file_changed(path);
            }
        }
        if self.search.in_file_active && !self.search.in_file_query.is_empty() {
            self.update_search_in_file_matches();
        }
        super::types::TabReloadResult { changed, failed }
    }

    /// Reload content of all open tabs from disk, closing tabs for deleted files.
    pub(super) fn reload_open_tabs(&mut self) {
        let result = self.reload_open_tabs_and_notify_lsp();
        let mut stale_names: Vec<String> = Vec::new();
        let mut stale_indices: Vec<usize> = Vec::new();
        for (i, tab) in self.viewer.tabs.iter().enumerate() {
            if !tab.file_path.exists() {
                if let Some(name) = tab.file_path.file_name() {
                    stale_names.push(name.to_string_lossy().into_owned());
                }
                stale_indices.push(i);
            }
        }
        for &idx in stale_indices.iter().rev() {
            self.viewer.close_tab(idx);
        }
        let mut messages = Vec::new();
        if !stale_names.is_empty() {
            messages.push(format!("Closed (file deleted): {}", stale_names.join(", ")));
        }
        if !result.failed.is_empty() {
            let names: Vec<String> = result
                .failed
                .iter()
                .map(|(p, e)| {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| p.display().to_string());
                    format!("{} ({})", name, e)
                })
                .collect();
            messages.push(format!("Failed to reload: {}", names.join(", ")));
        }
        if !messages.is_empty() {
            self.set_status_message(messages.join("; "));
        }
    }

    /// Poll background receivers for file tree, search, and go-to-definition results.
    pub(super) fn poll_background_results(&mut self) {
        if let Ok(tree) = self.file_tree_rx.try_recv() {
            self.file_tree = Some(tree);
            self.file_tree_scanning = false;
        }
        if let Ok(err_msg) = self.file_tree_error_rx.try_recv() {
            self.file_tree_scanning = false;
            self.set_status_message(err_msg);
        }
        if let Ok(results) = self.search.search_result_rx.try_recv() {
            self.search.in_files_results = results;
            self.search.in_files_searching = false;
        }
        if let Ok((gen, file_path, target_line, msg)) = self.goto_def_rx.try_recv() {
            self.apply_goto_def_result(gen, file_path, target_line, msg);
        }
    }

    /// Process LSP results: definition navigation, document symbol refresh.
    pub(super) fn process_lsp_results(&mut self) {
        self.handle_lsp_definition_result();
        self.update_lsp_document_symbols();
    }

    /// Navigate to an LSP definition result, or fall back to regex search.
    fn handle_lsp_definition_result(&mut self) {
        if let Some((def_path, def_line)) = self.lsp.definition_result.take() {
            self.lsp_goto_def_fallback_word = None;
            if !self.is_within_project_root(&def_path) {
                eprintln!(
                    "[lsp] rejecting definition outside project root: {:?}",
                    def_path
                );
                self.set_status_message("Definition is outside the project".to_string());
                return;
            }
            self.push_nav_history();
            self.load_file(def_path);
            self.viewer.scroll_to_line = Some(def_line);
            self.set_status_message(format!("LSP: definition at line {}", def_line));
            return;
        }
        // If definition completed with no result, fall back to regex
        if !self.lsp.definition_pending {
            if let Some(word) = self.lsp_goto_def_fallback_word.take() {
                self.goto_definition(&word);
            }
        }
    }

    /// Request and apply document symbols from the LSP for the active tab.
    fn update_lsp_document_symbols(&mut self) {
        if !self.settings.lsp_enabled {
            return;
        }
        let active_idx = match self.viewer.active_tab {
            Some(idx) if idx < self.viewer.tabs.len() => idx,
            _ => return,
        };
        let file_path = self.viewer.tabs[active_idx].file_path.clone();
        if !self.lsp.has_initialized_server_for(&file_path) {
            return;
        }
        if let Some(lsp_syms) = self.lsp.document_symbols.get(&file_path) {
            let converted = symbols::from_lsp_symbols(lsp_syms);
            if !converted.is_empty() {
                self.viewer.tabs[active_idx].symbols = converted;
            }
        } else {
            self.lsp.request_document_symbols(&file_path);
        }
    }

    /// Apply a go-to-definition result if it matches the current generation.
    pub(super) fn apply_goto_def_result(
        &mut self,
        gen: u64,
        file_path: PathBuf,
        target_line: usize,
        msg: String,
    ) {
        if gen != self.goto_def_gen {
            return;
        }
        if target_line > 0 {
            if !self.is_within_project_root(&file_path) {
                eprintln!(
                    "[goto-def] rejecting path outside project root: {:?}",
                    file_path
                );
                self.set_status_message("Definition is outside the project".to_string());
                return;
            }
            self.push_nav_history();
            self.load_file(file_path);
            self.viewer.scroll_to_line = Some(target_line);
        }
        self.set_status_message(msg);
    }

    /// Periodically sync running logs and clean up agent history.
    pub(super) fn sync_logs_and_cleanup(&mut self) {
        let has_active_logs = !self.claude.exec_ids.is_empty() || self.claude.show_log.is_some();
        if has_active_logs && self.claude.last_log_flush.elapsed() >= LOG_SYNC_INTERVAL {
            self.sync_running_logs();
        }
        if self.last_agent_cleanup.elapsed() >= Duration::from_secs(3600) {
            self.last_agent_cleanup = std::time::Instant::now();
            let _ = self.db.cleanup_agent_runs(200, 65536);
        }
    }

    /// Schedule repaint intervals based on current application state.
    pub(super) fn schedule_repaints(&self, ctx: &egui::Context) {
        let has_running = self.cached_has_running_cue;
        if has_running {
            let interval = if self.claude.show_log.is_some() {
                REPAINT_FAST
            } else {
                REPAINT_SLOW
            };
            ctx.request_repaint_after(interval);
        } else if !self.run_queue.is_empty() {
            ctx.request_repaint_after(REPAINT_SLOW);
        } else if self.fs_changed.load(Ordering::Relaxed) {
            ctx.request_repaint_after(FS_RESCAN_DEBOUNCE);
        }
        if !self.scheduled_runs.is_empty() {
            ctx.request_repaint_after(ELAPSED_REPAINT);
        }
        let has_async_git = self.git.importing_pr
            || self.git.pushing
            || self.git.pulling
            || self.git.creating_pr
            || self.git.notifying_pr;
        if has_async_git {
            ctx.request_repaint_after(REPAINT_SLOW);
        }
        if let Some(delay) = self.next_source_poll_delay() {
            ctx.request_repaint_after(delay);
        }
    }

    /// Compute the earliest next source poll delay.
    pub(super) fn next_source_poll_delay(&self) -> Option<Duration> {
        let mut min_delay = None::<Duration>;
        for s in &self.settings.sources {
            if !s.enabled || s.poll_interval_secs == 0 {
                continue;
            }
            let interval = Duration::from_secs(s.poll_interval_secs);
            let remaining = match self.sources.last_poll.get(&s.name) {
                Some(last) => interval.saturating_sub(last.elapsed()),
                None => Duration::ZERO,
            };
            let clamped = remaining.max(Duration::from_secs(1));
            min_delay = Some(match min_delay {
                Some(cur) => cur.min(clamped),
                None => clamped,
            });
        }
        min_delay
    }
}
