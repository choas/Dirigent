use eframe::egui;

use super::super::DirigentApp;

impl DirigentApp {
    pub(in crate::app) fn render_pull_unmerged_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_pull_unmerged {
            return;
        }

        let mut dismiss = false;
        let mut create_cue = false;

        egui::Window::new("Unmerged Files")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.label(
                    "Pull failed because there are unmerged files in \
                     your working tree from a previous conflict.",
                );
                ui.add_space(8.0);
                ui.label("To fix this, resolve the conflicts in a terminal:");
                ui.add_space(4.0);
                ui.code("git add <resolved-files>  then  git commit");
                ui.add_space(4.0);
                ui.label("Or abort the merge / rebase:");
                ui.add_space(4.0);
                ui.code("git merge --abort   or   git rebase --abort");
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui
                        .button("Create Cue")
                        .on_hover_text("Create a cue for Claude to resolve the unmerged files")
                        .clicked()
                    {
                        create_cue = true;
                    }
                    if ui.button("OK").clicked() {
                        dismiss = true;
                    }
                });
            });

        if create_cue {
            let cue_text =
                "Git pull failed because there are unmerged files from a previous conflict. \
                 Resolve the conflicts and complete the merge or rebase."
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
            self.git.show_pull_unmerged = false;
        } else if dismiss {
            self.git.show_pull_unmerged = false;
        }
    }
}
