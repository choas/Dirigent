use std::path::PathBuf;

use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM};
use crate::git;

/// Expand a leading `~` to the user's home directory.
fn expand_tilde(raw: &str) -> PathBuf {
    if raw == "~" || raw.starts_with("~/") || raw.starts_with("~\\") {
        match dirs::home_dir() {
            Some(home) => {
                if raw == "~" {
                    home
                } else {
                    home.join(&raw[2..])
                }
            }
            None => PathBuf::from(raw),
        }
    } else {
        PathBuf::from(raw)
    }
}

/// Format a byte size as a human-readable string (KB or MB).
fn format_size(size_bytes: u64) -> String {
    let size_kb = size_bytes as f64 / 1024.0;
    if size_kb >= 1024.0 {
        format!("{:.1} MB", size_kb / 1024.0)
    } else {
        format!("{:.0} KB", size_kb)
    }
}

/// Reveal a file or directory in the platform's file manager.
fn reveal_in_file_manager(path: &std::path::Path) -> Result<(), std::io::Error> {
    #[cfg(target_os = "macos")]
    let mut child = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()?;
    #[cfg(target_os = "windows")]
    let mut child = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn()?;
    #[cfg(target_os = "linux")]
    let mut child = {
        let dir = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| path.to_path_buf())
        };
        std::process::Command::new("xdg-open").arg(&dir).spawn()?
    };

    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Worktree label based on its current/locked state.
fn worktree_label(wt: &git::WorktreeInfo) -> String {
    if wt.is_current {
        format!("\u{25B6} {} (current)", wt.name)
    } else if wt.is_locked {
        format!("\u{25A0} {}", wt.name)
    } else {
        wt.name.clone()
    }
}

/// Accumulated deferred actions from the worktree panel UI.
#[derive(Default)]
struct WorktreeActions {
    switch_to: Option<PathBuf>,
    remove_path: Option<PathBuf>,
    create_name: Option<String>,
    delete_archive_pending: Option<PathBuf>,
    reveal_path: Option<PathBuf>,
}

impl DirigentApp {
    // ── Repo picker ──────────────────────────────────────────────────

    /// Repo picker window.
    pub(in crate::app) fn render_repo_picker(&mut self, ctx: &egui::Context) {
        if !self.show_repo_picker {
            return;
        }

        let mut open = self.show_repo_picker;
        let mut switch_to: Option<PathBuf> = None;
        let mut error_msg: Option<String> = None;

        egui::Window::new("Open Repository")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_size([450.0, 300.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .show(ctx, |ui| {
                ui.interact(ui.max_rect(), ui.id().with("body_bg"), egui::Sense::click());
                self.render_repo_path_input(ui, &mut switch_to, &mut error_msg);
                ui.separator();
                self.render_recent_repos(ui, &mut switch_to);
            });

        self.show_repo_picker = open;
        self.apply_repo_picker_result(switch_to, error_msg);
    }

    /// Path input row with Open button.
    fn render_repo_path_input(
        &mut self,
        ui: &mut egui::Ui,
        switch_to: &mut Option<PathBuf>,
        error_msg: &mut Option<String>,
    ) {
        ui.horizontal(|ui| {
            ui.label("Path:");
            ui.add(
                egui::TextEdit::singleline(&mut self.repo_path_input)
                    .desired_width(300.0)
                    .font(egui::TextStyle::Monospace),
            );
            if ui.button("Open").clicked() {
                let path = expand_tilde(&self.repo_path_input);
                if let Ok(canonical) = std::fs::canonicalize(&path) {
                    *switch_to = Some(canonical);
                } else {
                    *error_msg = Some(format!("Path not found: {}", path.display()));
                }
            }
        });
    }

    /// Recent repositories list.
    fn render_recent_repos(&self, ui: &mut egui::Ui, switch_to: &mut Option<PathBuf>) {
        ui.label("Recent repositories:");
        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                for repo_path in self.settings.recent_repos.clone() {
                    if ui.button(&repo_path).clicked() {
                        let path = PathBuf::from(&repo_path);
                        if let Ok(canonical) = std::fs::canonicalize(&path) {
                            *switch_to = Some(canonical);
                        }
                    }
                }
                if self.settings.recent_repos.is_empty() {
                    ui.label(
                        egui::RichText::new("(none)")
                            .italics()
                            .color(self.semantic.tertiary_text),
                    );
                }
            });
    }

    /// Apply the result of the repo picker dialog.
    fn apply_repo_picker_result(&mut self, switch_to: Option<PathBuf>, error_msg: Option<String>) {
        if let Some(msg) = error_msg {
            self.set_status_message(msg);
        }
        if let Some(new_root) = switch_to {
            self.show_repo_picker = false;
            self.switch_repo(new_root);
        }
    }

    // ── Worktree panel ───────────────────────────────────────────────

    /// Worktree panel.
    pub(in crate::app) fn render_worktree_panel(&mut self, ctx: &egui::Context) {
        if !self.git.show_worktree_panel {
            return;
        }

        let mut open = self.git.show_worktree_panel;
        let mut actions = WorktreeActions::default();

        egui::Window::new("Git Worktrees")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([400.0, 300.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .show(ctx, |ui| {
                ui.interact(ui.max_rect(), ui.id().with("body_bg"), egui::Sense::click());
                self.render_worktree_list(ui, &mut actions);
                self.render_create_worktree_input(ui, &mut actions);
                self.render_archived_dbs_section(ui, &mut actions);
            });

        self.git.show_worktree_panel = open;
        self.handle_worktree_actions(actions);
    }

    /// Scrollable list of existing worktrees with Switch/Remove buttons.
    fn render_worktree_list(&self, ui: &mut egui::Ui, actions: &mut WorktreeActions) {
        let fs = self.settings.font_size;
        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                for wt in &self.git.worktrees {
                    self.render_worktree_row(ui, wt, fs, actions);
                    ui.separator();
                }

                if self.git.worktrees.is_empty() {
                    ui.label(
                        egui::RichText::new("No worktrees found")
                            .italics()
                            .color(self.semantic.tertiary_text),
                    );
                }
            });
    }

    /// Single worktree row with label and action buttons.
    fn render_worktree_row(
        &self,
        ui: &mut egui::Ui,
        wt: &git::WorktreeInfo,
        fs: f32,
        actions: &mut WorktreeActions,
    ) {
        ui.horizontal(|ui| {
            let label = worktree_label(wt);
            ui.label(icon(&label, fs).strong());
            ui.label(
                egui::RichText::new(wt.path.to_string_lossy().as_ref())
                    .small()
                    .color(self.semantic.secondary_text),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !wt.is_current
                    && !wt.is_locked
                    && !wt.is_main
                    && ui.small_button("Remove").clicked()
                {
                    actions.remove_path = Some(wt.path.clone());
                }
                if !wt.is_current && ui.small_button("Switch").clicked() {
                    actions.switch_to = Some(wt.path.clone());
                }
            });
        });
    }

    /// Input field and Create button for new worktrees.
    fn render_create_worktree_input(&mut self, ui: &mut egui::Ui, actions: &mut WorktreeActions) {
        ui.add_space(SPACE_SM);
        ui.label("Create new worktree:");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.git.new_worktree_name)
                    .desired_width(200.0)
                    .hint_text("branch-name")
                    .font(egui::TextStyle::Monospace),
            );
            if ui.button("Create").clicked() && !self.git.new_worktree_name.is_empty() {
                actions.create_name = Some(self.git.new_worktree_name.clone());
            }
        });
    }

    /// Collapsible section listing archived worktree databases.
    fn render_archived_dbs_section(&mut self, ui: &mut egui::Ui, actions: &mut WorktreeActions) {
        if self.git.archived_dbs.is_empty() {
            return;
        }

        ui.add_space(SPACE_SM);
        let arrow = if self.git.show_archived_dbs {
            "\u{25BC}"
        } else {
            "\u{25B6}"
        };
        let header = format!(
            "{} Archived Worktree DBs ({})",
            arrow,
            self.git.archived_dbs.len()
        );
        if ui
            .selectable_label(false, egui::RichText::new(&header).strong())
            .clicked()
        {
            self.git.show_archived_dbs = !self.git.show_archived_dbs;
        }

        if !self.git.show_archived_dbs {
            return;
        }

        for db in &self.git.archived_dbs {
            self.render_archived_db_row(ui, db, actions);
        }
    }

    /// Single archived DB row with name, size, and action buttons.
    fn render_archived_db_row(
        &self,
        ui: &mut egui::Ui,
        db: &git::ArchivedDb,
        actions: &mut WorktreeActions,
    ) {
        ui.horizontal(|ui| {
            ui.label(&db.name);
            ui.label(
                egui::RichText::new(format_size(db.size_bytes))
                    .small()
                    .color(self.semantic.secondary_text),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Delete").clicked() {
                    actions.delete_archive_pending = Some(db.path.clone());
                }
                if ui.small_button("Reveal").clicked() {
                    actions.reveal_path = Some(db.path.clone());
                }
            });
        });
    }

    /// Process all deferred actions collected during the worktree panel frame.
    fn handle_worktree_actions(&mut self, actions: WorktreeActions) {
        if let Some(path) = actions.switch_to {
            self.git.show_worktree_panel = false;
            if let Ok(canonical) = std::fs::canonicalize(&path) {
                self.switch_repo(canonical);
            } else {
                self.switch_repo(path);
            }
        }

        if let Some(path) = actions.remove_path {
            self.do_remove_worktree(path, false);
        }

        if let Some(name) = actions.create_name {
            match git::create_worktree(&self.project_root, &name) {
                Ok(_path) => {
                    self.git.new_worktree_name.clear();
                    self.reload_worktrees();
                }
                Err(e) => {
                    self.set_status_message(format!("Failed to create worktree: {}", e));
                }
            }
        }

        if let Some(path) = actions.delete_archive_pending {
            self.git.pending_delete_archive = Some(path);
        }

        if let Some(path) = actions.reveal_path {
            if let Err(e) = reveal_in_file_manager(&path) {
                self.set_status_message(format!("Failed to reveal in file manager: {}", e));
            }
        }
    }

    // ── Worktree removal ─────────────────────────────────────────────

    /// Attempt to remove a worktree, archiving its DB first.
    /// If removal fails due to modified/untracked files, stores state for the
    /// force-remove confirmation dialog instead of just showing a status message.
    fn do_remove_worktree(&mut self, path: PathBuf, force: bool) {
        // Preflight: check for dirty files before attempting removal.
        // No archive is created here — archiving only happens once the user
        // confirms removal (or if there are no dirty files).
        if !force {
            let dirty = git::get_dirty_files(&path);
            if !dirty.is_empty() {
                let mut files: Vec<_> = dirty.iter().collect();
                files.sort_by_key(|(p, _)| (*p).clone());
                let listing: Vec<String> = files
                    .iter()
                    .take(10)
                    .map(|(p, status)| format!("  {} {}", status, p))
                    .collect();
                let mut msg = format!(
                    "{} modified or untracked file(s):\n{}",
                    dirty.len(),
                    listing.join("\n")
                );
                if dirty.len() > 10 {
                    msg.push_str(&format!("\n  … and {} more", dirty.len() - 10));
                }
                self.git.pending_force_remove = Some((path, msg));
                self.git.pending_archive_msg = None;
                return;
            }
        }

        // Archive the worktree's DB just before removal.
        let archive_msg = match git::main_worktree_path(&self.project_root) {
            Ok(main_path) => {
                let wt_name = self
                    .git
                    .worktrees
                    .iter()
                    .find(|wt| wt.path == path)
                    .map(|wt| wt.name.clone())
                    .unwrap_or_else(|| {
                        path.file_name()
                            .map(|f| f.to_string_lossy().to_string())
                            .unwrap_or_else(|| "unknown".to_string())
                    });
                match git::archive_worktree_db(&main_path, &path, &wt_name) {
                    Ok(Some(archive_path)) => {
                        let name = archive_path
                            .file_name()
                            .map(|f| f.to_string_lossy().to_string())
                            .unwrap_or_default();
                        Some(format!("DB archived as {}", name))
                    }
                    Ok(None) => None,
                    Err(e) => {
                        self.set_status_message(format!(
                            "Cannot remove worktree: failed to archive DB: {}",
                            e
                        ));
                        return;
                    }
                }
            }
            Err(e) => {
                self.set_status_message(format!(
                    "Cannot remove worktree: could not determine main worktree path: {}",
                    e
                ));
                return;
            }
        };

        match git::remove_worktree(&self.project_root, &path, force) {
            Ok(()) => {
                self.git.pending_force_remove = None;
                self.git.pending_archive_msg = None;
                self.reload_worktrees();
                if let Some(msg) = archive_msg {
                    self.set_status_message(format!("Worktree removed. {}", msg));
                }
            }
            Err(e) => {
                self.set_status_message(format!("Failed to remove worktree: {}", e));
            }
        }
    }

    /// Renders the force-remove confirmation dialog when a worktree has
    /// modified or untracked files.
    pub(in crate::app) fn render_force_remove_dialog(&mut self, ctx: &egui::Context) {
        let Some((ref path, ref error_msg)) = self.git.pending_force_remove else {
            return;
        };
        let path = path.clone();
        let error_msg = error_msg.clone();

        let mut force = false;
        let mut cancel = false;

        egui::Window::new("Remove Worktree")
            .collapsible(false)
            .resizable(false)
            .default_size([420.0, 0.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .show(ctx, |ui| {
                ui.label("The worktree contains modified or untracked files:");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(path.to_string_lossy().as_ref())
                        .monospace()
                        .color(self.semantic.secondary_text),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(&error_msg)
                        .small()
                        .color(self.semantic.tertiary_text),
                );
                ui.add_space(8.0);
                ui.label("Force remove will delete all changes in this worktree.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(egui::RichText::new("Force Remove").color(egui::Color32::RED))
                            .clicked()
                        {
                            force = true;
                        }
                    });
                });
            });

        if cancel {
            self.git.pending_force_remove = None;
            self.git.pending_archive_msg = None;
        } else if force {
            self.git.pending_force_remove = None;
            self.do_remove_worktree(path, true);
        }
    }

    /// Renders the confirmation dialog for deleting an archived worktree DB.
    pub(in crate::app) fn render_delete_archive_dialog(&mut self, ctx: &egui::Context) {
        let Some(ref path) = self.git.pending_delete_archive else {
            return;
        };
        let path = path.clone();

        let mut confirm = false;
        let mut cancel = false;

        egui::Window::new("Delete Archived DB")
            .collapsible(false)
            .resizable(false)
            .default_size([400.0, 0.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .show(ctx, |ui| {
                ui.label("Are you sure you want to delete this archived database?");
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(path.to_string_lossy().as_ref())
                        .monospace()
                        .color(self.semantic.secondary_text),
                );
                ui.add_space(8.0);
                ui.label("This action cannot be undone.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(egui::RichText::new("Delete").color(egui::Color32::RED))
                            .clicked()
                        {
                            confirm = true;
                        }
                    });
                });
            });

        if cancel {
            self.git.pending_delete_archive = None;
        } else if confirm {
            self.git.pending_delete_archive = None;
            match std::fs::remove_file(&path) {
                Ok(()) => self.reload_worktrees(),
                Err(e) => self.set_status_message(format!("Failed to delete archived DB: {}", e)),
            }
        }
    }
}
