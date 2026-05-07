use eframe::egui;

use crate::git::PullStrategy;

use super::super::DirigentApp;

impl DirigentApp {
    pub(in crate::app) fn render_pull_diverged_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_pull_diverged {
            return;
        }

        let mut dismiss = false;
        let mut chosen_strategy: Option<PullStrategy> = None;
        let mut create_cue = false;

        egui::Window::new("Branches Have Diverged")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.label(
                    "The remote branch has diverged from your local branch \
                     and cannot be fast-forwarded.",
                );
                ui.add_space(8.0);
                ui.label("How would you like to resolve this?");
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui
                        .button("Merge")
                        .on_hover_text("Create a merge commit combining both branches")
                        .clicked()
                    {
                        chosen_strategy = Some(PullStrategy::Merge);
                    }
                    if ui
                        .button("Rebase")
                        .on_hover_text("Replay your local commits on top of the remote branch")
                        .clicked()
                    {
                        chosen_strategy = Some(PullStrategy::Rebase);
                    }
                    if ui
                        .button("Create Cue")
                        .on_hover_text("Create a cue for Claude to resolve the diverged branches")
                        .clicked()
                    {
                        create_cue = true;
                    }
                    if ui.button("Cancel").clicked() {
                        dismiss = true;
                    }
                });
            });

        if create_cue {
            let cue_text = "Git pull failed because branches have diverged. \
                 Resolve the divergence and pull again."
                .to_string();
            match self.db.insert_global_cue(&cue_text) {
                Ok(_id) => {
                    self.reload_cues();
                    self.set_status_message("Created pull cue in Inbox".to_string());
                }
                Err(e) => {
                    self.set_status_message(format!("Failed to create cue: {}", e));
                }
            }
            self.git.show_pull_diverged = false;
        } else if let Some(strategy) = chosen_strategy {
            self.git.show_pull_diverged = false;
            self.start_git_pull_with_strategy(strategy);
        } else if dismiss {
            self.git.show_pull_diverged = false;
        }
    }
}
