use eframe::egui;

use crate::git::{self, MergeOperation};

use super::super::DirigentApp;

impl DirigentApp {
    pub(in crate::app) fn render_merge_conflicts_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_merge_conflicts {
            return;
        }

        let mut dismiss = false;
        let mut abort = false;
        let mut continue_op = false;
        let mut file_to_open: Option<String> = None;
        let mut file_to_stage: Option<String> = None;
        let mut stage_all = false;

        let op = self.git.merge_operation;
        let op_label = match op {
            Some(MergeOperation::Merge) => "Merge",
            Some(MergeOperation::Rebase) => "Rebase",
            None => "Merge",
        };

        let has_conflicts = !self.git.conflict_files.is_empty();

        egui::Window::new(format!("{} Conflicts", op_label))
            .collapsible(false)
            .resizable(true)
            .default_width(420.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .show(ctx, |ui| {
                if has_conflicts {
                    ui.label(format!(
                        "{} conflict(s) need to be resolved:",
                        self.git.conflict_files.len()
                    ));
                    ui.add_space(8.0);

                    // List conflicted files
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for file in &self.git.conflict_files {
                                ui.horizontal(|ui| {
                                    ui.colored_label(egui::Color32::from_rgb(220, 120, 50), "C");
                                    if ui
                                        .link(file)
                                        .on_hover_text("Open file to resolve conflicts")
                                        .clicked()
                                    {
                                        file_to_open = Some(file.clone());
                                    }
                                    if ui
                                        .small_button("Stage")
                                        .on_hover_text("Mark as resolved (git add)")
                                        .clicked()
                                    {
                                        file_to_stage = Some(file.clone());
                                    }
                                });
                            }
                        });

                    ui.add_space(8.0);

                    if self.git.conflict_files.len() > 1 {
                        if ui
                            .button("Stage All")
                            .on_hover_text("Mark all files as resolved (git add)")
                            .clicked()
                        {
                            stage_all = true;
                        }
                        ui.add_space(4.0);
                    }

                    ui.separator();
                    ui.add_space(4.0);
                    ui.label(
                        "Open each file, resolve the conflict markers \
                         (<<<<<<< / ======= / >>>>>>>), save, then stage.",
                    );
                } else {
                    ui.label("All conflicts resolved!");
                    ui.add_space(4.0);
                    ui.label(format!(
                        "Click \"Continue {}\" to complete the operation.",
                        op_label
                    ));
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let continue_enabled = !has_conflicts;
                    if ui
                        .add_enabled(
                            continue_enabled,
                            egui::Button::new(format!("Continue {}", op_label)),
                        )
                        .on_hover_text(if has_conflicts {
                            "Resolve all conflicts first"
                        } else {
                            "Complete the merge/rebase"
                        })
                        .clicked()
                    {
                        continue_op = true;
                    }
                    if ui
                        .button(format!("Abort {}", op_label))
                        .on_hover_text("Discard the merge/rebase and return to the previous state")
                        .clicked()
                    {
                        abort = true;
                    }
                    if ui.button("Close").clicked() {
                        dismiss = true;
                    }
                });
            });

        // Handle actions after UI rendering (avoids borrow issues)
        if let Some(file) = file_to_open {
            let path = self.project_root.join(&file);
            self.viewer.open_file_without_history(path);
        }

        if let Some(file) = file_to_stage {
            match git::stage_files(&self.project_root, &[file.clone()]) {
                Ok(()) => {
                    self.set_status_message(format!("Staged: {}", file));
                    self.git.conflict_files = git::get_conflicted_files(&self.project_root);
                }
                Err(e) => {
                    self.set_status_message(format!("Failed to stage {}: {}", file, e));
                }
            }
        }

        if stage_all {
            let files = self.git.conflict_files.clone();
            match git::stage_files(&self.project_root, &files) {
                Ok(()) => {
                    self.set_status_message(format!("Staged {} file(s)", files.len()));
                    self.git.conflict_files = git::get_conflicted_files(&self.project_root);
                }
                Err(e) => {
                    self.set_status_message(format!("Failed to stage files: {}", e));
                }
            }
        }

        if continue_op {
            self.git.show_merge_conflicts = false;
            let result = match op {
                Some(MergeOperation::Rebase) => git::rebase_continue(&self.project_root),
                _ => git::merge_continue(&self.project_root),
            };
            match result {
                Ok(msg) => {
                    self.set_status_message(format!("{} complete: {}", op_label, msg));
                    self.reload_git_info();
                    self.reload_commit_history();
                }
                Err(e) => {
                    let err = e.to_string();
                    // If continue failed because there are still conflicts, reopen dialog
                    if err.contains("CONFLICT") || err.contains("unmerged") {
                        self.open_merge_conflict_dialog();
                    } else {
                        self.set_status_message(format!("{} continue failed: {}", op_label, err));
                    }
                }
            }
        }

        if abort {
            self.git.show_merge_conflicts = false;
            let result = match op {
                Some(MergeOperation::Rebase) => git::rebase_abort(&self.project_root),
                _ => git::merge_abort(&self.project_root),
            };
            match result {
                Ok(()) => {
                    self.set_status_message(format!("{} aborted", op_label));
                    self.reload_git_info();
                    self.reload_commit_history();
                }
                Err(e) => {
                    self.set_status_message(format!("{} abort failed: {}", op_label, e));
                }
            }
        }

        if dismiss {
            self.git.show_merge_conflicts = false;
        }
    }
}
