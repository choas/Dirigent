use eframe::egui;

use super::super::DirigentApp;

impl DirigentApp {
    pub(in crate::app) fn render_push_error_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_push_error {
            return;
        }

        let mut dismiss = false;
        let mut create_cue = false;

        egui::Window::new("Push Failed")
            .collapsible(false)
            .resizable(true)
            .default_width(480.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.label("Git push failed with the following error:");
                ui.add_space(8.0);

                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        let mut error_text = self.git.push_error_message.clone();
                        ui.add(
                            egui::TextEdit::multiline(&mut error_text)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY),
                        );
                    });

                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui
                        .button("Create Push Cue")
                        .on_hover_text("Create a cue for Claude to push and fix any problems")
                        .clicked()
                    {
                        create_cue = true;
                    }
                    if ui.button("Dismiss").clicked() {
                        dismiss = true;
                    }
                });
            });

        if create_cue {
            let cue_text = format!(
                "Git push failed. Fix the issue and push again.\n\nError:\n{}",
                self.git.push_error_message
            );
            match self.db.insert_global_cue(&cue_text) {
                Ok(_id) => {
                    self.reload_cues();
                    self.set_status_message("Created push cue in Inbox".to_string());
                }
                Err(e) => {
                    self.set_status_message(format!("Failed to create cue: {}", e));
                }
            }
            self.git.show_push_error = false;
            self.git.push_error_message.clear();
        } else if dismiss {
            self.git.show_push_error = false;
            self.git.push_error_message.clear();
        }
    }
}
