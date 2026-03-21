use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM, SPACE_XS};

impl DirigentApp {
    pub(in crate::app) fn render_create_pr_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_create_pr {
            return;
        }

        let mut dismiss = false;
        let mut do_create = false;

        let fs = self.settings.font_size;

        egui::Window::new("Create Pull Request")
            .collapsible(false)
            .resizable(true)
            .default_width(480.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .show(ctx, |ui| {
                let branch = self
                    .git
                    .info
                    .as_ref()
                    .map(|i| i.branch.as_str())
                    .unwrap_or("unknown");
                ui.label(
                    egui::RichText::new(format!("Branch: {}", branch))
                        .monospace()
                        .color(self.semantic.accent),
                );
                ui.add_space(SPACE_SM);

                // Title
                ui.label(egui::RichText::new("Title").strong());
                ui.add(
                    egui::TextEdit::singleline(&mut self.git.pr_title)
                        .desired_width(f32::INFINITY)
                        .hint_text("PR title"),
                );
                ui.add_space(SPACE_XS);

                // Base branch
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Base branch").strong());
                    ui.add(
                        egui::TextEdit::singleline(&mut self.git.pr_base)
                            .desired_width(120.0)
                            .hint_text("main"),
                    );
                });
                ui.add_space(SPACE_XS);

                // Body
                ui.label(egui::RichText::new("Description").strong());
                ui.add(
                    egui::TextEdit::multiline(&mut self.git.pr_body)
                        .desired_width(f32::INFINITY)
                        .desired_rows(8)
                        .hint_text("PR description (optional)")
                        .font(egui::TextStyle::Monospace),
                );
                ui.add_space(SPACE_XS);

                // Draft checkbox
                ui.checkbox(&mut self.git.pr_draft, "Create as draft");

                ui.add_space(SPACE_SM);

                // Info: will push
                let ahead = self.git.ahead_of_remote;
                if ahead > 0 {
                    ui.label(
                        egui::RichText::new(format!(
                            "\u{2191} {} commit{} will be pushed",
                            ahead,
                            if ahead == 1 { "" } else { "s" }
                        ))
                        .color(self.semantic.accent),
                    );
                    ui.add_space(SPACE_XS);
                }

                // Buttons
                ui.horizontal(|ui| {
                    let can_create = !self.git.pr_title.trim().is_empty() && !self.git.creating_pr;
                    let create_btn = egui::Button::new(
                        icon("\u{2191} Push & Create PR", fs).color(self.semantic.badge_text),
                    )
                    .fill(self.semantic.accent);
                    if ui
                        .add_enabled(can_create, create_btn)
                        .on_hover_text("Push the branch and create the pull request on GitHub")
                        .clicked()
                    {
                        do_create = true;
                    }
                    if ui.button("Cancel").clicked() {
                        dismiss = true;
                    }
                });
            });

        if do_create {
            self.start_create_pr();
        } else if dismiss {
            self.git.show_create_pr = false;
        }
    }
}
