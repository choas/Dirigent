use eframe::egui;

use crate::app::{icon, DirigentApp, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::settings::{SourceConfig, SourceKind};

impl DirigentApp {
    pub(in crate::app) fn render_settings_sources_section(
        &mut self,
        ui: &mut egui::Ui,
        fs: f32,
        fetch_idx: &mut Option<usize>,
    ) {
        ui.add_space(SPACE_MD);
        ui.separator();
        ui.add_space(SPACE_SM);
        ui.horizontal(|ui| {
            let arrow = if self.sources_expanded {
                "\u{25BC}"
            } else {
                "\u{25B6}"
            };
            if ui.button(icon(&format!("{} Sources", arrow), fs)).clicked() {
                self.sources_expanded = !self.sources_expanded;
            }
            ui.label(
                egui::RichText::new(format!(
                    "{}/{}",
                    self.settings.sources.iter().filter(|s| s.enabled).count(),
                    self.settings.sources.len()
                ))
                .small()
                .color(self.semantic.secondary_text),
            );
            if self.sources_expanded {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("+ Add Source").clicked() {
                        self.settings.sources.push(SourceConfig::default());
                    }
                });
            }
        });

        if self.sources_expanded {
            self.render_settings_sources_list(ui, fs, fetch_idx);
        }
    }

    fn render_settings_sources_list(
        &mut self,
        ui: &mut egui::Ui,
        fs: f32,
        fetch_idx: &mut Option<usize>,
    ) {
        ui.add_space(SPACE_SM);

        if self.settings.sources.is_empty() {
            ui.label(
                egui::RichText::new("No sources configured. Add a source to pull cues from GitHub Issues, Trello, Asana, SonarQube, Notion, MCP, or custom commands.")
                    .italics()
                    .color(self.semantic.tertiary_text),
            );
        }

        let mut remove_idx = None;
        let num_sources = self.settings.sources.len();

        for i in 0..num_sources {
            self.render_settings_source_card(ui, i, fs, &mut remove_idx, fetch_idx);
            ui.add_space(SPACE_SM);
        }

        if let Some(idx) = remove_idx {
            self.settings.sources.remove(idx);
        }
    }

    fn render_settings_source_card(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        fs: f32,
        remove_idx: &mut Option<usize>,
        fetch_idx: &mut Option<usize>,
    ) {
        self.semantic.card_frame().show(ui, |ui| {
            // Header: name + enabled + delete
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].name)
                        .desired_width(150.0)
                        .font(egui::TextStyle::Body),
                );
                ui.checkbox(&mut self.settings.sources[i].enabled, "Enabled");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(icon("\u{2715}", fs))
                        .on_hover_text("Delete source")
                        .clicked()
                    {
                        *remove_idx = Some(i);
                    }
                });
            });

            self.render_settings_source_fields(ui, i);

            ui.horizontal(|ui| {
                if ui.small_button("Fetch Now").clicked() {
                    *fetch_idx = Some(i);
                }
            });
        });
    }

    fn render_settings_source_fields(&mut self, ui: &mut egui::Ui, i: usize) {
        egui::Grid::new(format!("source_grid_{}", i))
            .num_columns(2)
            .spacing([SPACE_SM, SPACE_XS])
            .show(ui, |ui| {
                ui.label("Kind:");
                let prev_kind = self.settings.sources[i].kind.clone();
                egui::ComboBox::from_id_salt(format!("source_kind_{}", i))
                    .selected_text(self.settings.sources[i].kind.display_name())
                    .show_ui(ui, |ui| {
                        for kind in SourceKind::all() {
                            ui.selectable_value(
                                &mut self.settings.sources[i].kind,
                                kind.clone(),
                                kind.display_name(),
                            );
                        }
                    });
                // Auto-fill sensible defaults when the kind changes.
                if self.settings.sources[i].kind != prev_kind {
                    self.settings.sources[i].label =
                        self.settings.sources[i].kind.default_label().to_string();
                }
                ui.end_row();

                ui.label("Label:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].label)
                        .desired_width(120.0)
                        .hint_text("filter tag")
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();

                self.render_settings_source_kind_fields(ui, i);

                ui.label("Poll interval:");
                ui.horizontal(|ui| {
                    let mut secs = self.settings.sources[i].poll_interval_secs as f64;
                    ui.add(
                        egui::DragValue::new(&mut secs)
                            .range(0.0..=86400.0)
                            .speed(10.0)
                            .suffix("s"),
                    );
                    self.settings.sources[i].poll_interval_secs = secs as u64;
                    ui.label(
                        egui::RichText::new("(0 = manual only)")
                            .small()
                            .color(self.semantic.tertiary_text),
                    );
                });
                ui.end_row();
            });
    }

    fn render_settings_source_kind_fields(&mut self, ui: &mut egui::Ui, i: usize) {
        match self.settings.sources[i].kind {
            SourceKind::GitHubIssues => {
                ui.label("GH Label:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].filter)
                        .desired_width(120.0)
                        .hint_text("e.g. enhancement")
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();
            }
            SourceKind::Slack => {
                ui.label("Bot Token:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].token)
                        .desired_width(200.0)
                        .hint_text("from env SLACK_BOT_TOKEN or .env")
                        .password(true)
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();

                ui.label("Channel:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].channel)
                        .desired_width(200.0)
                        .hint_text("C01ABCDEF or #channel-name")
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();
            }
            SourceKind::SonarQube => {
                ui.label("Host URL:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].host_url)
                        .desired_width(200.0)
                        .hint_text("http://localhost:9000")
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();

                ui.label("Project Key:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].project_key)
                        .desired_width(200.0)
                        .hint_text("e.g. my-project")
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();

                ui.label("Token:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].token)
                        .desired_width(200.0)
                        .hint_text("from env SONAR_TOKEN or .env")
                        .password(true)
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();
            }
            SourceKind::Trello => {
                ui.label("API Key:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].api_key)
                        .desired_width(200.0)
                        .hint_text("from env TRELLO_API_KEY or .env")
                        .password(true)
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();

                ui.label("Token:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].token)
                        .desired_width(200.0)
                        .hint_text("from env TRELLO_TOKEN or .env")
                        .password(true)
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();

                ui.label("Board ID:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].project_key)
                        .desired_width(200.0)
                        .hint_text("e.g. 60d5ecXXXXX")
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();

                ui.label("List Filter:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].filter)
                        .desired_width(120.0)
                        .hint_text("e.g. To Do (optional)")
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();
            }
            SourceKind::Asana => {
                ui.label("Token:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].token)
                        .desired_width(200.0)
                        .hint_text("from env ASANA_TOKEN or .env")
                        .password(true)
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();

                ui.label("Project GID:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].project_key)
                        .desired_width(200.0)
                        .hint_text("e.g. 120345678901234")
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();
            }
            _ => {
                ui.label("Command:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].command)
                        .desired_width(200.0)
                        .hint_text("shell command outputting JSON")
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();
            }
        }
    }
}
