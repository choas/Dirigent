use eframe::egui;

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

        egui::CentralPanel::default().show(ctx, |ui| {
            // Header bar
            ui.horizontal(|ui| {
                if ui.button(icon("\u{2190} Back", fs)).clicked() {
                    close = true;
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
                // Show current status
                if let Some(status) = self.agent_state.statuses.get(&kind) {
                    let (icon_str, color) = match status {
                        crate::agents::AgentStatus::Running => {
                            ("\u{21BB} Running", self.semantic.accent)
                        }
                        crate::agents::AgentStatus::Passed => {
                            ("\u{2713} Passed", self.semantic.success)
                        }
                        crate::agents::AgentStatus::Failed => {
                            ("\u{2717} Failed", self.semantic.danger)
                        }
                        crate::agents::AgentStatus::Error => ("! Error", self.semantic.danger),
                        crate::agents::AgentStatus::Idle => ("Idle", self.semantic.secondary_text),
                    };
                    ui.label(icon(icon_str, fs).color(color));
                }
                // Show command
                if let Some(config) = self.settings.agents.iter().find(|a| a.kind == kind) {
                    ui.separator();
                    ui.label(
                        egui::RichText::new(&config.command)
                            .monospace()
                            .small()
                            .color(self.semantic.muted_text()),
                    );
                }
            });
            ui.separator();

            if runs.is_empty() {
                ui.label(
                    egui::RichText::new("No runs recorded yet.")
                        .italics()
                        .color(self.semantic.tertiary_text),
                );
                return;
            }

            // Scrollable list of runs
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    for (idx, run) in runs.iter().enumerate() {
                        // Run header
                        let dur = if run.duration_ms < 1000 {
                            format!("{}ms", run.duration_ms)
                        } else {
                            format!("{:.1}s", run.duration_ms as f64 / 1000.0)
                        };
                        let (status_icon, status_color) = match run.status.as_str() {
                            "passed" => ("\u{2713}", self.semantic.success),
                            "failed" => ("\u{2717}", self.semantic.danger),
                            "error" => ("!", self.semantic.danger),
                            _ => ("\u{25CF}", self.semantic.secondary_text),
                        };

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

                        if idx < runs.len() - 1 {
                            ui.separator();
                        }
                    }
                });
        });

        if close {
            self.agent_state.show_output = None;
            if self.agent_state.return_to_settings {
                self.agent_state.return_to_settings = false;
                self.reload_settings_from_disk();
                self.show_settings = true;
            }
        }
    }
}
