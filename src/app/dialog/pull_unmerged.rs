use eframe::egui;

use super::super::DirigentApp;

impl DirigentApp {
    pub(in crate::app) fn render_pull_unmerged_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_pull_unmerged {
            return;
        }

        let mut dismiss = false;

        egui::Window::new("Unmerged Files")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
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

                if ui.button("OK").clicked() {
                    dismiss = true;
                }
            });

        if dismiss {
            self.git.show_pull_unmerged = false;
        }
    }
}
