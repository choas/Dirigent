use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::agents::{
    agents_for_language, default_agents, next_custom_id, AgentConfig, AgentKind, AgentLanguage,
    AgentTrigger,
};
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
                                    .hint_text("not found — enter path to claude")
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

                            ui.label("Env Variables:");
                            ui.add(
                                egui::TextEdit::multiline(&mut self.settings.claude_env_vars)
                                    .desired_width(250.0)
                                    .desired_rows(2)
                                    .hint_text("KEY=VALUE per line")
                                    .font(egui::TextStyle::Monospace),
                            );
                            ui.end_row();

                            ui.label("Pre-run Script:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.claude_pre_run_script)
                                    .desired_width(250.0)
                                    .hint_text("shell command before run")
                                    .font(egui::TextStyle::Monospace),
                            );
                            ui.end_row();

                            ui.label("Post-run Script:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.claude_post_run_script)
                                    .desired_width(250.0)
                                    .hint_text("shell command after run")
                                    .font(egui::TextStyle::Monospace),
                            );
                            ui.end_row();
                        }
                        CliProvider::OpenCode => {
                            ui.label("CLI Path:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.opencode_cli_path)
                                    .desired_width(250.0)
                                    .hint_text("not found — enter path to opencode")
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

                            ui.label("Env Variables:");
                            ui.add(
                                egui::TextEdit::multiline(&mut self.settings.opencode_env_vars)
                                    .desired_width(250.0)
                                    .desired_rows(2)
                                    .hint_text("KEY=VALUE per line")
                                    .font(egui::TextStyle::Monospace),
                            );
                            ui.end_row();

                            ui.label("Pre-run Script:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.opencode_pre_run_script)
                                    .desired_width(250.0)
                                    .hint_text("shell command before run")
                                    .font(egui::TextStyle::Monospace),
                            );
                            ui.end_row();

                            ui.label("Post-run Script:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.opencode_post_run_script)
                                    .desired_width(250.0)
                                    .hint_text("shell command after run")
                                    .font(egui::TextStyle::Monospace),
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

                    ui.label("Lava Lamp:");
                    ui.checkbox(&mut self.settings.lava_lamp_enabled, "Show lava lamp while running");
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
                                    SourceKind::Slack => {
                                        ui.label("Bot Token:");
                                        ui.add(
                                            egui::TextEdit::singleline(
                                                &mut self.settings.sources[i].token,
                                            )
                                            .desired_width(200.0)
                                            .hint_text("xoxb-...")
                                            .password(true)
                                            .font(egui::TextStyle::Monospace),
                                        );
                                        ui.end_row();

                                        ui.label("Channel:");
                                        ui.add(
                                            egui::TextEdit::singleline(
                                                &mut self.settings.sources[i].channel,
                                            )
                                            .desired_width(200.0)
                                            .hint_text("C01ABCDEF or #channel-name")
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
                        "{}/{}",
                        self.settings.agents.iter().filter(|a| a.enabled).count(),
                        self.settings.agents.len()
                    ))
                    .small()
                    .color(self.semantic.secondary_text),
                );
                if self.agents_expanded {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Reset Defaults").clicked() {
                            self.settings.agents = default_agents();
                        }
                        if ui.small_button("+ Add Agent").clicked() {
                            let id = next_custom_id(&self.settings.agents);
                            self.settings.agents.push(AgentConfig {
                                kind: AgentKind::Custom(id),
                                name: format!("Agent {}", id),
                                enabled: true,
                                command: String::new(),
                                trigger: AgentTrigger::Manual,
                                timeout_secs: 120,
                                working_dir: String::new(),
                                before_run: String::new(),
                            });
                        }
                    });
                }
            });

            if self.agents_expanded {
                ui.add_space(SPACE_SM);

                // Shell init (global, prepended to every agent command)
                ui.label("Shell Init:");
                ui.add(
                    egui::TextEdit::multiline(&mut self.settings.agent_shell_init)
                        .desired_width(f32::INFINITY)
                        .desired_rows(2)
                        .hint_text("e.g. source ~/.zprofile  (sets PATH, JAVA_HOME, …)")
                        .font(egui::TextStyle::Monospace),
                );
                ui.label(
                    egui::RichText::new("Prepended to every agent command. Use this when the macOS app doesn't inherit your shell environment.")
                        .small()
                        .color(self.semantic.tertiary_text),
                );
                ui.add_space(SPACE_SM);

                let card_width = ui.available_width();
                let mut delete_idx: Option<usize> = None;
                let mut view_log_kind: Option<crate::agents::AgentKind> = None;
                let num_agents = self.settings.agents.len();
                for i in 0..num_agents {
                    self.semantic.card_frame()
                        .show(ui, |ui| {
                            ui.set_width(card_width);
                            let kind_label = self.settings.agents[i].kind.label().to_string();
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.settings.agents[i].name)
                                        .desired_width(120.0)
                                        .hint_text(&kind_label)
                                        .font(egui::TextStyle::Body),
                                );
                                ui.label(
                                    egui::RichText::new(format!("({})", kind_label))
                                        .small()
                                        .color(self.semantic.tertiary_text),
                                );
                                ui.checkbox(&mut self.settings.agents[i].enabled, "Enabled");
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("\u{2715}").on_hover_text("Delete agent").clicked() {
                                        delete_idx = Some(i);
                                    }
                                });
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

                                    ui.label("Directory:");
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut self.settings.agents[i].working_dir,
                                        )
                                        .desired_width(200.0)
                                        .hint_text("relative to repo root (empty = root)")
                                        .font(egui::TextStyle::Monospace),
                                    );
                                    ui.end_row();

                                    ui.label("Before Run:");
                                    ui.vertical(|ui| {
                                        ui.add(
                                            egui::TextEdit::singleline(
                                                &mut self.settings.agents[i].before_run,
                                            )
                                            .desired_width(300.0)
                                            .hint_text("e.g. echo $PROMPT > /tmp/last_prompt")
                                            .font(egui::TextStyle::Monospace),
                                        );
                                        ui.label(
                                            egui::RichText::new("Runs before agent. $PROMPT env var has the cue text. Non-zero exit skips the agent.")
                                                .small()
                                                .color(self.semantic.tertiary_text),
                                        );
                                    });
                                    ui.end_row();

                                    ui.label("Trigger:");
                                    ui.horizontal(|ui| {
                                        let current_idx = self.settings.agents[i].trigger.variant_index();
                                        let mut selected_idx = current_idx;
                                        egui::ComboBox::from_id_salt(format!("agent_trigger_{}", i))
                                            .selected_text(
                                                self.settings.agents[i].trigger.display_name(),
                                            )
                                            .show_ui(ui, |ui| {
                                                for base in AgentTrigger::base_variants() {
                                                    if ui.selectable_label(
                                                        base.variant_index() == current_idx,
                                                        base.display_name(),
                                                    ).clicked() {
                                                        selected_idx = base.variant_index();
                                                    }
                                                }
                                            });
                                        if selected_idx != current_idx {
                                            self.settings.agents[i].trigger = match selected_idx {
                                                0 => AgentTrigger::AfterRun,
                                                1 => AgentTrigger::AfterCommit,
                                                2 => AgentTrigger::AfterAgent(AgentKind::Format),
                                                3 => AgentTrigger::OnFileChange,
                                                _ => AgentTrigger::Manual,
                                            };
                                        }
                                        if let AgentTrigger::AfterAgent(current_kind) = self.settings.agents[i].trigger {
                                            let own_kind = self.settings.agents[i].kind;
                                            let mut selected = current_kind;
                                            // Build list of other agents for the selector
                                            let other_agents: Vec<(AgentKind, String)> = self
                                                .settings
                                                .agents
                                                .iter()
                                                .filter(|a| a.kind != own_kind)
                                                .map(|a| (a.kind, a.display_name().to_string()))
                                                .collect();
                                            let selected_label = other_agents
                                                .iter()
                                                .find(|(k, _)| *k == selected)
                                                .map(|(_, n)| n.as_str())
                                                .unwrap_or(selected.label());
                                            egui::ComboBox::from_id_salt(format!("agent_trigger_kind_{}", i))
                                                .selected_text(selected_label)
                                                .show_ui(ui, |ui| {
                                                    for (k, name) in &other_agents {
                                                        ui.selectable_value(&mut selected, *k, name.as_str());
                                                    }
                                                });
                                            if selected != current_kind {
                                                self.settings.agents[i].trigger = AgentTrigger::AfterAgent(selected);
                                            }
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

                                        let agent_kind = self.settings.agents[i].kind;
                                        if self.agent_state.statuses.get(&agent_kind)
                                            == Some(&crate::agents::AgentStatus::Running)
                                        {
                                            if ui.small_button("\u{2715} Cancel").clicked() {
                                                self.cancel_agent(agent_kind);
                                            }
                                        }
                                    });
                                    ui.end_row();
                                });

                            ui.horizontal(|ui| {
                                let agent_kind = self.settings.agents[i].kind;
                                let is_running = self.agent_state.statuses.get(&agent_kind)
                                    == Some(&crate::agents::AgentStatus::Running);
                                if is_running {
                                    if ui.small_button("\u{2715} Cancel").clicked() {
                                        self.cancel_agent(agent_kind);
                                    }
                                } else {
                                    if ui.small_button("Run Now").clicked() {
                                        self.trigger_agent_manual(self.settings.agents[i].kind);
                                    }
                                }
                                if ui.small_button("View Logs").clicked() {
                                    view_log_kind = Some(self.settings.agents[i].kind);
                                }
                                if let Some(status) = self.agent_state.statuses.get(&agent_kind) {
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
                                // Show last run info (duration + time ago)
                                if let Some(info) = self.agent_state.last_run.get(&agent_kind) {
                                    let dur = if info.duration_ms < 1000 {
                                        format!("{}ms", info.duration_ms)
                                    } else {
                                        format!("{:.1}s", info.duration_ms as f64 / 1000.0)
                                    };
                                    let ago_secs = info.finished_at.elapsed().as_secs();
                                    let ago = if ago_secs < 5 {
                                        "just now".to_string()
                                    } else if ago_secs < 60 {
                                        format!("{}s ago", ago_secs)
                                    } else if ago_secs < 3600 {
                                        format!("{}m ago", ago_secs / 60)
                                    } else {
                                        format!("{}h ago", ago_secs / 3600)
                                    };
                                    ui.label(
                                        egui::RichText::new(format!("{} \u{2022} {}", dur, ago))
                                            .small()
                                            .color(self.semantic.tertiary_text),
                                    );
                                }
                            });
                        });
                    ui.add_space(SPACE_SM);
                }
                if let Some(idx) = delete_idx {
                    self.settings.agents.remove(idx);
                }
                if let Some(kind) = view_log_kind {
                    self.agent_state.show_output = Some(kind);
                    self.agent_state.return_to_settings = true;
                    close = true;
                }

                // Initialize from language preset
                ui.add_space(SPACE_SM);
                ui.horizontal(|ui| {
                    ui.label("Language:");
                    egui::ComboBox::from_id_salt("agent_init_language")
                        .selected_text(self.agents_init_language.label())
                        .show_ui(ui, |ui| {
                            for lang in AgentLanguage::all() {
                                ui.selectable_value(
                                    &mut self.agents_init_language,
                                    *lang,
                                    lang.label(),
                                );
                            }
                        });
                    if ui.button("Initialize").clicked() {
                        self.settings.agents = agents_for_language(self.agents_init_language);
                    }
                });
            }

            // Commands section
            ui.add_space(SPACE_MD);
            ui.separator();
            ui.add_space(SPACE_SM);
            ui.horizontal(|ui| {
                let arrow = if self.commands_expanded { "\u{25BC}" } else { "\u{25B6}" };
                if ui.button(icon(&format!("{} Commands", arrow), fs)).clicked() {
                    self.commands_expanded = !self.commands_expanded;
                }
                ui.label(
                    egui::RichText::new(format!("({} commands)", self.settings.commands.len()))
                        .small()
                        .color(self.semantic.secondary_text),
                );
                if self.commands_expanded {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("+ Add Command").clicked() {
                            self.settings.commands.push(settings::CueCommand {
                                name: "new".to_string(),
                                prompt: "{task}".to_string(),
                                pre_agent: String::new(),
                                post_agent: String::new(),
                            });
                        }
                        if ui.small_button("Reset Defaults").clicked() {
                            self.settings.commands = settings::default_commands();
                        }
                    });
                }
            });

            if self.commands_expanded {
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
                    self.semantic.card_frame()
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("[");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.settings.commands[i].name)
                                        .desired_width(80.0)
                                        .font(egui::TextStyle::Monospace),
                                );
                                ui.label("]");
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .small_button(icon("\u{2715}", fs))
                                            .on_hover_text("Delete command")
                                            .clicked()
                                        {
                                            remove_cmd_idx = Some(i);
                                        }
                                    },
                                );
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
                                });
                        });
                    ui.add_space(SPACE_SM);
                }

                if let Some(idx) = remove_cmd_idx {
                    self.settings.commands.remove(idx);
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
