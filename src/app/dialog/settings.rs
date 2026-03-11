use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::agents::{AgentTrigger, default_agents};
use crate::opencode;
use crate::settings::{self, default_playbook, CliProvider, SourceConfig, SourceKind, ThemeChoice};

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
                    if ui.button(icon("\u{2715}", fs)).on_hover_text("Close settings").clicked() {
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

            egui::Grid::new("settings_grid")
                .num_columns(2)
                .spacing([SPACE_MD, SPACE_SM])
                .show(ui, |ui| {
                    ui.label("Theme:");
                    let theme_label = self.settings.theme.display_name();
                    egui::ComboBox::from_id_salt("theme_combo")
                        .selected_text(theme_label)
                        .show_ui(ui, |ui| {
                            let mut prev_was_dark = true;
                            for variant in ThemeChoice::all_variants() {
                                if prev_was_dark && !variant.is_dark() {
                                    ui.separator();
                                    prev_was_dark = false;
                                }
                                ui.selectable_value(
                                    &mut self.settings.theme,
                                    variant.clone(),
                                    variant.display_name(),
                                );
                            }
                        });
                    ui.end_row();

                    ui.label("CLI Provider:");
                    let provider_label = self.settings.cli_provider.display_name();
                    egui::ComboBox::from_id_salt("provider_combo")
                        .selected_text(provider_label)
                        .show_ui(ui, |ui| {
                            for provider in CliProvider::all() {
                                ui.selectable_value(
                                    &mut self.settings.cli_provider,
                                    provider.clone(),
                                    provider.display_name(),
                                );
                            }
                        });
                    ui.end_row();

                    ui.label("Model:");
                    match self.settings.cli_provider {
                        CliProvider::Claude => {
                            egui::ComboBox::from_id_salt("model_combo")
                                .selected_text(&self.settings.claude_model)
                                .show_ui(ui, |ui| {
                                    for model in &[
                                        "claude-opus-4-6",
                                        "claude-sonnet-4-6",
                                    ] {
                                        ui.selectable_value(
                                            &mut self.settings.claude_model,
                                            model.to_string(),
                                            *model,
                                        );
                                    }
                                });
                        }
                        CliProvider::OpenCode => {
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt("model_combo")
                                    .selected_text(&self.settings.opencode_model)
                                    .show_ui(ui, |ui| {
                                        let models = if self.opencode_models.is_empty() {
                                            vec![
                                                "openai/o1".to_string(),
                                                "openai/o1-mini".to_string(),
                                                "openai/o3".to_string(),
                                                "openai/o3-mini".to_string(),
                                                "anthropic/claude-sonnet-4-6".to_string(),
                                                "anthropic/claude-opus-4-6".to_string(),
                                                "google/gemini-2.5-pro".to_string(),
                                                "google/gemini-2.5-flash".to_string(),
                                            ]
                                        } else {
                                            self.opencode_models.clone()
                                        };
                                        for model in &models {
                                            ui.selectable_value(
                                                &mut self.settings.opencode_model,
                                                model.clone(),
                                                model.as_str(),
                                            );
                                        }
                                    });
                                if ui.button(icon("\u{21bb}", fs))
                                    .on_hover_text("Refresh models from OpenCode")
                                    .clicked()
                                {
                                    refresh_models = true;
                                }
                            });
                        }
                    }
                    ui.end_row();

                    match self.settings.cli_provider {
                        CliProvider::Claude => {
                            ui.label("CLI Path:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.claude_cli_path)
                                    .desired_width(250.0)
                                    .hint_text("claude (default: from PATH)")
                                    .font(egui::TextStyle::Monospace),
                            );
                            ui.end_row();

                            ui.label("Extra Arguments:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.claude_extra_args)
                                    .desired_width(250.0)
                                    .hint_text("e.g. --max-turns 10")
                                    .font(egui::TextStyle::Monospace),
                            );
                            ui.end_row();

                            ui.label("Default Flags:");
                            ui.label(
                                egui::RichText::new(
                                    "-p <prompt> --verbose --output-format stream-json --dangerously-skip-permissions"
                                )
                                .monospace()
                                .weak(),
                            );
                            ui.end_row();
                        }
                        CliProvider::OpenCode => {
                            ui.label("CLI Path:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.opencode_cli_path)
                                    .desired_width(250.0)
                                    .hint_text("opencode (default: from PATH)")
                                    .font(egui::TextStyle::Monospace),
                            );
                            ui.end_row();

                            ui.label("Extra Arguments:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.opencode_extra_args)
                                    .desired_width(250.0)
                                    .hint_text("e.g. --mcp-server ...")
                                    .font(egui::TextStyle::Monospace),
                            );
                            ui.end_row();

                            ui.label("Default Flags:");
                            ui.label(
                                egui::RichText::new("run <prompt> --format json")
                                    .monospace()
                                    .weak(),
                            );
                            ui.end_row();
                        }
                    }

                    ui.label("Font:");
                    egui::ComboBox::from_id_salt("font_combo")
                        .selected_text(&self.settings.font_family)
                        .show_ui(ui, |ui| {
                            for font in &[
                                "Menlo",
                                "SF Mono",
                                "Monaco",
                                "Courier New",
                                "JetBrains Mono",
                                "Fira Code",
                                "Source Code Pro",
                                "Cascadia Code",
                            ] {
                                ui.selectable_value(
                                    &mut self.settings.font_family,
                                    font.to_string(),
                                    *font,
                                );
                            }
                        });
                    ui.end_row();

                    ui.label("Font Size:");
                    ui.add(egui::Slider::new(&mut self.settings.font_size, 8.0..=32.0).suffix(" px"));
                    ui.end_row();

                    ui.label("Notifications:");
                    ui.end_row();

                    ui.label("  Sound:");
                    ui.checkbox(&mut self.settings.notify_sound, "Play sound on task review");
                    ui.end_row();

                    ui.label("  Popup:");
                    ui.checkbox(&mut self.settings.notify_popup, "Show macOS notification");
                    ui.end_row();
                });

            // Sources section
            ui.add_space(SPACE_MD);
            ui.separator();
            ui.add_space(SPACE_SM);
            ui.horizontal(|ui| {
                ui.strong("Sources");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("+ Add Source").clicked() {
                        self.settings.sources.push(SourceConfig::default());
                    }
                });
            });
            ui.add_space(SPACE_SM);

            if self.settings.sources.is_empty() {
                ui.label(
                    egui::RichText::new("No sources configured. Add a source to pull cues from GitHub Issues, Notion, MCP, or custom commands.")
                        .italics()
                        .color(self.semantic.tertiary_text),
                );
            }

            let mut remove_idx = None;
            let num_sources = self.settings.sources.len();

            for i in 0..num_sources {
                self.semantic.card_frame()
                    .show(ui, |ui| {
                        // Header: name + enabled + delete
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.sources[i].name)
                                    .desired_width(150.0)
                                    .font(egui::TextStyle::Body),
                            );
                            ui.checkbox(&mut self.settings.sources[i].enabled, "Enabled");
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button(icon("\u{2715}", fs))
                                        .on_hover_text("Delete source")
                                        .clicked()
                                    {
                                        remove_idx = Some(i);
                                    }
                                },
                            );
                        });

                        egui::Grid::new(format!("source_grid_{}", i))
                            .num_columns(2)
                            .spacing([SPACE_SM, SPACE_XS])
                            .show(ui, |ui| {
                                ui.label("Kind:");
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
                                ui.end_row();

                                ui.label("Label:");
                                ui.add(
                                    egui::TextEdit::singleline(
                                        &mut self.settings.sources[i].label,
                                    )
                                    .desired_width(120.0)
                                    .hint_text("filter tag")
                                    .font(egui::TextStyle::Monospace),
                                );
                                ui.end_row();

                                match self.settings.sources[i].kind {
                                    SourceKind::GitHubIssues => {
                                        ui.label("GH Label:");
                                        ui.add(
                                            egui::TextEdit::singleline(
                                                &mut self.settings.sources[i].filter,
                                            )
                                            .desired_width(120.0)
                                            .hint_text("e.g. enhancement")
                                            .font(egui::TextStyle::Monospace),
                                        );
                                        ui.end_row();
                                    }
                                    _ => {
                                        ui.label("Command:");
                                        ui.add(
                                            egui::TextEdit::singleline(
                                                &mut self.settings.sources[i].command,
                                            )
                                            .desired_width(200.0)
                                            .hint_text("shell command outputting JSON")
                                            .font(egui::TextStyle::Monospace),
                                        );
                                        ui.end_row();
                                    }
                                }

                                ui.label("Poll interval:");
                                ui.horizontal(|ui| {
                                    let mut secs =
                                        self.settings.sources[i].poll_interval_secs as f64;
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

                        ui.horizontal(|ui| {
                            if ui.small_button("Fetch Now").clicked() {
                                fetch_idx = Some(i);
                            }
                        });
                    });
                ui.add_space(SPACE_SM);
            }

            if let Some(idx) = remove_idx {
                self.settings.sources.remove(idx);
            }

            // Agents section
            ui.add_space(SPACE_MD);
            ui.separator();
            ui.add_space(SPACE_SM);
            ui.horizontal(|ui| {
                let arrow = if self.agents_expanded { "\u{25BC}" } else { "\u{25B6}" };
                if ui.button(icon(&format!("{} Agents", arrow), fs)).clicked() {
                    self.agents_expanded = !self.agents_expanded;
                }
                ui.label(
                    egui::RichText::new(format!(
                        "({} agents)",
                        self.settings.agents.iter().filter(|a| a.enabled).count()
                    ))
                    .small()
                    .color(self.semantic.secondary_text),
                );
                if self.agents_expanded {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Reset Defaults").clicked() {
                            self.settings.agents = default_agents();
                        }
                    });
                }
            });

            if self.agents_expanded {
                ui.add_space(SPACE_SM);

                let num_agents = self.settings.agents.len();
                for i in 0..num_agents {
                    self.semantic.card_frame()
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(self.settings.agents[i].kind.label())
                                        .strong(),
                                );
                                ui.checkbox(&mut self.settings.agents[i].enabled, "Enabled");
                            });

                            egui::Grid::new(format!("agent_grid_{}", i))
                                .num_columns(2)
                                .spacing([SPACE_SM, SPACE_XS])
                                .show(ui, |ui| {
                                    ui.label("Command:");
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut self.settings.agents[i].command,
                                        )
                                        .desired_width(300.0)
                                        .hint_text("shell command")
                                        .font(egui::TextStyle::Monospace),
                                    );
                                    ui.end_row();

                                    ui.label("Trigger:");
                                    egui::ComboBox::from_id_salt(format!("agent_trigger_{}", i))
                                        .selected_text(
                                            self.settings.agents[i].trigger.display_name(),
                                        )
                                        .show_ui(ui, |ui| {
                                            for trigger in AgentTrigger::all() {
                                                ui.selectable_value(
                                                    &mut self.settings.agents[i].trigger,
                                                    trigger.clone(),
                                                    trigger.display_name(),
                                                );
                                            }
                                        });
                                    ui.end_row();

                                    ui.label("Timeout:");
                                    ui.horizontal(|ui| {
                                        let mut secs =
                                            self.settings.agents[i].timeout_secs as f64;
                                        ui.add(
                                            egui::DragValue::new(&mut secs)
                                                .range(5.0..=600.0)
                                                .speed(5.0)
                                                .suffix("s"),
                                        );
                                        self.settings.agents[i].timeout_secs = secs as u64;
                                    });
                                    ui.end_row();
                                });

                            ui.horizontal(|ui| {
                                if ui.small_button("Run Now").clicked() {
                                    self.trigger_agent_manual(self.settings.agents[i].kind);
                                }
                                if let Some(status) =
                                    self.agent_state.statuses.get(&self.settings.agents[i].kind)
                                {
                                    let (icon_str, color) = match status {
                                        crate::agents::AgentStatus::Running => {
                                            ("\u{21BB} running", self.semantic.accent)
                                        }
                                        crate::agents::AgentStatus::Passed => {
                                            ("\u{2713} passed", self.semantic.success)
                                        }
                                        crate::agents::AgentStatus::Failed => {
                                            ("\u{2717} failed", self.semantic.danger)
                                        }
                                        crate::agents::AgentStatus::Error => {
                                            ("! error", self.semantic.danger)
                                        }
                                        _ => ("", self.semantic.tertiary_text),
                                    };
                                    if !icon_str.is_empty() {
                                        ui.label(
                                            egui::RichText::new(icon_str)
                                                .small()
                                                .color(color),
                                        );
                                    }
                                }
                            });
                        });
                    ui.add_space(SPACE_SM);
                }
            }

            // Playbook section
            ui.add_space(SPACE_MD);
            ui.separator();
            ui.add_space(SPACE_SM);
            ui.horizontal(|ui| {
                let arrow = if self.playbook_expanded { "\u{25BC}" } else { "\u{25B6}" };
                if ui.button(icon(&format!("{} Playbook", arrow), fs)).clicked() {
                    self.playbook_expanded = !self.playbook_expanded;
                }
                ui.label(
                    egui::RichText::new(format!("({} plays)", self.settings.playbook.len()))
                        .small()
                        .color(self.semantic.secondary_text),
                );
                if self.playbook_expanded {
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
            });

            if self.playbook_expanded {
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
                    self.semantic.card_frame()
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.settings.playbook[i].name)
                                        .desired_width(200.0)
                                        .font(egui::TextStyle::Body),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .small_button(icon("\u{2715}", fs))
                                            .on_hover_text("Delete play")
                                            .clicked()
                                        {
                                            remove_play_idx = Some(i);
                                        }
                                    },
                                );
                            });
                            ui.add(
                                egui::TextEdit::multiline(&mut self.settings.playbook[i].prompt)
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(3)
                                    .hint_text("Prompt text...")
                                    .font(egui::TextStyle::Monospace),
                            );
                        });
                    ui.add_space(SPACE_SM);
                }

                if let Some(idx) = remove_play_idx {
                    self.settings.playbook.remove(idx);
                }
            }

            ui.add_space(SPACE_MD);
            if ui.button("Save").clicked() {
                save = true;
            }
                }); // end ScrollArea
        });

        if close {
            self.show_settings = false;
        }
        if save {
            settings::save_settings(&self.project_root, &self.settings);
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
