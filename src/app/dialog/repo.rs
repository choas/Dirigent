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
        let mut delete_archive_pending: Option<PathBuf> = None;
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
                // Consume clicks on empty space so they don't pass through
                // to the modal overlay and dismiss this dialog.
                ui.interact(ui.max_rect(), ui.id().with("body_bg"), egui::Sense::click());

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
                        if self.git.show_archived_dbs {
                            "\u{25BC}"
                        } else {
                            "\u{25B6}"
                        },
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
                                            delete_archive_pending = Some(db.path.clone());
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
            self.do_remove_worktree(path, false);
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

        if let Some(path) = delete_archive_pending {
            self.git.pending_delete_archive = Some(path);
        }

        if let Some(path) = reveal_path {
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("open")
                    .arg("-R")
                    .arg(&path)
                    .spawn();
            }
            #[cfg(target_os = "windows")]
            {
                let _ = std::process::Command::new("explorer")
                    .arg(format!("/select,{}", path.display()))
                    .spawn();
            }
            #[cfg(target_os = "linux")]
            {
                let dir = if path.is_dir() {
                    path.clone()
                } else {
                    path.parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or(path.clone())
                };
                let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
            }
        }
    }

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
