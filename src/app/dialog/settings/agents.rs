use eframe::egui;

use crate::agents::{
    agents_for_language, default_agents, next_custom_id, AgentConfig, AgentKind, AgentLanguage,
    AgentTrigger,
};
use crate::app::{DirigentApp, SPACE_SM, SPACE_XS};

impl DirigentApp {
    pub(in crate::app) fn render_settings_agents_section(
        &mut self,
        ui: &mut egui::Ui,
        fs: f32,
        close: &mut bool,
    ) {
        let summary = format!(
            "{}/{}",
            self.settings.agents.iter().filter(|a| a.enabled).count(),
            self.settings.agents.len()
        );
        self.agents_expanded = super::collapsible_section_header(
            ui,
            self.agents_expanded,
            "Agents",
            &summary,
            fs,
            self.semantic.secondary_text,
            |ui| self.render_settings_agents_header_actions(ui),
        );

        if self.agents_expanded {
            self.render_settings_agents_list(ui, fs, close);
        }
    }

    fn render_settings_agents_header_actions(&mut self, ui: &mut egui::Ui) {
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

    fn render_settings_agents_list(&mut self, ui: &mut egui::Ui, fs: f32, close: &mut bool) {
        ui.add_space(SPACE_SM);

        self.render_settings_agent_shell_init(ui);

        let card_width = ui.available_width();
        let mut delete_idx: Option<usize> = None;
        let mut view_log_kind: Option<crate::agents::AgentKind> = None;
        let num_agents = self.settings.agents.len();
        for i in 0..num_agents {
            self.render_settings_agent_card(
                ui,
                i,
                fs,
                card_width,
                &mut delete_idx,
                &mut view_log_kind,
            );
            ui.add_space(SPACE_SM);
        }
        if let Some(idx) = delete_idx {
            self.settings.agents.remove(idx);
        }
        if let Some(kind) = view_log_kind {
            self.agent_state.show_output = Some(kind);
            self.agent_state.return_to_settings = true;
            *close = true;
        }

        self.render_settings_agent_language_init(ui);
    }

    fn render_settings_agent_shell_init(&mut self, ui: &mut egui::Ui) {
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
    }

    fn render_settings_agent_card(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        _fs: f32,
        card_width: f32,
        delete_idx: &mut Option<usize>,
        view_log_kind: &mut Option<crate::agents::AgentKind>,
    ) {
        self.semantic.card_frame().show(ui, |ui| {
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
                    if ui
                        .small_button("\u{2715}")
                        .on_hover_text("Delete agent")
                        .clicked()
                    {
                        *delete_idx = Some(i);
                    }
                });
            });

            self.render_settings_agent_fields(ui, i);

            self.render_settings_agent_actions(ui, i, view_log_kind);
        });
    }

    fn render_settings_agent_fields(&mut self, ui: &mut egui::Ui, i: usize) {
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

                self.render_settings_agent_trigger(ui, i);

                self.render_settings_agent_timeout(ui, i);
            });
    }

    fn render_settings_agent_trigger(&mut self, ui: &mut egui::Ui, i: usize) {
        ui.label("Trigger:");
        ui.horizontal(|ui| {
            let current_idx = self.settings.agents[i].trigger.variant_index();
            let mut selected_idx = current_idx;
            egui::ComboBox::from_id_salt(format!("agent_trigger_{}", i))
                .selected_text(self.settings.agents[i].trigger.display_name())
                .show_ui(ui, |ui| {
                    for base in AgentTrigger::base_variants() {
                        if ui
                            .selectable_label(
                                base.variant_index() == current_idx,
                                base.display_name(),
                            )
                            .clicked()
                        {
                            selected_idx = base.variant_index();
                        }
                    }
                });
            if selected_idx != current_idx {
                self.settings.agents[i].trigger =
                    Self::trigger_from_index(&self.settings.agents, i, selected_idx);
            }
            self.render_settings_agent_trigger_kind_selector(ui, i);
        });
        ui.end_row();
    }

    fn trigger_from_index(agents: &[AgentConfig], i: usize, idx: usize) -> AgentTrigger {
        match idx {
            0 => AgentTrigger::AfterRun,
            1 => AgentTrigger::AfterCommit,
            2 => {
                let own_kind = agents[i].kind;
                agents
                    .iter()
                    .find(|a| a.kind != own_kind)
                    .map(|a| AgentTrigger::AfterAgent(a.kind))
                    .unwrap_or(AgentTrigger::AfterRun)
            }
            3 => AgentTrigger::OnFileChange,
            _ => AgentTrigger::Manual,
        }
    }

    fn render_settings_agent_trigger_kind_selector(&mut self, ui: &mut egui::Ui, i: usize) {
        if let AgentTrigger::AfterAgent(current_kind) = self.settings.agents[i].trigger {
            let own_kind = self.settings.agents[i].kind;
            // Guard against self-retrigger: if current_kind matches the agent's
            // own kind, reassign to a different agent or fall back to AfterRun.
            if current_kind == own_kind {
                let alt = self
                    .settings
                    .agents
                    .iter()
                    .find(|a| a.kind != own_kind)
                    .map(|a| a.kind);
                self.settings.agents[i].trigger = match alt {
                    Some(k) => AgentTrigger::AfterAgent(k),
                    None => AgentTrigger::AfterRun,
                };
                return;
            }
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
    }

    fn render_settings_agent_timeout(&mut self, ui: &mut egui::Ui, i: usize) {
        ui.label("Timeout:");
        ui.horizontal(|ui| {
            let mut secs = self.settings.agents[i].timeout_secs as f64;
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
                && ui.small_button("\u{2715} Cancel").clicked()
            {
                self.cancel_agent(agent_kind);
            }
        });
        ui.end_row();
    }

    fn render_settings_agent_actions(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        view_log_kind: &mut Option<crate::agents::AgentKind>,
    ) {
        ui.horizontal(|ui| {
            let agent_kind = self.settings.agents[i].kind;
            let is_running = self.agent_state.statuses.get(&agent_kind)
                == Some(&crate::agents::AgentStatus::Running);
            if is_running {
                if ui.small_button("\u{2715} Cancel").clicked() {
                    self.cancel_agent(agent_kind);
                }
            } else if ui.small_button("Run Now").clicked() {
                self.trigger_agent_manual(self.settings.agents[i].kind);
            }
            if ui.small_button("View Logs").clicked() {
                *view_log_kind = Some(self.settings.agents[i].kind);
            }
            self.render_settings_agent_status_label(ui, agent_kind);
        });
    }

    fn render_settings_agent_status_label(
        &mut self,
        ui: &mut egui::Ui,
        agent_kind: crate::agents::AgentKind,
    ) {
        if let Some(status) = self.agent_state.statuses.get(&agent_kind) {
            let (icon_str, color) = match status {
                crate::agents::AgentStatus::Running => ("\u{21BB} running", self.semantic.accent),
                crate::agents::AgentStatus::Passed => ("\u{2713} passed", self.semantic.success),
                crate::agents::AgentStatus::Failed => ("\u{2717} failed", self.semantic.danger),
                crate::agents::AgentStatus::Error => ("! error", self.semantic.danger),
                _ => ("", self.semantic.tertiary_text),
            };
            if !icon_str.is_empty() {
                ui.label(egui::RichText::new(icon_str).small().color(color));
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
    }

    fn render_settings_agent_language_init(&mut self, ui: &mut egui::Ui) {
        // Initialize from language preset
        ui.add_space(SPACE_SM);
        ui.horizontal(|ui| {
            ui.label("Language:");
            egui::ComboBox::from_id_salt("agent_init_language")
                .selected_text(self.agents_init_language.label())
                .show_ui(ui, |ui| {
                    for lang in AgentLanguage::all() {
                        ui.selectable_value(&mut self.agents_init_language, *lang, lang.label());
                    }
                });
            if ui.button("Initialize").clicked() {
                self.settings.agents = agents_for_language(self.agents_init_language);
            }
        });
    }
}
