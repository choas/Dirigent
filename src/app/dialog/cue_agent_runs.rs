use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM, SPACE_XS};

impl DirigentApp {
    /// Agent runs for a specific cue, rendered in the central panel.
    pub(in crate::app) fn render_cue_agent_runs_central(&mut self, ctx: &egui::Context) {
        let cue_id = self.show_agent_runs_for_cue.unwrap();
        let fs = self.settings.font_size;

        let cue_text = self
            .cues
            .iter()
            .find(|c| c.id == cue_id)
            .map(|c| {
                if c.text.len() > 80 {
                    format!("{}...", &c.text[..77])
                } else {
                    c.text.clone()
                }
            })
            .unwrap_or_default();

        let runs = self.db.get_agent_runs_for_cue(cue_id).unwrap_or_default();

        let mut close = false;

        egui::CentralPanel::default().show(ctx, |ui| {
            // Header bar
            ui.horizontal(|ui| {
                if ui.button(icon("\u{2190} Back", fs)).clicked() {
                    close = true;
                }
                ui.separator();
                ui.strong("Agent Runs");
                ui.separator();
                ui.label(
                    egui::RichText::new(&cue_text)
                        .small()
                        .color(self.semantic.secondary_text),
                );
            });
            ui.separator();

            if runs.is_empty() {
                ui.label(
                    egui::RichText::new("No agent runs recorded for this cue.")
                        .italics()
                        .color(self.semantic.tertiary_text),
                );
                return;
            }

            // Manual trigger buttons
            let agent_buttons: Vec<_> = self
                .settings
                .agents
                .iter()
                .filter(|a| a.enabled)
                .map(|a| {
                    let is_running = self.agent_state.statuses.get(&a.kind).copied()
                        == Some(crate::agents::AgentStatus::Running);
                    (a.kind, a.command.clone(), is_running)
                })
                .collect();
            if !agent_buttons.is_empty() {
                let mut trigger_kind = None;
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Run:")
                            .small()
                            .color(self.semantic.secondary_text),
                    );
                    for (kind, command, is_running) in &agent_buttons {
                        let label = if *is_running {
                            format!("{} \u{21BB}", kind.label())
                        } else {
                            kind.label().to_string()
                        };
                        if ui
                            .add_enabled(
                                !is_running,
                                egui::Button::new(egui::RichText::new(&label).small()),
                            )
                            .on_hover_text(command)
                            .clicked()
                        {
                            trigger_kind = Some(*kind);
                        }
                    }
                });
                if let Some(kind) = trigger_kind {
                    self.trigger_agent_manual(kind);
                }
                ui.separator();
            }

            // Scrollable list of runs (most recent first)
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    for (idx, run) in runs.iter().enumerate() {
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

                        // Run header
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(status_icon)
                                    .strong()
                                    .color(status_color),
                            );
                            ui.label(
                                egui::RichText::new(run.agent_kind.to_uppercase())
                                    .small()
                                    .strong(),
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
                        });

                        // Command
                        if !run.command.is_empty() {
                            ui.label(
                                egui::RichText::new(format!("$ {}", &run.command))
                                    .monospace()
                                    .small()
                                    .color(self.semantic.muted_text()),
                            );
                        }

                        // Output block
                        egui::Frame::none()
                            .inner_margin(egui::Margin {
                                left: SPACE_SM,
                                top: SPACE_XS,
                                right: SPACE_XS,
                                bottom: SPACE_SM,
                            })
                            .rounding(4.0)
                            .fill(self.semantic.selection_bg())
                            .show(ui, |ui| {
                                if run.output.trim().is_empty() {
                                    ui.label(
                                        egui::RichText::new("(no output)")
                                            .italics()
                                            .color(self.semantic.tertiary_text),
                                    );
                                } else {
                                    egui::ScrollArea::vertical()
                                        .id_salt(("agent_run_output", idx))
                                        .max_height(300.0)
                                        .show(ui, |ui| {
                                            ui.label(
                                                egui::RichText::new(&run.output)
                                                    .monospace()
                                                    .small(),
                                            );
                                        });
                                }
                            });

                        if idx < runs.len() - 1 {
                            ui.separator();
                        }
                    }
                });
        });

        if close {
            self.show_agent_runs_for_cue = None;
        }
    }
}
