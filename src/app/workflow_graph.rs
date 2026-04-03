use eframe::egui;

use super::{icon, truncate_str, CueAction, DirigentApp, SPACE_SM, SPACE_XS};
use crate::workflow::WorkflowStepStatus;

impl DirigentApp {
    /// Render the workflow graph as a central panel overlay.
    pub(super) fn render_workflow_graph_central(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.render_workflow_graph_panel(ui);
        });
    }

    fn render_workflow_graph_panel(&mut self, ui: &mut egui::Ui) {
        let mut actions: Vec<(i64, CueAction)> = Vec::new();

        // Header
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Workflow Plan")
                    .size(self.settings.font_size * 1.3)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button(icon("\u{2715} Close", self.settings.font_size))
                    .on_hover_text("Close workflow view (plan remains active)")
                    .clicked()
                {
                    self.show_workflow_graph = false;
                }

                if self.is_workflow_active() {
                    let has_paused = self.workflow_plan.as_ref().map_or(false, |p| {
                        p.steps
                            .iter()
                            .any(|s| s.status == WorkflowStepStatus::PausedAwaitingReview)
                    });
                    if has_paused {
                        if ui
                            .small_button(icon("\u{25B6} Resume", self.settings.font_size))
                            .on_hover_text("Resume workflow from paused step")
                            .clicked()
                        {
                            actions.push((0, CueAction::ResumeWorkflow));
                        }
                    }
                    if ui
                        .small_button(
                            icon("\u{2716} Cancel", self.settings.font_size)
                                .color(self.semantic.danger),
                        )
                        .on_hover_text("Cancel the workflow and stop all running cues")
                        .clicked()
                    {
                        actions.push((0, CueAction::CancelWorkflow));
                    }
                } else {
                    let has_plan = self.workflow_plan.is_some();
                    let all_complete = self
                        .workflow_plan
                        .as_ref()
                        .map_or(false, |p| p.is_complete());

                    if has_plan && !all_complete {
                        if ui
                            .small_button(
                                icon("\u{25B6} Start", self.settings.font_size)
                                    .color(self.semantic.badge_text),
                            )
                            .on_hover_text("Start executing the workflow plan")
                            .clicked()
                        {
                            actions.push((0, CueAction::StartWorkflow));
                        }
                    }
                }
            });
        });

        ui.separator();

        // Warning message if any
        if let Some(ref warning) = self.workflow_warning {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("\u{26A0} {}", warning))
                        .color(self.semantic.warning)
                        .small(),
                );
            });
            ui.add_space(SPACE_XS);
        }

        // Scroll area for the graph
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if let Some(ref plan) = self.workflow_plan.clone() {
                    for (step_idx, step) in plan.steps.iter().enumerate() {
                        // Step header
                        self.render_step_header(ui, step_idx, step, &mut actions);

                        // Cue nodes within the step
                        ui.horizontal_wrapped(|ui| {
                            for &cue_id in &step.cue_ids {
                                self.render_cue_node(ui, cue_id, step, &mut actions);
                            }
                        });

                        // Connector / pause indicator between steps
                        if step_idx < plan.steps.len() - 1 {
                            self.render_step_connector(ui, step_idx, step, &mut actions);
                        }
                    }
                } else if self.workflow_generating {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.spinner();
                        ui.label("Analyzing cues...");
                    });
                }
            });

        // Process actions
        for (id, action) in actions {
            self.process_cue_action(id, action);
        }
    }

    fn render_step_header(
        &self,
        ui: &mut egui::Ui,
        step_idx: usize,
        step: &crate::workflow::WorkflowStep,
        _actions: &mut Vec<(i64, CueAction)>,
    ) {
        let status_icon = match step.status {
            WorkflowStepStatus::Pending => "\u{25CB}", // ○
            WorkflowStepStatus::Running => "\u{25CF}", // ●
            WorkflowStepStatus::PausedAwaitingReview => "\u{23F8}", // ⏸
            WorkflowStepStatus::Completed => "\u{2713}", // ✓
            WorkflowStepStatus::Failed => "\u{2717}",  // ✗
        };
        let status_color = self.step_status_color(step.status);

        ui.add_space(SPACE_SM);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{} Step {}", status_icon, step_idx))
                    .color(status_color)
                    .strong(),
            );
            if !step.label.is_empty() {
                ui.label(
                    egui::RichText::new(format!("— {}", step.label))
                        .color(self.semantic.secondary_text),
                );
            }
            let parallel_label = if step.cue_ids.len() > 1 {
                format!("({} parallel)", step.cue_ids.len())
            } else {
                "(sequential)".to_string()
            };
            ui.label(
                egui::RichText::new(parallel_label)
                    .small()
                    .color(self.semantic.tertiary_text),
            );
        });

        if !step.rationale.is_empty() {
            ui.label(
                egui::RichText::new(format!("  {}", step.rationale))
                    .small()
                    .italics()
                    .color(self.semantic.tertiary_text),
            );
        }
    }

    fn render_cue_node(
        &self,
        ui: &mut egui::Ui,
        cue_id: i64,
        step: &crate::workflow::WorkflowStep,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let cue = self.cues.iter().find(|c| c.id == cue_id);
        let cue_text = cue.map(|c| c.text.as_str()).unwrap_or("(deleted)");
        let file_path = cue.map(|c| c.file_path.as_str()).unwrap_or("");
        let cue_status = cue.map(|c| c.status);

        let status_color = self.step_status_color(step.status);
        let border_color = if step.status == WorkflowStepStatus::Running {
            self.semantic.accent
        } else {
            status_color
        };

        let frame = egui::Frame::NONE
            .inner_margin(8.0)
            .corner_radius(6)
            .stroke(egui::Stroke::new(1.5, border_color))
            .fill(if self.semantic.is_dark() {
                egui::Color32::from_white_alpha(8)
            } else {
                egui::Color32::from_black_alpha(4)
            });

        frame.show(ui, |ui| {
            ui.set_min_width(160.0);
            ui.set_max_width(260.0);

            // Cue text (truncated)
            let display_text = truncate_str(cue_text, 80);
            ui.label(egui::RichText::new(display_text).strong());

            // File path
            if !file_path.is_empty() {
                let short_path = file_path.rsplit('/').next().unwrap_or(file_path);
                if ui
                    .small_button(
                        egui::RichText::new(short_path)
                            .small()
                            .color(self.semantic.accent),
                    )
                    .on_hover_text(file_path)
                    .clicked()
                {
                    if let Some(c) = cue {
                        actions.push((
                            cue_id,
                            CueAction::Navigate(
                                c.file_path.clone(),
                                c.line_number,
                                c.line_number_end,
                            ),
                        ));
                    }
                }
            }

            // Status indicator
            ui.horizontal(|ui| {
                let status_text = match cue_status {
                    Some(s) => s.label(),
                    None => "Deleted",
                };
                ui.label(egui::RichText::new(status_text).small().color(status_color));

                // Running elapsed time
                if cue_status == Some(crate::db::CueStatus::Ready) {
                    let elapsed = self.format_elapsed(cue_id);
                    if !elapsed.is_empty() {
                        ui.label(
                            egui::RichText::new(elapsed)
                                .small()
                                .color(self.semantic.accent),
                        );
                    }
                }
            });

            // Remove from workflow button (only if plan is not running)
            let is_running = step.status == WorkflowStepStatus::Running;
            if !is_running {
                ui.horizontal(|ui| {
                    if ui
                        .small_button(
                            egui::RichText::new("\u{2715}")
                                .small()
                                .color(self.semantic.tertiary_text),
                        )
                        .on_hover_text("Remove from workflow")
                        .clicked()
                    {
                        actions.push((cue_id, CueAction::RemoveFromWorkflow(cue_id)));
                    }
                });
            }
        });
    }

    fn render_step_connector(
        &self,
        ui: &mut egui::Ui,
        step_idx: usize,
        step: &crate::workflow::WorkflowStep,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        ui.add_space(SPACE_XS);
        ui.horizontal(|ui| {
            // Vertical line
            ui.add_space(20.0);
            ui.label(
                egui::RichText::new("\u{2502}") // │
                    .color(self.semantic.separator),
            );

            // Pause toggle button between steps
            let pause_label = if step.pause_after {
                egui::RichText::new("\u{23F8} Pause here")
                    .color(self.semantic.warning)
                    .small()
            } else {
                egui::RichText::new("\u{23F8} Add pause")
                    .color(self.semantic.tertiary_text)
                    .small()
            };
            if ui
                .small_button(pause_label)
                .on_hover_text("Toggle pause point after this step")
                .clicked()
            {
                actions.push((0, CueAction::TogglePause(step_idx)));
            }
        });

        // Show pause indicator if active
        if step.pause_after {
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new("── PAUSE ──")
                        .color(self.semantic.warning)
                        .small()
                        .strong(),
                );
            });
        }

        ui.add_space(SPACE_XS);
    }

    fn step_status_color(&self, status: WorkflowStepStatus) -> egui::Color32 {
        match status {
            WorkflowStepStatus::Pending => self.semantic.tertiary_text,
            WorkflowStepStatus::Running => self.semantic.accent,
            WorkflowStepStatus::PausedAwaitingReview => self.semantic.warning,
            WorkflowStepStatus::Completed => self.semantic.success,
            WorkflowStepStatus::Failed => self.semantic.danger,
        }
    }
}
