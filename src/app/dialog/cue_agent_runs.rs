use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM, SPACE_XS};
use crate::agents::AgentKind;
use crate::db::AgentRunEntry;

impl DirigentApp {
    /// Agent runs for a specific cue, rendered in the central panel.
    pub(in crate::app) fn render_cue_agent_runs_central(&mut self, ctx: &egui::Context) {
        let cue_id = self.show_agent_runs_for_cue.unwrap();
        let cue_text = self.cue_text_truncated(cue_id);
        let runs = self.db.get_agent_runs_for_cue(cue_id).unwrap_or_default();

        let mut close = false;

        egui::CentralPanel::default().show(ctx, |ui| {
            close = self.render_agent_runs_header(ui, &cue_text);
            ui.separator();

            self.render_agent_trigger_buttons(ui);

            if runs.is_empty() {
                ui.label(
                    egui::RichText::new("No agent runs recorded for this cue.")
                        .italics()
                        .color(self.semantic.tertiary_text),
                );
                return;
            }

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    let last = runs.len() - 1;
                    for (idx, run) in runs.iter().enumerate() {
                        self.render_agent_run_entry(ui, run, idx);
                        if idx < last {
                            ui.separator();
                        }
                    }
                });
        });

        if close {
            self.show_agent_runs_for_cue = None;
        }
    }

    /// Truncated cue text for display in the header.
    fn cue_text_truncated(&self, cue_id: i64) -> String {
        self.cues
            .iter()
            .find(|c| c.id == cue_id)
            .map(|c| {
                if c.text.len() > 80 {
                    format!("{}...", crate::app::truncate_str(&c.text, 77))
                } else {
                    c.text.clone()
                }
            })
            .unwrap_or_default()
    }

    /// Render the header bar with back button and cue text. Returns `true` if close was requested.
    fn render_agent_runs_header(&self, ui: &mut egui::Ui, cue_text: &str) -> bool {
        let fs = self.settings.font_size;
        let mut close = false;
        ui.horizontal(|ui| {
            if ui.button(icon("\u{2190} Back", fs)).clicked() {
                close = true;
            }
            ui.separator();
            ui.strong("Agent Runs");
            ui.separator();
            ui.label(
                egui::RichText::new(cue_text)
                    .small()
                    .color(self.semantic.secondary_text),
            );
        });
        close
    }

    /// Render manual trigger buttons for enabled agents.
    fn render_agent_trigger_buttons(&mut self, ui: &mut egui::Ui) {
        let agent_buttons: Vec<_> = self
            .settings
            .agents
            .iter()
            .filter(|a| a.enabled && !a.command.trim().is_empty())
            .map(|a| {
                let is_running = self.agent_state.statuses.get(&a.kind).copied()
                    == Some(crate::agents::AgentStatus::Running);
                (
                    a.kind,
                    a.display_name().to_string(),
                    a.command.clone(),
                    is_running,
                )
            })
            .collect();

        if agent_buttons.is_empty() {
            return;
        }

        let mut trigger_kind = None;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Run:")
                    .small()
                    .color(self.semantic.secondary_text),
            );
            for (kind, name, command, is_running) in &agent_buttons {
                trigger_kind = trigger_kind.or(self.render_agent_button(
                    ui,
                    *kind,
                    name,
                    command,
                    *is_running,
                ));
            }
        });
        if let Some(kind) = trigger_kind {
            self.trigger_agent_manual(kind);
        }
        ui.separator();
    }

    /// Render a single agent trigger button. Returns `Some(kind)` if clicked.
    fn render_agent_button(
        &self,
        ui: &mut egui::Ui,
        kind: AgentKind,
        name: &str,
        command: &str,
        is_running: bool,
    ) -> Option<AgentKind> {
        let label = if is_running {
            format!("{} \u{21BB}", name)
        } else {
            name.to_string()
        };
        let clicked = ui
            .add_enabled(
                !is_running,
                egui::Button::new(egui::RichText::new(&label).small()),
            )
            .on_hover_text(command)
            .clicked();
        if clicked {
            Some(kind)
        } else {
            None
        }
    }

    /// Render a single agent run entry (status, command, output block).
    fn render_agent_run_entry(&self, ui: &mut egui::Ui, run: &AgentRunEntry, idx: usize) {
        let dur = crate::app::util::format_duration_ms(run.duration_ms);
        let (status_icon, status_color) = self.status_icon_and_color(&run.status);

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
        self.render_run_output_block(ui, run, idx);
    }

    /// Render the output frame for a single agent run.
    fn render_run_output_block(&self, ui: &mut egui::Ui, run: &AgentRunEntry, idx: usize) {
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
                    egui::ScrollArea::vertical()
                        .id_salt(("agent_run_output", idx))
                        .max_height(300.0)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(&run.output).monospace().small());
                        });
                }
            });
    }

    /// Map a run status string to an icon character and color.
    fn status_icon_and_color(&self, status: &str) -> (&'static str, egui::Color32) {
        match status {
            "passed" => ("\u{2713}", self.semantic.success),
            "failed" => ("\u{2717}", self.semantic.danger),
            "error" => ("!", self.semantic.danger),
            _ => ("\u{25CF}", self.semantic.secondary_text),
        }
    }
}
