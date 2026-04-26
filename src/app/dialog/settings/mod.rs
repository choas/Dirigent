mod agents;
mod commands;
mod general;
mod lsp;
mod playbook;
mod sources;

use std::sync::mpsc;

use eframe::egui;

use crate::app::{icon, DirigentApp, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::opencode;
use crate::settings;

/// Render a collapsible section header with an arrow toggle, label, and summary text.
/// Returns the (possibly toggled) expanded state.
fn collapsible_section_header(
    ui: &mut egui::Ui,
    expanded: bool,
    label: &str,
    summary: &str,
    fs: f32,
    secondary_text_color: egui::Color32,
    add_actions: impl FnOnce(&mut egui::Ui),
) -> bool {
    ui.add_space(SPACE_MD);
    ui.separator();
    ui.add_space(SPACE_SM);
    let mut result = expanded;
    ui.horizontal(|ui| {
        let arrow = ["\u{25B6}", "\u{25BC}"][result as usize];
        if ui
            .button(icon(&format!("{} {}", arrow, label), fs))
            .clicked()
        {
            result = !result;
        }
        ui.label(
            egui::RichText::new(summary)
                .small()
                .color(secondary_text_color),
        );
        if result {
            add_actions(ui);
        }
    });
    result
}

impl DirigentApp {
    pub(in crate::app) fn render_settings_panel(&mut self, ui: &mut egui::Ui) {
        let mut save = false;
        let mut close = false;
        let mut fetch_idx: Option<usize> = None;
        let mut refresh_models = false;
        let fs = self.settings.font_size;

        // Drain any completed background model fetch
        if let Ok(models) = self.opencode_models_rx.try_recv() {
            self.opencode_models = models;
            self.opencode_models_loading = false;
        }

        // Kick off background fetch if not yet loaded and not already loading
        if self.opencode_models.is_empty() && !self.opencode_models_loading {
            self.spawn_opencode_models_fetch();
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
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

                    self.render_settings_lsp_section(ui, fs);

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

    fn spawn_opencode_models_fetch(&mut self) {
        self.opencode_models_loading = true;
        let (tx, rx) = mpsc::channel();
        self.opencode_models_rx = rx;
        let cli_path = self.settings.opencode_cli_path.clone();
        let ctx = self.egui_ctx.clone();
        std::thread::spawn(move || {
            let models = opencode::get_available_models(&cli_path);
            let _ = tx.send(models);
            if let Some(c) = ctx.get() {
                c.request_repaint();
            }
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
            if let Err(e) = settings::save_settings(&self.project_root, &self.settings) {
                self.set_status_message(format!("Failed to save settings: {e}"));
            }
            if let Err(e) = settings::sync_home_guard_hook(
                &self.project_root,
                self.settings.allow_home_folder_access,
            ) {
                self.set_status_message(format!("Failed to sync guard hook: {e:#}"));
            }
            self.needs_theme_apply = true;
        }
        if refresh_models {
            self.opencode_models.clear();
            self.spawn_opencode_models_fetch();
        }
        if let Some(idx) = fetch_idx {
            self.trigger_source_fetch(idx);
        }
    }
}
