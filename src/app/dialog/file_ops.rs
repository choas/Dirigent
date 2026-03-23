use eframe::egui;

use crate::app::DirigentApp;

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
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
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
                        let name_valid =
                            !self.rename_buffer.is_empty() && !self.rename_buffer.contains('/');
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
            let new_path = target.parent().unwrap_or(&target).join(&new_name);
            self.rename_target = None;
            if new_path == target {
                return;
            }
            match std::fs::rename(&target, &new_path) {
                Ok(()) => {
                    // Update any open tab that referenced the old path
                    if !is_dir {
                        if let Some(idx) = self.viewer.find_tab(&target) {
                            if let Some(new_tab) = super::super::create_tab_state(&new_path) {
                                self.viewer.tabs[idx] = new_tab;
                            }
                        }
                    } else {
                        // Update tabs whose files were inside the renamed directory
                        for tab in &mut self.viewer.tabs {
                            if tab.file_path.starts_with(&target) {
                                if let Ok(rel) = tab.file_path.strip_prefix(&target) {
                                    let updated = new_path.join(rel);
                                    tab.file_path = updated;
                                }
                            }
                        }
                    }
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
            let result = if is_dir {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            match result {
                Ok(()) => {
                    // Close the tab if the deleted file was open
                    if !is_dir {
                        if let Some(idx) = self.viewer.find_tab(&path) {
                            self.viewer.close_tab(idx);
                        }
                    } else {
                        // Close any tabs whose files were inside this directory
                        let indices: Vec<usize> = self
                            .viewer
                            .tabs
                            .iter()
                            .enumerate()
                            .filter(|(_, t)| t.file_path.starts_with(&path))
                            .map(|(i, _)| i)
                            .rev()
                            .collect();
                        for idx in indices {
                            self.viewer.close_tab(idx);
                        }
                    }
                    let display = path
                        .strip_prefix(&self.project_root)
                        .unwrap_or(&path)
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
    }
}
