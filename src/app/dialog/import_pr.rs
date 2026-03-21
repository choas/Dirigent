use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM, SPACE_XS};

impl DirigentApp {
    pub(in crate::app) fn render_import_pr_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_import_pr {
            return;
        }

        let mut dismiss = false;
        let mut do_import = false;

        let fs = self.settings.font_size;

        egui::Window::new("Import PR Findings")
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .show(ctx, |ui| {
                ui.label("Import actionable findings from a GitHub Pull Request review (e.g. CodeRabbit) as cues.");
                ui.add_space(SPACE_SM);

                // PR number input
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("PR #").strong());
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.git.import_pr_number)
                            .desired_width(80.0)
                            .hint_text("e.g. 1"),
                    );
                    // Auto-focus on first render
                    if self.git.import_pr_number.is_empty() {
                        resp.request_focus();
                    }
                });
                ui.add_space(SPACE_XS);

                ui.label(
                    egui::RichText::new("Findings will be tagged PR<number> and added to Inbox.")
                        .small()
                        .color(self.semantic.tertiary_text),
                );

                ui.add_space(SPACE_SM);

                // Buttons
                ui.horizontal(|ui| {
                    let valid_number = self
                        .git
                        .import_pr_number
                        .trim()
                        .parse::<u32>()
                        .map(|n| n > 0)
                        .unwrap_or(false);
                    let can_import = valid_number && !self.git.importing_pr;
                    let import_btn = egui::Button::new(
                        icon("\u{2193} Import Findings", fs).color(self.semantic.badge_text),
                    )
                    .fill(self.semantic.accent);
                    if ui
                        .add_enabled(can_import, import_btn)
                        .on_hover_text("Fetch PR review comments and create cues from findings")
                        .clicked()
                    {
                        do_import = true;
                    }
                    if ui.button("Cancel").clicked() {
                        dismiss = true;
                    }
                });
            });

        if do_import {
            self.start_import_pr_findings();
        } else if dismiss {
            self.git.show_import_pr = false;
        }
    }
}
