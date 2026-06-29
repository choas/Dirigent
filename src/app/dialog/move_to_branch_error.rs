use eframe::egui;

use super::super::DirigentApp;

impl DirigentApp {
    pub(in crate::app) fn render_move_to_branch_error_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_move_to_branch_error {
            return;
        }

        let mut dismiss = false;
        let mut try_again = false;
        let mut create_cue = false;

        let suggestion = suggestion_for(&self.git.move_to_branch_error_message);

        egui::Window::new("Move to Branch Failed")
            .collapsible(false)
            .resizable(true)
            .default_width(480.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.label("Moving the commits to a new branch failed:");
                ui.add_space(8.0);

                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .show(ui, |ui| {
                        let mut error_text = self.git.move_to_branch_error_message.clone();
                        ui.add(
                            egui::TextEdit::multiline(&mut error_text)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY),
                        );
                    });

                ui.add_space(10.0);

                ui.label(egui::RichText::new("Suggestion").strong());
                ui.add_space(2.0);
                ui.label(egui::RichText::new(suggestion.text).color(self.semantic.tertiary_text));

                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui
                        .button("Try Again")
                        .on_hover_text("Reopen the Move Commits dialog")
                        .clicked()
                    {
                        try_again = true;
                    }
                    if ui
                        .button("Create Cue")
                        .on_hover_text("Create a cue for Claude to resolve the problem")
                        .clicked()
                    {
                        create_cue = true;
                    }
                    if ui.button("Dismiss").clicked() {
                        dismiss = true;
                    }
                });
            });

        if try_again {
            self.git.show_move_to_branch_error = false;
            self.git.move_to_branch_error_message.clear();
            self.open_move_to_branch_dialog();
        } else if create_cue {
            let cue_text = format!(
                "Moving commits to a new branch failed. Resolve the underlying problem so the \
                 commits can be moved.\n\nError:\n{}",
                self.git.move_to_branch_error_message
            );
            match self.db.insert_global_cue(&cue_text) {
                Ok(_id) => {
                    self.reload_cues();
                    self.set_status_message("Created move-to-branch cue in Inbox".to_string());
                }
                Err(e) => {
                    self.set_status_message(format!("Failed to create cue: {}", e));
                }
            }
            self.git.show_move_to_branch_error = false;
            self.git.move_to_branch_error_message.clear();
        } else if dismiss {
            self.git.show_move_to_branch_error = false;
            self.git.move_to_branch_error_message.clear();
        }
    }
}

struct Suggestion {
    text: &'static str,
}

/// Map a move-to-branch failure message to a concrete, actionable suggestion.
fn suggestion_for(message: &str) -> Suggestion {
    let lower = message.to_lowercase();
    let text = if lower.contains("uncommitted changes") {
        "Your working tree has uncommitted changes, which would be lost when the current branch \
         is reset. Commit them (or stash them with `git stash`), then move the commits again."
    } else if lower.contains("no upstream") {
        "The current branch has no upstream (remote-tracking) branch, so there is nothing to \
         reset it back to. Push the branch first (or set its upstream), then try again."
    } else if lower.contains("already exists") {
        "A branch with that name already exists. Choose a different branch name and try again."
    } else if lower.contains("not on a branch") || lower.contains("determine head") {
        "HEAD is not on a branch (detached HEAD). Check out a branch first, then try again."
    } else {
        "Resolve the problem reported above, then try moving the commits again. You can also \
         create a cue to have Claude investigate and fix it."
    };
    Suggestion { text }
}
