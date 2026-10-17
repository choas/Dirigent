use eframe::egui;

use super::super::{DirigentApp, SPACE_SM, SPACE_XS, icon};
use crate::db::CueStatus;
use crate::prompt_hints;
use crate::prompt_suggestions;
use crate::settings::CliProvider;

impl DirigentApp {
    // Feature 2: Global prompt input
    pub(in super::super) fn render_prompt_field(&mut self, ui: &mut egui::Ui) {
        let prompt_frame = egui::Frame::NONE
            .fill(self.semantic.prompt_surface())
            .inner_margin(egui::Margin::symmetric(SPACE_SM as i8, SPACE_SM as i8));

        egui::Panel::bottom("prompt_field")
            .frame(prompt_frame)
            .show_inside(ui, |ui| {
                // Top border line to visually separate from content above
                let rect = ui.available_rect_before_wrap();
                ui.painter().hline(
                    rect.x_range(),
                    rect.top(),
                    egui::Stroke::new(1.0_f32, self.semantic.prompt_border()),
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
                && let Some(paths) = rfd::FileDialog::new()
                    .add_filter("All files", &["*"])
                    .pick_files()
            {
                self.global_prompt_images.extend(paths);
            }
            self.render_herdr_button(ui);
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

    /// Experimental: run this prompt as native Claude Code in a herdr pane,
    /// whatever the configured provider. Shown when herdr was found (or its
    /// path set) and the provider isn't Herdr already.
    fn render_herdr_button(&mut self, ui: &mut egui::Ui) {
        if self.settings.herdr_cli_path.is_empty()
            || self.settings.cli_provider == CliProvider::Herdr
        {
            return;
        }
        let has_prompt = !self.global_prompt_input.trim().is_empty();
        if ui
            .add_enabled(
                has_prompt,
                egui::Button::new(icon("H", self.settings.font_size)),
            )
            .on_hover_text(
                "Run this prompt as interactive Claude Code in a herdr pane (experimental)",
            )
            .clicked()
        {
            self.submit_prompt(Some(CliProvider::Herdr));
        }
    }

    /// Render the send button and handle submit logic.
    fn render_prompt_send_button(&mut self, ui: &mut egui::Ui, input_response: &egui::Response) {
        let btn_clicked = Self::render_send_button(
            ui,
            self.settings.font_size,
            input_response,
            &self.semantic,
            "Create cue  (\u{2318}Enter to run)",
        );
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
        if (btn_clicked || enter_submitted || cmd_enter) && !self.global_prompt_input.is_empty() {
            let run_with = cmd_enter.then(|| self.settings.cli_provider.clone());
            self.submit_prompt(run_with);
        }
    }

    /// Submit the current prompt text as a new cue, and run it right away with
    /// `run_with` when given.
    fn submit_prompt(&mut self, run_with: Option<CliProvider>) {
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
                if let Some(provider) = run_with {
                    match self.db.update_cue_status(id, CueStatus::Ready) {
                        Ok(()) => {
                            self.claude.expand_running = true;
                            self.reload_cues();
                            self.trigger_run_with_provider(id, provider);
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
    fn render_prompt_hints_and_suggestions(&mut self, ui: &mut egui::Ui) {
        if !self.settings.prompt_suggestions_enabled {
            return;
        }
        // Only re-analyze when the input text has actually changed.
        if self.global_prompt_input != self.cached_prompt_input {
            self.cached_prompt_input = self.global_prompt_input.clone();
            self.cached_prompt_hints = prompt_hints::analyze(&self.global_prompt_input);
            self.cached_prompt_suggestions =
                prompt_suggestions::analyse_prompt(&self.global_prompt_input, false);
        }
        if !self.cached_prompt_hints.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for hint in &self.cached_prompt_hints {
                    ui.label(
                        egui::RichText::new(format!("\u{26A0} {}", hint.label))
                            .small()
                            .color(self.semantic.warning),
                    )
                    .on_hover_text(hint.detail);
                }
            });
        }

        if !self.cached_prompt_suggestions.is_empty() {
            ui.add_space(SPACE_XS);
            for suggestion in &self.cached_prompt_suggestions {
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
