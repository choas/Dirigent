use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM, SPACE_XS};
use crate::db::CueStatus;
use crate::prompt_hints;
use crate::prompt_suggestions;

impl DirigentApp {
    // Feature 2: Global prompt input
    pub(in super::super) fn render_prompt_field(&mut self, ctx: &egui::Context) {
        let prompt_frame = egui::Frame::NONE
            .fill(self.semantic.prompt_surface())
            .inner_margin(egui::Margin::symmetric(SPACE_SM as i8, SPACE_SM as i8));

        egui::TopBottomPanel::bottom("prompt_field")
            .frame(prompt_frame)
            .show(ctx, |ui| {
                // Top border line to visually separate from content above
                let rect = ui.available_rect_before_wrap();
                ui.painter().hline(
                    rect.x_range(),
                    rect.top(),
                    egui::Stroke::new(1.0, self.semantic.prompt_border()),
                );

                self.render_prompt_attached_images(ui);
                self.render_prompt_input_row(ui);
                self.render_prompt_hints_and_suggestions(ui);
            });
    }

    /// Render attached image thumbnails above the prompt input.
    fn render_prompt_attached_images(&mut self, ui: &mut egui::Ui) {
        if self.global_prompt_images.is_empty() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new("Attached:")
                    .small()
                    .color(self.semantic.accent),
            );
            let mut remove_idx = None;
            for (i, path) in self.global_prompt_images.iter().enumerate() {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string_lossy().to_string());
                ui.label(egui::RichText::new(&name).monospace().small());
                if ui
                    .small_button("\u{2715}")
                    .on_hover_text("Remove")
                    .clicked()
                {
                    remove_idx = Some(i);
                }
            }
            if let Some(i) = remove_idx {
                self.global_prompt_images.remove(i);
            }
        });
        ui.add_space(SPACE_XS);
    }

    /// Render the main prompt input row (attach button, text edit, send button).
    fn render_prompt_input_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(icon("\u{25B6}", self.settings.font_size).color(self.semantic.accent));
            if ui
                .button(icon("+", self.settings.font_size))
                .on_hover_text("Attach files (or drag & drop)")
                .clicked()
            {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("All files", &["*"])
                    .pick_files()
                {
                    self.global_prompt_images.extend(paths);
                }
            }
            let input_response = ui.add(
                egui::TextEdit::multiline(&mut self.global_prompt_input)
                    .desired_width(ui.available_width() - 44.0)
                    .desired_rows(2)
                    .hint_text("Describe what you want...")
                    .font(egui::TextStyle::Monospace),
            );
            self.render_prompt_send_button(ui, &input_response);
        });
    }

    /// Render the send button and handle submit logic.
    fn render_prompt_send_button(&mut self, ui: &mut egui::Ui, input_response: &egui::Response) {
        ui.vertical_centered(|ui| {
            let input_h = input_response.rect.height();
            let btn_size = self.settings.font_size + 12.0;
            ui.add_space((input_h - btn_size) / 2.0);
            let send_btn = egui::Button::new(
                icon("\u{2191}", self.settings.font_size).color(self.semantic.accent_text()),
            )
            .fill(self.semantic.accent)
            .corner_radius(btn_size as u8 / 2)
            .min_size(egui::vec2(btn_size, btn_size));
            let btn_clicked = ui
                .add(send_btn)
                .on_hover_text("Create cue  (\u{2318}Enter to run)")
                .clicked();
            let (enter_submitted, cmd_enter) = if input_response.has_focus() {
                ui.input(|i| {
                    let pressed = i.key_pressed(egui::Key::Enter) && !i.modifiers.shift;
                    (
                        pressed && !i.modifiers.command,
                        pressed && i.modifiers.command,
                    )
                })
            } else {
                (false, false)
            };
            if (btn_clicked || enter_submitted || cmd_enter) && !self.global_prompt_input.is_empty()
            {
                self.submit_prompt(cmd_enter);
            }
        });
    }

    /// Submit the current prompt text as a new cue.
    fn submit_prompt(&mut self, run_immediately: bool) {
        let text = self.global_prompt_input.trim().to_string();
        if text.is_empty() {
            return;
        }
        let images: Vec<String> = self
            .global_prompt_images
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        match self.db.insert_cue(&text, "", 0, None, &images) {
            Ok(id) => {
                self.global_prompt_images.clear();
                self.global_prompt_input.clear();
                if run_immediately {
                    match self.db.update_cue_status(id, CueStatus::Ready) {
                        Ok(()) => {
                            self.claude.expand_running = true;
                            self.reload_cues();
                            self.trigger_claude(id);
                        }
                        Err(e) => {
                            self.set_status_message(format!("Failed to update cue status: {e}"));
                        }
                    }
                }
                self.reload_cues();
            }
            Err(e) => {
                self.set_status_message(format!("Failed to create cue: {e}"));
                self.reload_cues();
            }
        }
    }

    /// Render prompt refinement hints and suggestions below the input.
    fn render_prompt_hints_and_suggestions(&self, ui: &mut egui::Ui) {
        if !self.settings.prompt_suggestions_enabled {
            return;
        }
        let hints = prompt_hints::analyze(&self.global_prompt_input);
        if !hints.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for hint in &hints {
                    ui.label(
                        egui::RichText::new(format!("\u{26A0} {}", hint.label))
                            .small()
                            .color(self.semantic.warning),
                    )
                    .on_hover_text(hint.detail);
                }
            });
        }

        let suggestions = prompt_suggestions::analyse_prompt(&self.global_prompt_input, false);
        if !suggestions.is_empty() {
            ui.add_space(SPACE_XS);
            for suggestion in &suggestions {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("\u{26A0} {}", suggestion.label))
                            .small()
                            .color(self.semantic.warning),
                    );
                    ui.label(
                        egui::RichText::new(suggestion.detail)
                            .small()
                            .color(self.semantic.muted_text()),
                    );
                });
            }
        }
    }
}
