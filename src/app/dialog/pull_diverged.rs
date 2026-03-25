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
                    if ui.button("Cancel").clicked() {
                        dismiss = true;
                    }
                });
            });

        if let Some(strategy) = chosen_strategy {
            self.git.show_pull_diverged = false;
            self.start_git_pull_with_strategy(strategy);
        } else if dismiss {
            self.git.show_pull_diverged = false;
        }
    }
}
