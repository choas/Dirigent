use std::path::{Path, PathBuf};

use eframe::egui;

use crate::app::DirigentApp;

/// Returns `true` if `name` is a valid filename for rename operations.
/// Rejects empty names, ".", "..", any path separators, and parent-traversal components.
fn validate_filename(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return false;
    }
    if trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains(std::path::MAIN_SEPARATOR)
    {
        return false;
    }
    // Reject if any path component is a parent traversal
    Path::new(trimmed)
        .components()
        .all(|c| !matches!(c, std::path::Component::ParentDir))
}

impl DirigentApp {
    /// Renders the rename dialog for a file or directory.
    pub(in crate::app) fn render_rename_dialog(&mut self, ctx: &egui::Context) {
        let Some(ref target) = self.rename_target else {
            return;
        };
        let target = target.clone();
        let is_dir = target.is_dir();
        let label = if is_dir { "Directory" } else { "File" };

        let mut confirm = false;
        let mut cancel = false;

        egui::Window::new(format!("Rename {}", label))
            .collapsible(false)
            .resizable(false)
            .default_size([400.0, 0.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .show(ctx, |ui| {
                let display = target
                    .strip_prefix(&self.project_root)
                    .unwrap_or(&target)
                    .to_string_lossy()
                    .to_string();
                ui.label(format!("Rename: {}", display));
                ui.add_space(8.0);
                let resp = ui.text_edit_singleline(&mut self.rename_buffer);
                if !self.rename_focus_requested {
                    resp.request_focus();
                    self.rename_focus_requested = true;
                }
                if resp.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && validate_filename(&self.rename_buffer)
                {
                    confirm = true;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let name_valid = validate_filename(&self.rename_buffer);
                        if ui
                            .add_enabled(name_valid, egui::Button::new("Rename"))
                            .clicked()
                        {
                            confirm = true;
                        }
                    });
                });
            });

        if cancel {
            self.rename_target = None;
        } else if confirm {
            let new_name = self.rename_buffer.trim().to_string();
            self.rename_target = None;
            self.execute_rename(&target, &new_name, is_dir);
        }
    }

    /// Performs the actual file/directory rename, updates open tabs, and sets a status message.
    fn execute_rename(&mut self, target: &Path, new_name: &str, is_dir: bool) {
        if !validate_filename(new_name) {
            self.set_status_message("Rename failed: invalid filename".to_string());
            return;
        }
        let sanitized = new_name.trim();
        let new_path = target.parent().unwrap_or(target).join(sanitized);
        if new_path == target {
            return;
        }
        match std::fs::rename(target, &new_path) {
            Ok(()) => {
                self.update_tabs_after_rename(target, &new_path, is_dir);
                let display = new_path
                    .strip_prefix(&self.project_root)
                    .unwrap_or(&new_path)
                    .to_string_lossy()
                    .to_string();
                self.set_status_message(format!("Renamed to: {}", display));
                self.reload_file_tree();
                self.reload_git_info();
            }
            Err(e) => {
                self.set_status_message(format!("Rename failed: {}", e));
            }
        }
    }

    /// Updates open tabs after a rename: replaces the tab for a single file,
    /// or updates paths for all tabs inside a renamed directory.
    fn update_tabs_after_rename(&mut self, old_path: &Path, new_path: &PathBuf, is_dir: bool) {
        if is_dir {
            for tab in &mut self.viewer.tabs {
                if let Ok(rel) = tab.file_path.strip_prefix(old_path) {
                    tab.file_path = new_path.join(rel);
                }
            }
            return;
        }
        let Some(idx) = self.viewer.find_tab(&old_path.to_path_buf()) else {
            return;
        };
        match super::super::create_tab_state(new_path) {
            Some(new_tab) => self.viewer.tabs[idx] = new_tab,
            None => {
                eprintln!(
                    "Warning: failed to create tab state for renamed file: {}",
                    new_path.display()
                );
                self.viewer.close_tab(idx);
            }
        }
    }

    /// Renders the confirmation dialog for deleting a file or directory.
    pub(in crate::app) fn render_file_delete_dialog(&mut self, ctx: &egui::Context) {
        let Some((ref path, is_dir)) = self.pending_file_delete else {
            return;
        };
        let path = path.clone();
        let label = if is_dir { "Directory" } else { "File" };

        let mut confirm = false;
        let mut cancel = false;

        egui::Window::new(format!("Delete {}", label))
            .collapsible(false)
            .resizable(false)
            .default_size([400.0, 0.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .show(ctx, |ui| {
                ui.label(format!(
                    "Are you sure you want to delete this {}?",
                    label.to_lowercase()
                ));
                ui.add_space(8.0);
                let display = path
                    .strip_prefix(&self.project_root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();
                ui.label(
                    egui::RichText::new(&display)
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
            self.pending_file_delete = None;
        } else if confirm {
            self.pending_file_delete = None;
            self.execute_file_delete(&path, is_dir);
        }
    }

    /// Performs the actual file/directory deletion, closes affected tabs, and sets a status message.
    fn execute_file_delete(&mut self, path: &Path, is_dir: bool) {
        let result = if is_dir {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };
        match result {
            Ok(()) => {
                self.close_tabs_for_deleted_path(path, is_dir);
                let display = path
                    .strip_prefix(&self.project_root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();
                self.set_status_message(format!("Deleted: {}", display));
                self.reload_file_tree();
                self.reload_git_info();
            }
            Err(e) => {
                self.set_status_message(format!("Delete failed: {}", e));
            }
        }
    }

    /// Closes open tabs for a deleted file, or all tabs inside a deleted directory.
    fn close_tabs_for_deleted_path(&mut self, path: &Path, is_dir: bool) {
        if !is_dir {
            if let Some(idx) = self.viewer.find_tab(&path.to_path_buf()) {
                self.viewer.close_tab(idx);
            }
        } else {
            let indices: Vec<usize> = self
                .viewer
                .tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| t.file_path.starts_with(path))
                .map(|(i, _)| i)
                .rev()
                .collect();
            for idx in indices {
                self.viewer.close_tab(idx);
            }
        }
    }
}
