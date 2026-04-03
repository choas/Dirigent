use eframe::egui;

use crate::app::{icon, DirigentApp, SPACE_SM, SPACE_XS};
use crate::settings;

impl DirigentApp {
    pub(in crate::app) fn render_settings_commands_section(&mut self, ui: &mut egui::Ui, fs: f32) {
        let summary = format!("({} commands)", self.settings.commands.len());
        self.commands_expanded = super::collapsible_section_header(
            ui,
            self.commands_expanded,
            "Commands",
            &summary,
            fs,
            self.semantic.secondary_text,
            |ui| self.render_commands_header_buttons(ui),
        );

        if self.commands_expanded {
            self.render_settings_commands_list(ui, fs);
        }
    }

    fn render_commands_header_buttons(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("+ Add Command").clicked() {
                self.settings.commands.push(settings::CueCommand {
                    name: "new".to_string(),
                    prompt: "{task}".to_string(),
                    pre_agent: String::new(),
                    post_agent: String::new(),
                    cli_args: String::new(),
                });
            }
            if ui.small_button("Reset Defaults").clicked() {
                self.settings.commands = settings::default_commands();
            }
        });
    }

    fn render_settings_commands_list(&mut self, ui: &mut egui::Ui, fs: f32) {
        ui.add_space(SPACE_XS);
        ui.label(
            egui::RichText::new("Prefix a cue with [name] to activate a command. Use {task} in the prompt template for the cue text.")
                .small()
                .color(self.semantic.tertiary_text),
        );
        ui.add_space(SPACE_SM);

        if self.settings.commands.is_empty() {
            ui.label(
                egui::RichText::new("No commands configured. Add a command or reset to defaults.")
                    .italics()
                    .color(self.semantic.tertiary_text),
            );
        }

        let mut remove_cmd_idx = None;
        let num_commands = self.settings.commands.len();

        for i in 0..num_commands {
            self.render_settings_command_card(ui, i, fs, &mut remove_cmd_idx);
            ui.add_space(SPACE_SM);
        }

        if let Some(idx) = remove_cmd_idx {
            self.settings.commands.remove(idx);
        }
    }

    fn render_settings_command_card(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        fs: f32,
        remove_cmd_idx: &mut Option<usize>,
    ) {
        self.semantic.card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("[");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.commands[i].name)
                        .desired_width(80.0)
                        .font(egui::TextStyle::Monospace),
                );
                ui.label("]");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(icon("\u{2715}", fs))
                        .on_hover_text("Delete command")
                        .clicked()
                    {
                        *remove_cmd_idx = Some(i);
                    }
                });
            });
            ui.label(
                egui::RichText::new("Prompt template:")
                    .small()
                    .color(self.semantic.secondary_text),
            );
            ui.add(
                egui::TextEdit::multiline(&mut self.settings.commands[i].prompt)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3)
                    .hint_text("Use {task} for the cue text")
                    .font(egui::TextStyle::Monospace),
            );
            egui::Grid::new(format!("cmd_grid_{}", i))
                .num_columns(2)
                .spacing([SPACE_SM, SPACE_XS])
                .show(ui, |ui| {
                    ui.label("Pre-agent:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.commands[i].pre_agent)
                            .desired_width(250.0)
                            .hint_text("shell command (empty = use provider default)")
                            .font(egui::TextStyle::Monospace),
                    );
                    ui.end_row();

                    ui.label("Post-agent:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.commands[i].post_agent)
                            .desired_width(250.0)
                            .hint_text("shell command (empty = use provider default)")
                            .font(egui::TextStyle::Monospace),
                    );
                    ui.end_row();

                    ui.label("CLI args:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.commands[i].cli_args)
                            .desired_width(250.0)
                            .hint_text("extra CLI flags (e.g. --permission-mode plan)")
                            .font(egui::TextStyle::Monospace),
                    );
                    ui.end_row();
                });
        });
    }
}
