use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM, SPACE_XS};

impl DirigentApp {
    pub(in crate::app) fn render_commit_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_commit_dialog {
            return;
        }

        let mut dismiss = false;
        let mut do_commit = false;

        let fs = self.settings.font_size;

        egui::Window::new("Commit")
            .collapsible(false)
            .resizable(false)
            .default_width(450.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                if self.git.commit_review_cue_id.is_some() {
                    ui.label("Commit the reviewed changes with a message.");
                } else {
                    ui.label("Describe the current working-copy commit and start a new change.");
                }
                ui.add_space(SPACE_XS);

                if let Some(ref info) = self.git.info {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Branch:")
                                .small()
                                .color(self.semantic.tertiary_text),
                        );
                        ui.label(
                            egui::RichText::new(&info.branch)
                                .small()
                                .strong()
                                .color(self.semantic.secondary_text),
                        );
                    });
                    let summary = crate::git::format_status_summary(info);
                    if !summary.is_empty() {
                        ui.label(
                            egui::RichText::new(summary)
                                .small()
                                .color(self.semantic.tertiary_text),
                        );
                    }
                    ui.add_space(SPACE_XS);
                }

                ui.label(egui::RichText::new("Commit message").strong());
                let response = ui.add(
                    egui::TextEdit::multiline(&mut self.git.commit_message_input)
                        .desired_width(f32::INFINITY)
                        .desired_rows(3)
                        .hint_text("Describe what changed"),
                );

                if self.git.commit_needs_focus {
                    response.request_focus();
                    self.git.commit_needs_focus = false;
                }

                if response.has_focus()
                    && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter))
                    && !self.git.commit_message_input.trim().is_empty()
                {
                    do_commit = true;
                }

                ui.add_space(SPACE_XS);

                ui.label(
                    egui::RichText::new("Cmd+Enter to commit")
                        .small()
                        .color(self.semantic.tertiary_text),
                );

                ui.add_space(SPACE_SM);

                ui.horizontal(|ui| {
                    let can_commit = !self.git.commit_message_input.trim().is_empty();
                    let commit_btn = egui::Button::new(
                        icon("\u{2714} Commit", fs).color(self.semantic.badge_text),
                    )
                    .fill(self.semantic.accent);
                    if ui
                        .add_enabled(can_commit, commit_btn)
                        .on_hover_text("Commit working-copy changes")
                        .clicked()
                    {
                        do_commit = true;
                    }
                    if ui.button("Cancel").clicked() {
                        dismiss = true;
                    }
                });

                ui.add_space(SPACE_XS);
            });

        if do_commit {
            self.start_jj_commit();
        } else if dismiss {
            self.git.show_commit_dialog = false;
            self.git.commit_review_cue_id = None;
        }
    }
}
