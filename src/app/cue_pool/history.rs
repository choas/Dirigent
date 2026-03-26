use eframe::egui;

use super::super::{icon, DirigentApp};
use super::ReuseCueData;

impl DirigentApp {
    pub(in crate::app) fn render_prompt_history(
        &mut self,
        ui: &mut egui::Ui,
    ) -> Option<ReuseCueData> {
        self.render_prompt_history_search_bar(ui);
        self.render_prompt_history_results(ui)
    }

    fn render_prompt_history_search_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            self.render_prompt_history_toggle(ui);
            if self.prompt_history_active {
                self.render_prompt_history_input(ui);
            }
        });
    }

    fn render_prompt_history_toggle(&mut self, ui: &mut egui::Ui) {
        let (search_icon, hover) = if self.prompt_history_active {
            ("\u{2715}", "Close search")
        } else {
            ("\u{1F50D}", "Search past prompts")
        };
        if ui.small_button(search_icon).on_hover_text(hover).clicked() {
            self.prompt_history_active = !self.prompt_history_active;
            if !self.prompt_history_active {
                self.prompt_history_query.clear();
                self.prompt_history_results.clear();
            }
        }
    }

    fn render_prompt_history_input(&mut self, ui: &mut egui::Ui) {
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.prompt_history_query)
                .desired_width(ui.available_width())
                .hint_text("Search past cues...")
                .font(egui::TextStyle::Small),
        );
        if response.changed() && self.prompt_history_query.len() >= 2 {
            self.prompt_history_results = self
                .db
                .search_cue_history(&self.prompt_history_query, 10)
                .unwrap_or_default();
        } else if self.prompt_history_query.len() < 2 {
            self.prompt_history_results.clear();
        }
    }

    fn render_prompt_history_results(&mut self, ui: &mut egui::Ui) -> Option<ReuseCueData> {
        if !self.prompt_history_active || self.prompt_history_results.is_empty() {
            return None;
        }
        let mut reuse_cue: Option<ReuseCueData> = None;
        egui::Frame::NONE
            .inner_margin(egui::Margin::same(4))
            .corner_radius(4)
            .fill(self.semantic.selection_bg())
            .show(ui, |ui| {
                for (_id, text, file_path, line_number, line_number_end, images) in
                    &self.prompt_history_results
                {
                    ui.horizontal(|ui| {
                        let preview: String =
                            text.lines().next().unwrap_or("").chars().take(60).collect();
                        let location = if file_path.is_empty() {
                            "Global".to_string()
                        } else {
                            file_path.clone()
                        };
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&preview).small());
                            ui.label(
                                egui::RichText::new(&location)
                                    .small()
                                    .color(self.semantic.muted_text()),
                            );
                        });
                        if ui
                            .small_button(icon("\u{21A9} Reuse", self.settings.font_size))
                            .on_hover_text("Create a new cue with this text")
                            .clicked()
                        {
                            reuse_cue = Some((
                                text.clone(),
                                file_path.clone(),
                                *line_number,
                                *line_number_end,
                                images.clone(),
                            ));
                        }
                    });
                    ui.add_space(2.0);
                }
            });
        reuse_cue
    }

    pub(in crate::app) fn handle_reuse_cue(&mut self, reuse_cue: Option<ReuseCueData>) {
        let Some((text, file_path, line_number, line_number_end, images)) = reuse_cue else {
            return;
        };
        match self
            .db
            .insert_cue(&text, &file_path, line_number, line_number_end, &images)
        {
            Ok(_) => {
                self.reload_cues();
                self.prompt_history_active = false;
                self.prompt_history_query.clear();
                self.prompt_history_results.clear();
            }
            Err(e) => {
                eprintln!("Failed to insert cue: {e}");
            }
        }
    }
}
