use eframe::egui;

use super::super::{DirigentApp, SPACE_SM};
use crate::settings;

impl DirigentApp {
    /// Render a modal dialog for filling in play template variables.
    pub(in crate::app) fn render_play_variables_dialog(&mut self, ctx: &egui::Context) {
        if self.pending_play.is_none() {
            return;
        }

        let mut submit = false;
        let mut cancel = false;

        egui::Window::new("Play Settings")
            .collapsible(false)
            .resizable(false)
            .default_size([400.0, 200.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .show(ctx, |ui| {
                let pending = self.pending_play.as_mut().unwrap();

                for (i, var) in pending.variables.iter().enumerate() {
                    // Skip auto-resolved variables.
                    if pending.auto_resolved.contains_key(&i) {
                        continue;
                    }

                    ui.add_space(SPACE_SM);
                    ui.label(egui::RichText::new(&var.name).strong());

                    if var.options.is_empty() {
                        // Free-text input.
                        ui.add(
                            egui::TextEdit::singleline(&mut pending.custom_text[i])
                                .desired_width(300.0)
                                .hint_text(format!("Enter {}...", var.name.to_lowercase())),
                        );
                    } else {
                        // Combo box with predefined options + "Other" for custom.
                        let sel = &mut pending.selected[i];
                        let other_idx = var.options.len();
                        let display = if *sel < var.options.len() {
                            var.options[*sel].as_str()
                        } else {
                            "Other..."
                        };
                        egui::ComboBox::from_id_salt(format!("play_var_{}", i))
                            .selected_text(display)
                            .width(300.0)
                            .show_ui(ui, |ui| {
                                for (j, opt) in var.options.iter().enumerate() {
                                    ui.selectable_value(sel, j, opt);
                                }
                                ui.selectable_value(sel, other_idx, "Other...");
                            });

                        // Show custom text input when "Other" is selected.
                        if *sel == other_idx {
                            ui.add(
                                egui::TextEdit::singleline(&mut pending.custom_text[i])
                                    .desired_width(300.0)
                                    .hint_text(format!("Custom {}...", var.name.to_lowercase())),
                            );
                        }
                    }
                }

                ui.add_space(SPACE_SM * 2.0);
                ui.horizontal(|ui| {
                    if ui.button("Create Cue").clicked() {
                        submit = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });

        if cancel {
            self.pending_play = None;
            return;
        }

        if submit {
            let pending = self.pending_play.take().unwrap();
            let resolved: Vec<(String, String)> = pending
                .variables
                .iter()
                .enumerate()
                .map(|(i, var)| {
                    let value = if let Some(auto) = pending.auto_resolved.get(&i) {
                        auto.clone()
                    } else if var.options.is_empty() {
                        // Free-text variable.
                        pending.custom_text[i].clone()
                    } else if pending.selected[i] < var.options.len() {
                        var.options[pending.selected[i]].clone()
                    } else {
                        // "Other" selected — use custom text.
                        pending.custom_text[i].clone()
                    };
                    (var.token.clone(), value)
                })
                .collect();
            let final_prompt = settings::substitute_play_variables(&pending.prompt, &resolved);
            let _ = self.db.insert_cue(&final_prompt, "", 0, None, &[]);
            self.reload_cues();
        }
    }
}
