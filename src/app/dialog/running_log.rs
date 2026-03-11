use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM, SPACE_XS};
use crate::db::CueStatus;
use crate::settings::CliProvider;

impl DirigentApp {
    // AI provider conversation rendered in the central panel (replaces code viewer)
    pub(in crate::app) fn render_running_log_central(&mut self, ctx: &egui::Context) {
        let cue_id = self.claude.show_log.unwrap();
        let fs = self.settings.font_size;

        // Drain any pending log updates before rendering
        self.drain_log_channel();

        let is_running = self
            .cues
            .iter()
            .any(|c| c.id == cue_id && c.status == CueStatus::Ready);

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

        // Collect conversation data: past executions + current running log
        let past_execs = self.claude.conversation_history.clone();
        let (current_running_log, current_provider) = self
            .claude
            .running_logs
            .get(&cue_id)
            .cloned()
            .unwrap_or((String::new(), CliProvider::Claude));
        let current_exec_id = self.claude.exec_ids.get(&cue_id).copied();

        let mut close = false;

        egui::CentralPanel::default().show(ctx, |ui| {
            // Header bar
            ui.horizontal(|ui| {
                if ui.button(icon("\u{2190} Back", fs)).clicked() {
                    close = true;
                }
                ui.separator();
                ui.strong("Conversation");
                ui.separator();
                if is_running {
                    let elapsed = self.format_elapsed(cue_id);
                    let status = if elapsed.is_empty() {
                        "\u{25CF} Running".to_string()
                    } else {
                        format!("\u{25CF} Running ({})", elapsed)
                    };
                    ui.label(icon(&status, fs).color(self.semantic.accent));
                    ui.ctx()
                        .request_repaint_after(super::super::ELAPSED_REPAINT);
                } else {
                    ui.label(icon("\u{2713} Completed", fs).color(self.semantic.success));
                }
                ui.separator();
                ui.label(
                    egui::RichText::new(&cue_text)
                        .small()
                        .color(self.semantic.secondary_text),
                );
            });
            ui.separator();

            // Conversation scroll area
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    let user_color = self.semantic.accent;

                    if past_execs.is_empty() && current_running_log.is_empty() {
                        let msg = if is_running {
                            "Waiting for output..."
                        } else {
                            "No output recorded."
                        };
                        ui.label(
                            egui::RichText::new(msg)
                                .italics()
                                .color(self.semantic.tertiary_text),
                        );
                    }

                    for (idx, exec) in past_execs.iter().enumerate() {
                        let is_current_running = current_exec_id == Some(exec.id);
                        let exec_provider_name = exec.provider.display_name();
                        let exec_provider_color = self.semantic.provider_color(&exec.provider);

                        // -- User message --
                        let user_text = crate::claude::extract_user_text_from_prompt(&exec.prompt);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("You").strong().color(user_color));
                            if idx > 0 {
                                ui.label(
                                    egui::RichText::new(format!("(reply #{})", idx))
                                        .small()
                                        .color(self.semantic.tertiary_text),
                                );
                            }
                        });
                        egui::Frame::none()
                            .inner_margin(egui::Margin {
                                left: SPACE_SM,
                                top: SPACE_XS,
                                right: SPACE_XS,
                                bottom: SPACE_SM,
                            })
                            .show(ui, |ui| {
                                ui.label(&user_text);
                            });

                        // -- Provider response --
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(exec_provider_name)
                                    .strong()
                                    .color(exec_provider_color),
                            );
                        });
                        egui::Frame::none()
                            .inner_margin(egui::Margin {
                                left: SPACE_SM,
                                top: SPACE_XS,
                                right: SPACE_XS,
                                bottom: SPACE_SM,
                            })
                            .show(ui, |ui| {
                                if is_current_running {
                                    // Show live streaming log for the currently running execution
                                    if current_running_log.is_empty() {
                                        ui.label(
                                            egui::RichText::new("Waiting for output...")
                                                .italics()
                                                .color(self.semantic.tertiary_text),
                                        );
                                    } else {
                                        ui.label(
                                            egui::RichText::new(&current_running_log)
                                                .monospace()
                                                .small(),
                                        );
                                    }
                                } else if let Some(ref log_text) = exec.log {
                                    if !log_text.is_empty() {
                                        ui.label(egui::RichText::new(log_text).monospace().small());
                                    } else {
                                        ui.label(
                                            egui::RichText::new("(no output)")
                                                .italics()
                                                .color(self.semantic.tertiary_text),
                                        );
                                    }
                                } else {
                                    ui.label(
                                        egui::RichText::new("(no output)")
                                            .italics()
                                            .color(self.semantic.tertiary_text),
                                    );
                                }
                            });

                        if idx < past_execs.len() - 1 {
                            ui.separator();
                        }
                    }

                    // If currently running but not yet in past_execs (just started)
                    if is_running
                        && current_exec_id.is_some()
                        && !past_execs.iter().any(|e| Some(e.id) == current_exec_id)
                    {
                        if !past_execs.is_empty() {
                            ui.separator();
                        }
                        let current_provider_name = current_provider.display_name();
                        let current_provider_color =
                            self.semantic.provider_color(&current_provider);
                        // Show the user's prompt from running_logs context
                        // (the execution hasn't been saved to history yet)
                        ui.label(
                            egui::RichText::new(current_provider_name)
                                .strong()
                                .color(current_provider_color),
                        );
                        egui::Frame::none()
                            .inner_margin(egui::Margin {
                                left: SPACE_SM,
                                top: SPACE_XS,
                                right: SPACE_XS,
                                bottom: SPACE_SM,
                            })
                            .show(ui, |ui| {
                                if current_running_log.is_empty() {
                                    ui.label(
                                        egui::RichText::new("Waiting for output...")
                                            .italics()
                                            .color(self.semantic.tertiary_text),
                                    );
                                } else {
                                    ui.label(
                                        egui::RichText::new(&current_running_log)
                                            .monospace()
                                            .small(),
                                    );
                                }
                            });
                    }
                });
        });

        if close {
            self.claude.show_log = None;
        }
    }
}
