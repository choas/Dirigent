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
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let Some(pending) = self.pending_play.as_mut() else {
                    return;
                };
                Self::render_play_variable_inputs(ui, pending);

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
            self.submit_pending_play();
        }
    }

    /// Render input widgets for all non-auto-resolved variables.
    fn render_play_variable_inputs(ui: &mut egui::Ui, pending: &mut super::super::PendingPlay) {
        let var_count = pending.variables.len();
        for i in 0..var_count {
            if pending.auto_resolved.contains_key(&i) {
                continue;
            }

            let has_options = !pending.variables[i].options.is_empty();
            let var_name = pending.variables[i].name.clone();

            ui.add_space(SPACE_SM);
            ui.label(egui::RichText::new(&var_name).strong());

            if !has_options {
                Self::render_freetext_input(ui, &mut pending.custom_text[i], &var_name);
            } else {
                Self::render_combo_input(ui, pending, i);
            }
        }
    }

    /// Render a free-text input for a variable with no predefined options.
    fn render_freetext_input(ui: &mut egui::Ui, text: &mut String, var_name: &str) {
        ui.add(
            egui::TextEdit::singleline(text)
                .desired_width(300.0)
                .hint_text(format!("Enter {}...", var_name.to_lowercase())),
        );
    }

    /// Render a combo box with predefined options plus an "Other" free-text fallback.
    fn render_combo_input(ui: &mut egui::Ui, pending: &mut super::super::PendingPlay, i: usize) {
        let var = &pending.variables[i];
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

        if *sel == other_idx {
            let var_name = pending.variables[i].name.clone();
            Self::render_custom_input(ui, &mut pending.custom_text[i], &var_name);
        }
    }

    /// Render the custom text input shown when "Other" is selected.
    fn render_custom_input(ui: &mut egui::Ui, text: &mut String, var_name: &str) {
        ui.add(
            egui::TextEdit::singleline(text)
                .desired_width(300.0)
                .hint_text(format!("Custom {}...", var_name.to_lowercase())),
        );
    }

    /// Resolve all variable values and create the cue.
    fn submit_pending_play(&mut self) {
        let Some(pending) = self.pending_play.take() else {
            return;
        };
        let resolved: Vec<(String, String)> = pending
            .variables
            .iter()
            .enumerate()
            .map(|(i, var)| {
                let value = resolve_variable_value(&pending, i, var);
                (var.token.clone(), value)
            })
            .collect();
        let final_prompt = settings::substitute_play_variables(&pending.prompt, &resolved);
        match self.db.insert_global_cue(&final_prompt) {
            Ok(_) => self.reload_cues(),
            Err(e) => {
                self.set_status_message(format!("Failed to create cue: {e}"));
                self.pending_play = Some(pending);
            }
        }
    }
}

/// Determine the resolved value for a single play variable.
fn resolve_variable_value(
    pending: &super::super::PendingPlay,
    i: usize,
    var: &settings::PlayVariable,
) -> String {
    if let Some(auto) = pending.auto_resolved.get(&i) {
        return auto.clone();
    }
    if var.options.is_empty() {
        return pending.custom_text[i].clone();
    }
    if pending.selected[i] < var.options.len() {
        return var.options[pending.selected[i]].clone();
    }
    // "Other" selected — use custom text.
    pending.custom_text[i].clone()
}
