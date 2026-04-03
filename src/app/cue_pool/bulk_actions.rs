use eframe::egui;

use super::super::{icon, CueAction, DirigentApp, SPACE_XS};
use crate::db::{Cue, CueStatus};

impl DirigentApp {
    pub(in crate::app) fn render_section_bulk_actions(
        &mut self,
        ui: &mut egui::Ui,
        status: CueStatus,
        section_cues: &[&Cue],
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        if status == CueStatus::Inbox && section_cues.len() >= 2 {
            self.render_inbox_bulk_actions(ui, section_cues, actions);
        }
        if status == CueStatus::Review && section_cues.len() > 1 {
            self.render_review_bulk_actions(ui, actions);
        }
        if status == CueStatus::Done {
            self.render_done_bulk_actions(ui, section_cues, actions);
        }
    }

    fn render_inbox_bulk_actions(
        &self,
        ui: &mut egui::Ui,
        section_cues: &[&Cue],
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        ui.horizontal(|ui| {
            if self.workflow_generating {
                ui.spinner();
                ui.label(
                    egui::RichText::new("Analyzing\u{2026}")
                        .small()
                        .color(self.semantic.accent),
                );
            } else if self.workflow_plan.is_some() {
                // Workflow plan exists — show "View Workflow" button
                let btn = egui::Button::new(
                    icon("\u{1F4CA} View Workflow", self.settings.font_size)
                        .color(self.semantic.badge_text),
                )
                .fill(self.semantic.accent);
                if ui
                    .add(btn)
                    .on_hover_text("View the workflow execution plan")
                    .clicked()
                {
                    // Action handled by opening the workflow graph overlay
                    // (the view toggle is handled in the code_viewer overlay dispatch)
                }
            } else {
                let btn = egui::Button::new(
                    icon("\u{26A1} Plan Workflow", self.settings.font_size)
                        .color(self.semantic.badge_text),
                )
                .fill(self.semantic.accent);
                if ui
                    .add(btn)
                    .on_hover_text(format!(
                        "Use AI to analyze these {} cues and create an optimal execution plan",
                        section_cues.len()
                    ))
                    .clicked()
                {
                    actions.push((0, CueAction::CreateWorkflow));
                }
            }
        });
        ui.add_space(SPACE_XS);
    }

    fn render_review_bulk_actions(
        &mut self,
        ui: &mut egui::Ui,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        ui.horizontal(|ui| {
            if ui
                .small_button(icon("\u{2713} Commit All", self.settings.font_size))
                .on_hover_text("Commit all uncommitted changes and move all Review cues to Done")
                .clicked()
            {
                actions.push((0, CueAction::CommitAll));
            }
            if ui
                .small_button(icon("\u{1F3F7} Tag All", self.settings.font_size))
                .on_hover_text("Add a tag to all Review cues")
                .clicked()
            {
                self.tag_all_review_input = Some(String::new());
            }
        });
        self.render_tag_all_review_input(ui, actions);
        ui.add_space(SPACE_XS);
    }

    fn render_tag_all_review_input(
        &mut self,
        ui: &mut egui::Ui,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let mut cancel_tag_all = false;
        if let Some(ref mut tag_text) = self.tag_all_review_input {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("\u{1F3F7}").color(self.semantic.accent));
                let response = ui.add(
                    egui::TextEdit::singleline(tag_text)
                        .desired_width(100.0)
                        .hint_text("Tag for all"),
                );
                let submit = ui
                    .small_button(icon("\u{2713} Set", self.settings.font_size))
                    .on_hover_text("Apply tag to all Review cues")
                    .clicked()
                    || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if submit && !tag_text.trim().is_empty() {
                    actions.push((0, CueAction::TagAllReview(tag_text.trim().to_string())));
                }
                if ui
                    .small_button("\u{2715}")
                    .on_hover_text("Cancel")
                    .clicked()
                {
                    cancel_tag_all = true;
                }
            });
        }
        if cancel_tag_all {
            self.tag_all_review_input = None;
        }
    }

    fn render_done_bulk_actions(
        &mut self,
        ui: &mut egui::Ui,
        section_cues: &[&Cue],
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let has_pr_cues = section_cues.iter().any(|c| {
            c.source_ref
                .as_ref()
                .map(|s| s.starts_with("pr"))
                .unwrap_or(false)
        });
        if !has_pr_cues {
            return;
        }
        ui.horizontal(|ui| {
            self.render_push_and_notify_button(ui, actions);
            self.render_refresh_pr_button(ui, actions);
        });
        ui.add_space(SPACE_XS);
    }

    fn render_push_and_notify_button(
        &self,
        ui: &mut egui::Ui,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        if !self.git.notifying_pr && !self.git.pushing {
            let btn = egui::Button::new(
                icon("\u{2191} Push & Notify PR", self.settings.font_size)
                    .color(self.semantic.badge_text),
            )
            .fill(self.semantic.accent);
            if ui
                .add(btn)
                .on_hover_text("Push commits and reply to all PR comments that findings were fixed")
                .clicked()
            {
                actions.push((0, CueAction::PushAndNotifyPR));
            }
        } else if self.git.notifying_pr {
            ui.label(
                egui::RichText::new("Notifying PR...")
                    .small()
                    .color(self.semantic.accent),
            );
        }
    }

    fn render_refresh_pr_button(&self, ui: &mut egui::Ui, actions: &mut Vec<(i64, CueAction)>) {
        if self.git.importing_pr {
            ui.label(
                egui::RichText::new("Refreshing PR\u{2026}")
                    .small()
                    .color(self.semantic.accent),
            );
        } else if ui
            .small_button(icon("\u{21BB} Refresh PR", self.settings.font_size))
            .on_hover_text("Re-import findings from the PR (check for new review comments)")
            .clicked()
        {
            actions.push((0, CueAction::RefreshPR));
        }
    }
}
