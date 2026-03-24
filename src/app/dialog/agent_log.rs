use eframe::egui;

use crate::agents::{AgentKind, AgentStatus};
use crate::db::AgentRunEntry;

use super::super::{icon, DirigentApp, SPACE_SM, SPACE_XS};

impl DirigentApp {
    /// Agent run log rendered in the central panel (replaces code viewer).
    pub(in crate::app) fn render_agent_log_central(&mut self, ctx: &egui::Context) {
        let kind = self.agent_state.show_output.unwrap();
        let fs = self.settings.font_size;

        let kind_key = kind.db_key();
        let runs = self
            .db
            .get_recent_agent_runs_by_kind(&kind_key, 50)
            .unwrap_or_default();

        let mut close = false;
        let mut analyze_run_idx: Option<usize> = None;

        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_agent_log_header(ui, kind, fs, &mut close);
            ui.separator();

            if runs.is_empty() {
                ui.label(
                    egui::RichText::new("No runs recorded yet.")
                        .italics()
                        .color(self.semantic.tertiary_text),
                );
                return;
            }

            self.render_agent_log_run_list(ui, &runs, &mut analyze_run_idx);
        });

        if let Some(idx) = analyze_run_idx {
            self.handle_analyze_run(kind, &runs, idx);
        }

        if close {
            self.handle_agent_log_close();
        }
    }

    /// Render the header bar with back button, agent label, status, and command.
    fn render_agent_log_header(
        &self,
        ui: &mut egui::Ui,
        kind: AgentKind,
        fs: f32,
        close: &mut bool,
    ) {
        ui.horizontal(|ui| {
            if ui.button(icon("\u{2190} Back", fs)).clicked() {
                *close = true;
            }
            ui.separator();
            let agent_label = self
                .settings
                .agents
                .iter()
                .find(|a| a.kind == kind)
                .map(|a| a.display_name().to_string())
                .unwrap_or_else(|| kind.label().to_string());
            ui.strong(format!("{} Runs", agent_label));
            ui.separator();
            self.render_agent_status_label(ui, kind, fs);
            self.render_agent_command_label(ui, kind);
        });
    }

    /// Show current agent status icon and label.
    fn render_agent_status_label(&self, ui: &mut egui::Ui, kind: AgentKind, fs: f32) {
        if let Some(status) = self.agent_state.statuses.get(&kind) {
            let (icon_str, color) = match status {
                AgentStatus::Running => ("\u{21BB} Running", self.semantic.accent),
                AgentStatus::Passed => ("\u{2713} Passed", self.semantic.success),
                AgentStatus::Failed => ("\u{2717} Failed", self.semantic.danger),
                AgentStatus::Error => ("! Error", self.semantic.danger),
                AgentStatus::Idle => ("Idle", self.semantic.secondary_text),
            };
            ui.label(icon(icon_str, fs).color(color));
        }
    }

    /// Show the agent command in monospace.
    fn render_agent_command_label(&self, ui: &mut egui::Ui, kind: AgentKind) {
        if let Some(config) = self.settings.agents.iter().find(|a| a.kind == kind) {
            ui.separator();
            ui.label(
                egui::RichText::new(&config.command)
                    .monospace()
                    .small()
                    .color(self.semantic.muted_text()),
            );
        }
    }

    /// Render the scrollable list of agent runs.
    fn render_agent_log_run_list(
        &self,
        ui: &mut egui::Ui,
        runs: &[AgentRunEntry],
        analyze_run_idx: &mut Option<usize>,
    ) {
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for (idx, run) in runs.iter().enumerate() {
                    self.render_single_run_entry(ui, run, idx, analyze_run_idx);
                    if idx < runs.len() - 1 {
                        ui.separator();
                    }
                }
            });
    }

    /// Render a single run entry: header line + output block.
    fn render_single_run_entry(
        &self,
        ui: &mut egui::Ui,
        run: &AgentRunEntry,
        idx: usize,
        analyze_run_idx: &mut Option<usize>,
    ) {
        let dur = format_duration_ms(run.duration_ms);
        let (status_icon, status_color) = status_icon_and_color(run, &self.semantic);

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(status_icon)
                    .strong()
                    .color(status_color),
            );
            ui.label(
                egui::RichText::new(&run.started_at)
                    .small()
                    .color(self.semantic.muted_text()),
            );
            ui.label(
                egui::RichText::new(format!("({})", dur))
                    .small()
                    .color(self.semantic.secondary_text),
            );
            if let Some(cue_id) = run.cue_id {
                ui.label(
                    egui::RichText::new(format!("cue #{}", cue_id))
                        .small()
                        .color(self.semantic.tertiary_text),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(
                        egui::RichText::new("\u{1F50D} Analyze")
                            .small()
                            .color(self.semantic.accent),
                    )
                    .clicked()
                {
                    *analyze_run_idx = Some(idx);
                }
            });
        });

        // Output block
        egui::Frame::NONE
            .inner_margin(egui::Margin {
                left: SPACE_SM as i8,
                top: SPACE_XS as i8,
                right: SPACE_XS as i8,
                bottom: SPACE_SM as i8,
            })
            .corner_radius(4)
            .fill(self.semantic.selection_bg())
            .show(ui, |ui| {
                if run.output.trim().is_empty() {
                    ui.label(
                        egui::RichText::new("(no output)")
                            .italics()
                            .color(self.semantic.tertiary_text),
                    );
                } else {
                    ui.label(egui::RichText::new(&run.output).monospace().small());
                }
            });
    }

    /// Create a cue from the selected run's output for analysis.
    fn handle_analyze_run(&mut self, kind: AgentKind, runs: &[AgentRunEntry], idx: usize) {
        if let Some(run) = runs.get(idx) {
            let status_label = match run.status.as_str() {
                "passed" => "PASSED",
                "failed" => "FAILED",
                "error" => "ERROR",
                other => other,
            };
            let cue_text = format!(
                "Analyze this {} agent run ({}):\n\n$ {}\n\n{}",
                kind.label(),
                status_label,
                run.command,
                run.output.trim(),
            );
            if let Ok(id) = self.db.insert_cue(&cue_text, "", 0, None, &[]) {
                self.reload_cues();
                self.editing_cue = Some(super::super::EditingCue {
                    id,
                    text: cue_text.clone(),
                    focus_requested: true,
                });
                // Close the agent log so the user lands on the cue pool
                self.agent_state.show_output = None;
            }
        }
    }

    /// Handle closing the agent log and optionally returning to settings.
    fn handle_agent_log_close(&mut self) {
        self.agent_state.show_output = None;
        if self.agent_state.return_to_settings {
            self.agent_state.return_to_settings = false;
            self.reload_settings_from_disk();
            self.show_settings = true;
        }
    }
}

/// Format a duration in milliseconds to a human-readable string.
fn format_duration_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}

/// Map a run status string to its icon and color.
fn status_icon_and_color<'a>(
    run: &AgentRunEntry,
    semantic: &crate::settings::SemanticColors,
) -> (&'a str, egui::Color32) {
    match run.status.as_str() {
        "passed" => ("\u{2713}", semantic.success),
        "failed" => ("\u{2717}", semantic.danger),
        "error" => ("!", semantic.danger),
        _ => ("\u{25CF}", semantic.secondary_text),
    }
}
