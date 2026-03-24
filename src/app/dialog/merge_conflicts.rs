use eframe::egui;

use crate::git::{self, MergeOperation};

use super::super::DirigentApp;

/// Actions collected during UI rendering, applied after the closure ends.
struct MergeConflictActions {
    dismiss: bool,
    abort: bool,
    continue_op: bool,
    file_to_open: Option<String>,
    file_to_stage: Option<String>,
    stage_all: bool,
}

impl MergeConflictActions {
    fn new() -> Self {
        Self {
            dismiss: false,
            abort: false,
            continue_op: false,
            file_to_open: None,
            file_to_stage: None,
            stage_all: false,
        }
    }
}

impl DirigentApp {
    pub(in crate::app) fn render_merge_conflicts_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_merge_conflicts {
            return;
        }

        let mut actions = MergeConflictActions::new();

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
                Self::render_conflict_body(
                    ui,
                    &self.git.conflict_files,
                    has_conflicts,
                    op_label,
                    &mut actions,
                );
            });

        self.apply_merge_conflict_actions(actions, op, op_label, has_conflicts);
    }

    fn render_conflict_body(
        ui: &mut egui::Ui,
        conflict_files: &[String],
        has_conflicts: bool,
        op_label: &str,
        actions: &mut MergeConflictActions,
    ) {
        if has_conflicts {
            Self::render_conflict_file_list(ui, conflict_files, actions);
        } else {
            ui.label("All conflicts resolved!");
            ui.add_space(4.0);
            ui.label(format!(
                "Click \"Continue {}\" to complete the operation.",
                op_label
            ));
        }

        ui.add_space(12.0);
        Self::render_conflict_action_buttons(ui, has_conflicts, op_label, actions);
    }

    fn render_conflict_file_list(
        ui: &mut egui::Ui,
        conflict_files: &[String],
        actions: &mut MergeConflictActions,
    ) {
        ui.label(format!(
            "{} conflict(s) need to be resolved:",
            conflict_files.len()
        ));
        ui.add_space(8.0);

        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                for file in conflict_files {
                    ui.horizontal(|ui| {
                        ui.colored_label(egui::Color32::from_rgb(220, 120, 50), "C");
                        if ui
                            .link(file)
                            .on_hover_text("Open file to resolve conflicts")
                            .clicked()
                        {
                            actions.file_to_open = Some(file.clone());
                        }
                        if ui
                            .small_button("Stage")
                            .on_hover_text("Mark as resolved (git add)")
                            .clicked()
                        {
                            actions.file_to_stage = Some(file.clone());
                        }
                    });
                }
            });

        ui.add_space(8.0);

        if conflict_files.len() > 1
            && ui
                .button("Stage All")
                .on_hover_text("Mark all files as resolved (git add)")
                .clicked()
        {
            actions.stage_all = true;
        }
        if conflict_files.len() > 1 {
            ui.add_space(4.0);
        }

        ui.separator();
        ui.add_space(4.0);
        ui.label(
            "Open each file, resolve the conflict markers \
             (<<<<<<< / ======= / >>>>>>>), save, then stage.",
        );
    }

    fn render_conflict_action_buttons(
        ui: &mut egui::Ui,
        has_conflicts: bool,
        op_label: &str,
        actions: &mut MergeConflictActions,
    ) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !has_conflicts,
                    egui::Button::new(format!("Continue {}", op_label)),
                )
                .on_hover_text(if has_conflicts {
                    "Resolve all conflicts first"
                } else {
                    "Complete the merge/rebase"
                })
                .clicked()
            {
                actions.continue_op = true;
            }
            if ui
                .button(format!("Abort {}", op_label))
                .on_hover_text("Discard the merge/rebase and return to the previous state")
                .clicked()
            {
                actions.abort = true;
            }
            if ui.button("Close").clicked() {
                actions.dismiss = true;
            }
        });
    }

    fn apply_merge_conflict_actions(
        &mut self,
        actions: MergeConflictActions,
        op: Option<MergeOperation>,
        op_label: &str,
        _has_conflicts: bool,
    ) {
        if let Some(file) = actions.file_to_open {
            let path = self.project_root.join(&file);
            self.viewer.open_file_without_history(path);
        }

        if let Some(file) = actions.file_to_stage {
            self.handle_stage_file(file);
        }

        if actions.stage_all {
            self.handle_stage_all();
        }

        if actions.continue_op {
            self.handle_merge_continue(op, op_label);
        }

        if actions.abort {
            self.handle_merge_abort(op, op_label);
        }

        if actions.dismiss {
            self.git.show_merge_conflicts = false;
        }
    }

    fn handle_stage_file(&mut self, file: String) {
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

    fn handle_stage_all(&mut self) {
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

    fn handle_merge_continue(&mut self, op: Option<MergeOperation>, op_label: &str) {
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
                if err.contains("CONFLICT") || err.contains("unmerged") {
                    self.open_merge_conflict_dialog();
                } else {
                    self.set_status_message(format!("{} continue failed: {}", op_label, err));
                }
            }
        }
    }

    fn handle_merge_abort(&mut self, op: Option<MergeOperation>, op_label: &str) {
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
}
