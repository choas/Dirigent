mod agents;
mod commands;
mod general;
mod playbook;
mod sources;

use eframe::egui;

use crate::app::{icon, DirigentApp, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::opencode;
use crate::settings;

impl DirigentApp {
    pub(in crate::app) fn render_settings_panel(&mut self, ctx: &egui::Context) {
        let mut save = false;
        let mut close = false;
        let mut fetch_idx: Option<usize> = None;
        let mut refresh_models = false;
        let fs = self.settings.font_size;

        // Load OpenCode models if not already loaded
        if self.opencode_models.is_empty() {
            let cli_path = self.settings.opencode_cli_path.clone();
            self.opencode_models = opencode::get_available_models(&cli_path);
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Settings");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(icon("\u{2715}", fs))
                        .on_hover_text("Close settings")
                        .clicked()
                    {
                        close = true;
                    }
                    if ui.button("Save").clicked() {
                        save = true;
                    }
                });
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.add_space(SPACE_SM);

                    self.render_settings_general_grid(ui, &mut refresh_models);

                    self.render_settings_smart_ai_section(ui);

                    self.render_settings_sources_section(ui, fs, &mut fetch_idx);

                    self.render_settings_agents_section(ui, fs, &mut close);

                    self.render_settings_commands_section(ui, fs);

                    self.render_settings_playbook_section(ui, fs);

                    ui.add_space(SPACE_MD);
                    if ui.button("Save").clicked() {
                        save = true;
                    }
                }); // end ScrollArea
        });

        self.handle_settings_actions(save, close, refresh_models, fetch_idx);
    }

    fn render_settings_smart_ai_section(&mut self, ui: &mut egui::Ui) {
        ui.add_space(SPACE_MD);
        ui.separator();
        ui.add_space(SPACE_SM);
        ui.strong("Smart AI Interaction");
        ui.add_space(SPACE_XS);

        egui::Grid::new("smart_ai_grid")
            .num_columns(2)
            .spacing([SPACE_MD, SPACE_SM])
            .show(ui, |ui| {
                ui.label("Prompt Suggestions:");
                ui.checkbox(&mut self.settings.prompt_suggestions_enabled, "Show refinement hints below prompt field")
                    .on_hover_text("Heuristic checks for short, vague, or missing-verb prompts.");
                ui.end_row();

                ui.label("Auto-Context File:");
                ui.checkbox(&mut self.settings.auto_context_file, "Include file content (\u{00B1}50 lines) around cue location")
                    .on_hover_text("Sends the surrounding source code directly to the AI, reducing tool calls and latency.");
                ui.end_row();

                ui.label("Auto-Context Git Diff:");
                ui.checkbox(&mut self.settings.auto_context_git_diff, "Include git diff in prompt")
                    .on_hover_text("Appends the current unstaged diff so the AI sees recent changes without an extra tool call.");
                ui.end_row();
            });
    }

    fn handle_settings_actions(
        &mut self,
        save: bool,
        close: bool,
        refresh_models: bool,
        fetch_idx: Option<usize>,
    ) {
        if close {
            self.show_settings = false;
        }
        if save {
            settings::save_settings(&self.project_root, &self.settings);
            settings::sync_home_guard_hook(
                &self.project_root,
                self.settings.allow_home_folder_access,
            );
            self.needs_theme_apply = true;
        }
        if refresh_models {
            let cli_path = self.settings.opencode_cli_path.clone();
            self.opencode_models = opencode::get_available_models(&cli_path);
        }
        if let Some(idx) = fetch_idx {
            self.trigger_source_fetch(idx);
        }
    }
}
