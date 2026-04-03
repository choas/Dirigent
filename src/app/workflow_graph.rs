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

        self.render_workflow_header(ui, &mut actions);
        ui.separator();

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

        self.render_workflow_scroll(ui, &mut actions);

        for (id, action) in actions {
            self.process_cue_action(id, action);
        }
    }

    fn render_workflow_header(&mut self, ui: &mut egui::Ui, actions: &mut Vec<(i64, CueAction)>) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Workflow Plan")
                    .size(self.settings.font_size * 1.3)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.render_header_buttons(ui, actions);
            });
        });
    }

    fn render_header_buttons(&mut self, ui: &mut egui::Ui, actions: &mut Vec<(i64, CueAction)>) {
        if ui
            .small_button(icon("\u{2715} Close", self.settings.font_size))
            .on_hover_text("Close workflow view (plan remains active)")
            .clicked()
        {
            self.show_workflow_graph = false;
        }

        if self.is_workflow_active() {
            self.render_active_workflow_buttons(ui, actions);
        } else {
            self.render_inactive_workflow_buttons(ui, actions);
        }
    }

    fn render_active_workflow_buttons(
        &self,
        ui: &mut egui::Ui,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
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
                icon("\u{2716} Cancel", self.settings.font_size).color(self.semantic.danger),
            )
            .on_hover_text("Cancel the workflow and stop all running cues")
            .clicked()
        {
            actions.push((0, CueAction::CancelWorkflow));
        }
    }

    fn render_inactive_workflow_buttons(
        &self,
        ui: &mut egui::Ui,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let has_plan = self.workflow_plan.is_some();
        let all_complete = self
            .workflow_plan
            .as_ref()
            .map_or(false, |p| p.is_complete());

        if has_plan && !all_complete {
            if ui
                .small_button(
                    icon("\u{25B6} Start", self.settings.font_size).color(self.semantic.badge_text),
                )
                .on_hover_text("Start executing the workflow plan")
                .clicked()
            {
                actions.push((0, CueAction::StartWorkflow));
            }
        }
    }

    fn render_workflow_scroll(&mut self, ui: &mut egui::Ui, actions: &mut Vec<(i64, CueAction)>) {
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if let Some(ref plan) = self.workflow_plan.clone() {
                    self.render_plan_steps(ui, &plan.steps, actions);
                } else if self.workflow_generating {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.spinner();
                        ui.label("Analyzing cues...");
                    });
                }
            });
    }

    fn render_plan_steps(
        &mut self,
        ui: &mut egui::Ui,
        steps: &[crate::workflow::WorkflowStep],
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        for (step_idx, step) in steps.iter().enumerate() {
            self.render_step_header(ui, step_idx, step, actions);

            ui.horizontal_wrapped(|ui| {
                for &cue_id in &step.cue_ids {
                    self.render_cue_node(ui, cue_id, step, actions);
                }
            });

            if step_idx < steps.len() - 1 {
                self.render_step_connector(ui, step_idx, step, actions);
            }
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

        let frame = self.cue_node_frame(step, status_color);
        frame.show(ui, |ui| {
            ui.set_min_width(160.0);
            ui.set_max_width(260.0);

            let display_text = truncate_str(cue_text, 80);
            ui.label(egui::RichText::new(display_text).strong());

            self.render_cue_file_path(ui, cue, cue_id, file_path, actions);
            self.render_cue_status_row(ui, cue_id, cue_status, status_color);
            self.render_cue_remove_button(ui, cue_id, step, actions);
        });
    }

    fn cue_node_frame(
        &self,
        step: &crate::workflow::WorkflowStep,
        status_color: egui::Color32,
    ) -> egui::Frame {
        let border_color = if step.status == WorkflowStepStatus::Running {
            self.semantic.accent
        } else {
            status_color
        };
        let fill = if self.semantic.is_dark() {
            egui::Color32::from_white_alpha(8)
        } else {
            egui::Color32::from_black_alpha(4)
        };
        egui::Frame::NONE
            .inner_margin(8.0)
            .corner_radius(6)
            .stroke(egui::Stroke::new(1.5, border_color))
            .fill(fill)
    }

    fn render_cue_file_path(
        &self,
        ui: &mut egui::Ui,
        cue: Option<&crate::db::Cue>,
        cue_id: i64,
        file_path: &str,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        if file_path.is_empty() {
            return;
        }
        let short_path = file_path.rsplit('/').next().unwrap_or(file_path);
        let clicked = ui
            .small_button(
                egui::RichText::new(short_path)
                    .small()
                    .color(self.semantic.accent),
            )
            .on_hover_text(file_path)
            .clicked();
        if let Some(c) = cue.filter(|_| clicked) {
            actions.push((
                cue_id,
                CueAction::Navigate(c.file_path.clone(), c.line_number, c.line_number_end),
            ));
        }
    }

    fn render_cue_status_row(
        &self,
        ui: &mut egui::Ui,
        cue_id: i64,
        cue_status: Option<crate::db::CueStatus>,
        status_color: egui::Color32,
    ) {
        ui.horizontal(|ui| {
            let status_text = match cue_status {
                Some(s) => s.label(),
                None => "Deleted",
            };
            ui.label(egui::RichText::new(status_text).small().color(status_color));

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
    }

    fn render_cue_remove_button(
        &self,
        ui: &mut egui::Ui,
        cue_id: i64,
        step: &crate::workflow::WorkflowStep,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        if step.status == WorkflowStepStatus::Running {
            return;
        }
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
