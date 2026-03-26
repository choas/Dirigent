use eframe::egui;

use crate::app::{icon, DirigentApp, SPACE_MD, SPACE_SM};
use crate::settings::{self, default_playbook};

impl DirigentApp {
    pub(in crate::app) fn render_settings_playbook_section(&mut self, ui: &mut egui::Ui, fs: f32) {
        ui.add_space(SPACE_MD);
        ui.separator();
        ui.add_space(SPACE_SM);
        ui.horizontal(|ui| {
            let arrow = ["\u{25B6}", "\u{25BC}"][self.playbook_expanded as usize];
            if ui
                .button(icon(&format!("{} Playbook", arrow), fs))
                .clicked()
            {
                self.playbook_expanded = !self.playbook_expanded;
            }
            ui.label(
                egui::RichText::new(format!("({} plays)", self.settings.playbook.len()))
                    .small()
                    .color(self.semantic.secondary_text),
            );
            if self.playbook_expanded {
                self.render_settings_playbook_actions(ui);
            }
        });

        if self.playbook_expanded {
            self.render_settings_playbook_list(ui, fs);
        }
    }

    fn render_settings_playbook_actions(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("+ Add Play").clicked() {
                self.settings.playbook.push(settings::Play {
                    name: "New Play".to_string(),
                    prompt: String::new(),
                });
            }
            if ui.small_button("Reset Defaults").clicked() {
                self.settings.playbook = default_playbook();
            }
        });
    }

    fn render_settings_playbook_list(&mut self, ui: &mut egui::Ui, fs: f32) {
        ui.add_space(SPACE_SM);

        if self.settings.playbook.is_empty() {
            ui.label(
                egui::RichText::new("No plays configured. Add a play or reset to defaults.")
                    .italics()
                    .color(self.semantic.tertiary_text),
            );
        }

        let mut remove_play_idx = None;
        let num_plays = self.settings.playbook.len();

        for i in 0..num_plays {
            self.render_settings_play_card(ui, i, fs, &mut remove_play_idx);
            ui.add_space(SPACE_SM);
        }

        if let Some(idx) = remove_play_idx {
            self.settings.playbook.remove(idx);
        }
    }

    fn render_settings_play_card(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        fs: f32,
        remove_play_idx: &mut Option<usize>,
    ) {
        self.semantic.card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.playbook[i].name)
                        .desired_width(200.0)
                        .font(egui::TextStyle::Body),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(icon("\u{2715}", fs))
                        .on_hover_text("Delete play")
                        .clicked()
                    {
                        *remove_play_idx = Some(i);
                    }
                });
            });
            ui.add(
                egui::TextEdit::multiline(&mut self.settings.playbook[i].prompt)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3)
                    .hint_text("Prompt text...")
                    .font(egui::TextStyle::Monospace),
            );
        });
    }
}
