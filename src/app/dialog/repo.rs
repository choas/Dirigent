use std::path::PathBuf;

use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM};
use crate::git;

impl DirigentApp {
    // Repo picker window
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
                ui.horizontal(|ui| {
                    ui.label("Path:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.repo_path_input)
                            .desired_width(300.0)
                            .font(egui::TextStyle::Monospace),
                    );
                    if ui.button("Open").clicked() {
                        let raw = &self.repo_path_input;
                        let path = if raw == "~" || raw.starts_with("~/") {
                            let home = std::env::var("HOME")
                                .map(PathBuf::from)
                                .unwrap_or_else(|_| PathBuf::from("/"));
                            if raw == "~" {
                                home
                            } else {
                                home.join(&raw[2..])
                            }
                        } else {
                            PathBuf::from(raw)
                        };
                        if let Ok(canonical) = std::fs::canonicalize(&path) {
                            switch_to = Some(canonical);
                        } else {
                            error_msg = Some(format!("Path not found: {}", path.display()));
                        }
                    }
                });

                ui.separator();
                ui.label("Recent repositories:");
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for repo_path in self.settings.recent_repos.clone() {
                            if ui.button(&repo_path).clicked() {
                                let path = PathBuf::from(&repo_path);
                                if let Ok(canonical) = std::fs::canonicalize(&path) {
                                    switch_to = Some(canonical);
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
            });

        self.show_repo_picker = open;

        if let Some(msg) = error_msg {
            self.set_status_message(msg);
        }
        if let Some(new_root) = switch_to {
            self.show_repo_picker = false;
            self.switch_repo(new_root);
        }
    }

    // Worktree panel
    pub(in crate::app) fn render_worktree_panel(&mut self, ctx: &egui::Context) {
        if !self.git.show_worktree_panel {
            return;
        }

        let mut open = self.git.show_worktree_panel;
        let mut switch_to: Option<PathBuf> = None;
        let mut remove_path: Option<PathBuf> = None;
        let mut create_name: Option<String> = None;
        let mut delete_archive: Option<PathBuf> = None;
        let mut reveal_path: Option<PathBuf> = None;
        let fs = self.settings.font_size;

        egui::Window::new("Git Worktrees")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([400.0, 300.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for wt in &self.git.worktrees {
                            ui.horizontal(|ui| {
                                let label = if wt.is_current {
                                    format!("\u{25B6} {} (current)", wt.name)
                                } else if wt.is_locked {
                                    format!("\u{25A0} {}", wt.name)
                                } else {
                                    wt.name.clone()
                                };
                                ui.label(icon(&label, fs).strong());
                                ui.label(
                                    egui::RichText::new(wt.path.to_string_lossy().as_ref())
                                        .small()
                                        .color(self.semantic.secondary_text),
                                );

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if !wt.is_current
                                            && !wt.is_locked
                                            && ui.small_button("Remove").clicked()
                                        {
                                            remove_path = Some(wt.path.clone());
                                        }
                                        if !wt.is_current && ui.small_button("Switch").clicked() {
                                            switch_to = Some(wt.path.clone());
                                        }
                                    },
                                );
                            });
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
                        create_name = Some(self.git.new_worktree_name.clone());
                    }
                });

                // Archived worktree DBs section
                if !self.git.archived_dbs.is_empty() {
                    ui.add_space(SPACE_SM);
                    let header = format!(
                        "{} Archived Worktree DBs ({})",
                        if self.git.show_archived_dbs { "\u{25BC}" } else { "\u{25B6}" },
                        self.git.archived_dbs.len()
                    );
                    if ui
                        .selectable_label(false, egui::RichText::new(&header).strong())
                        .clicked()
                    {
                        self.git.show_archived_dbs = !self.git.show_archived_dbs;
                    }

                    if self.git.show_archived_dbs {
                        for db in &self.git.archived_dbs {
                            ui.horizontal(|ui| {
                                ui.label(&db.name);
                                let size_kb = db.size_bytes as f64 / 1024.0;
                                let size_str = if size_kb >= 1024.0 {
                                    format!("{:.1} MB", size_kb / 1024.0)
                                } else {
                                    format!("{:.0} KB", size_kb)
                                };
                                ui.label(
                                    egui::RichText::new(size_str)
                                        .small()
                                        .color(self.semantic.secondary_text),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button("Delete").clicked() {
                                            delete_archive = Some(db.path.clone());
                                        }
                                        if ui.small_button("Reveal").clicked() {
                                            reveal_path = Some(db.path.clone());
                                        }
                                    },
                                );
                            });
                        }
                    }
                }
            });

        self.git.show_worktree_panel = open;

        if let Some(path) = switch_to {
            self.git.show_worktree_panel = false;
            if let Ok(canonical) = std::fs::canonicalize(&path) {
                self.switch_repo(canonical);
            } else {
                self.switch_repo(path);
            }
        }

        if let Some(path) = remove_path {
            // Archive the worktree's DB before removal
            let archive_msg = match git::main_worktree_path(&self.project_root) {
                Ok(main_path) => {
                    // Find the worktree name (branch name) for the archive filename
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
                        Ok(None) => None, // No DB to archive
                        Err(e) => {
                            eprintln!("Failed to archive worktree DB: {}", e);
                            None
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Could not determine main worktree path: {}", e);
                    None
                }
            };

            match git::remove_worktree(&self.project_root, &path) {
                Ok(()) => {
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

        if let Some(name) = create_name {
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

        if let Some(path) = delete_archive {
            if std::fs::remove_file(&path).is_ok() {
                self.reload_worktrees(); // refreshes archived_dbs list too
            } else {
                self.set_status_message("Failed to delete archived DB".to_string());
            }
        }

        if let Some(path) = reveal_path {
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("open")
                    .arg("-R")
                    .arg(&path)
                    .spawn();
            }
        }
    }
}
