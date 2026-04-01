use eframe::egui;
use std::sync::mpsc;

use crate::app::{icon, DirigentApp, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::settings::{NotionPageType, SourceConfig, SourceKind};

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
            SourceKind::Notion => {
                ui.label("Token:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].token)
                        .desired_width(200.0)
                        .hint_text("from env NOTION_TOKEN or .env")
                        .password(true)
                        .font(egui::TextStyle::Monospace),
                );
                ui.end_row();

                // Database/Page selector: dropdown when objects are loaded, else text input.
                ui.label("Database / Page:");
                let is_loading = self
                    .notion_objects_rx
                    .as_ref()
                    .is_some_and(|(idx, _)| *idx == i);
                let has_objects = self.notion_objects.get(&i).is_some_and(|v| !v.is_empty());

                ui.horizontal(|ui| {
                    if has_objects {
                        let objects = &self.notion_objects[&i];
                        let current_id = &self.settings.sources[i].project_key;
                        let selected_label = objects
                            .iter()
                            .find(|o| o.id == *current_id)
                            .map(|o| {
                                let icon = if o.object_type == "database" {
                                    "\u{1F5C3}" // 🗃️
                                } else {
                                    "\u{1F4C4}" // 📄
                                };
                                format!("{} {}", icon, o.title)
                            })
                            .unwrap_or_else(|| {
                                if current_id.is_empty() {
                                    "Select\u{2026}".to_string()
                                } else {
                                    current_id.clone()
                                }
                            });

                        egui::ComboBox::from_id_salt(format!("notion_obj_{}", i))
                            .selected_text(&selected_label)
                            .width(200.0)
                            .show_ui(ui, |ui| {
                                for obj in objects {
                                    let icon = if obj.object_type == "database" {
                                        "\u{1F5C3}" // 🗃️
                                    } else {
                                        "\u{1F4C4}" // 📄
                                    };
                                    let label = format!("{} {}", icon, obj.title);
                                    if ui
                                        .selectable_value(
                                            &mut self.settings.sources[i].project_key,
                                            obj.id.clone(),
                                            &label,
                                        )
                                        .changed()
                                    {
                                        // Auto-set page type hint based on object type.
                                    }
                                }
                            });
                    }

                    if is_loading {
                        ui.spinner();
                    } else if ui.small_button("Load").clicked() {
                        self.start_notion_objects_fetch(i);
                    }
                });
                ui.end_row();

                // Always show a text field for manual ID / URL entry.
                ui.label("");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.sources[i].project_key)
                            .desired_width(200.0)
                            .hint_text("ID or Notion URL")
                            .font(egui::TextStyle::Monospace),
                    );
                    if has_objects {
                        ui.label(
                            egui::RichText::new("(or pick from dropdown above)")
                                .small()
                                .color(self.semantic.tertiary_text),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new("(click Load to list databases & pages)")
                                .small()
                                .color(self.semantic.tertiary_text),
                        );
                    }
                });
                ui.end_row();

                // Show error if the last fetch failed.
                if let Some(err) = self.notion_objects_error.get(&i) {
                    ui.label("");
                    ui.label(
                        egui::RichText::new(err)
                            .small()
                            .color(egui::Color32::from_rgb(220, 50, 50)),
                    );
                    ui.end_row();
                }

                ui.label("Page Type:");
                egui::ComboBox::from_id_salt(format!("notion_page_type_{}", i))
                    .selected_text(self.settings.sources[i].notion_page_type.display_name())
                    .show_ui(ui, |ui| {
                        for pt in NotionPageType::all() {
                            ui.selectable_value(
                                &mut self.settings.sources[i].notion_page_type,
                                pt.clone(),
                                pt.display_name(),
                            );
                        }
                    });
                ui.end_row();

                if self.settings.sources[i].notion_page_type == NotionPageType::KanbanBoard {
                    ui.label("Inbox Status:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.sources[i].filter)
                            .desired_width(120.0)
                            .hint_text("e.g. Not started")
                            .font(egui::TextStyle::Monospace),
                    );
                    ui.end_row();

                    ui.label("Status Property:");
                    ui.add(
                        egui::TextEdit::singleline(
                            &mut self.settings.sources[i].notion_status_property,
                        )
                        .desired_width(120.0)
                        .hint_text("default: Status")
                        .font(egui::TextStyle::Monospace),
                    );
                    ui.end_row();
                }

                let done_hint = match self.settings.sources[i].notion_page_type {
                    NotionPageType::TodoList => "checkbox property name, e.g. Done",
                    NotionPageType::KanbanBoard => "target status, e.g. Done",
                };
                ui.label("Done Value:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.sources[i].notion_done_value)
                        .desired_width(120.0)
                        .hint_text(done_hint)
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

    /// Kick off a background fetch of Notion databases/pages for source `i`.
    fn start_notion_objects_fetch(&mut self, i: usize) {
        let token =
            crate::sources::resolve_source_token(&self.settings.sources[i], &self.project_root);
        self.notion_objects_error.remove(&i);

        let (tx, rx) = mpsc::channel();
        self.notion_objects_rx = Some((i, rx));

        std::thread::spawn(move || {
            let result = crate::sources::fetch_notion_objects(&token);
            let _ = tx.send(result.map_err(|e| e.to_string()));
        });
    }

    /// Poll for the result of a Notion objects fetch (called from the update loop).
    pub(in crate::app) fn process_notion_objects_result(&mut self) {
        let (idx, rx) = match self.notion_objects_rx {
            Some((idx, ref rx)) => (idx, rx),
            None => return,
        };
        let result = match rx.try_recv() {
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.notion_objects_rx = None;
                self.notion_objects_error
                    .insert(idx, "Notion fetch failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        let idx_copy = idx;
        self.notion_objects_rx = None;
        match result {
            Ok(objects) => {
                self.notion_objects.insert(idx_copy, objects);
            }
            Err(e) => {
                self.notion_objects_error.insert(idx_copy, e);
            }
        }
    }
}
