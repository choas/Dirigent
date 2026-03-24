use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM, SPACE_XS};

impl DirigentApp {
    pub(in crate::app) fn render_import_pr_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_import_pr {
            return;
        }

        let mut dismiss = false;
        let mut do_import = false;

        let is_refresh = self.is_pr_refresh();

        let title = if is_refresh {
            "Refresh PR Findings"
        } else {
            "Import PR Findings"
        };

        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .show(ctx, |ui| {
                self.render_import_pr_body(ui, is_refresh);
                self.render_import_pr_buttons(ui, is_refresh, &mut do_import, &mut dismiss);
            });

        if do_import {
            self.start_import_pr_findings();
        } else if dismiss {
            self.git.show_import_pr = false;
        }
    }

    /// Checks whether the current PR number already has imported cues (i.e. this is a refresh).
    fn is_pr_refresh(&self) -> bool {
        self.git
            .import_pr_number
            .trim()
            .parse::<u32>()
            .ok()
            .map(|n| {
                let tag = format!("PR{}", n);
                self.cues.iter().any(|c| c.tag.as_deref() == Some(&tag))
            })
            .unwrap_or(false)
    }

    /// Renders the description text, PR number input, and hint label.
    fn render_import_pr_body(&mut self, ui: &mut egui::Ui, is_refresh: bool) {
        if is_refresh {
            ui.label("Re-fetch findings from the PR to check for new review comments (e.g. after CodeRabbit re-reviews).");
        } else {
            ui.label("Import actionable findings from a GitHub Pull Request review (e.g. CodeRabbit) as cues.");
        }
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

        let hint = if is_refresh {
            "Already-imported findings will be skipped. Only new comments are added."
        } else {
            "Findings will be tagged PR<number> and added to Inbox."
        };
        ui.label(
            egui::RichText::new(hint)
                .small()
                .color(self.semantic.tertiary_text),
        );

        ui.add_space(SPACE_SM);
    }

    /// Renders the Import/Refresh and Cancel buttons.
    fn render_import_pr_buttons(
        &self,
        ui: &mut egui::Ui,
        is_refresh: bool,
        do_import: &mut bool,
        dismiss: &mut bool,
    ) {
        let fs = self.settings.font_size;
        ui.horizontal(|ui| {
            let valid_number = self
                .git
                .import_pr_number
                .trim()
                .parse::<u32>()
                .map(|n| n > 0)
                .unwrap_or(false);
            let can_import = valid_number && !self.git.importing_pr;
            let btn_label = if is_refresh {
                "\u{21BB} Refresh Findings"
            } else {
                "\u{2193} Import Findings"
            };
            let import_btn = egui::Button::new(
                icon(btn_label, fs).color(self.semantic.badge_text),
            )
            .fill(self.semantic.accent);
            if ui
                .add_enabled(can_import, import_btn)
                .on_hover_text("Fetch PR review comments and create cues from findings")
                .clicked()
            {
                *do_import = true;
            }
            if ui.button("Cancel").clicked() {
                *dismiss = true;
            }
        });
    }
}
