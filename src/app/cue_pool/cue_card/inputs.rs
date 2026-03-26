use eframe::egui;

use super::super::super::{icon, CueAction, DirigentApp, SPACE_XS};
use crate::db::{Cue, CueStatus};

impl DirigentApp {
    pub(in crate::app) fn render_schedule_input(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let fs = self.settings.font_size;
        let mut cancel_schedule = false;
        if let Some(schedule_text) = self.schedule_inputs.get_mut(&cue.id) {
            ui.add_space(SPACE_XS);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("\u{23F2}").color(self.semantic.accent));
                let response = ui.add(
                    egui::TextEdit::singleline(schedule_text)
                        .desired_width(60.0)
                        .hint_text("5m, 2h"),
                );
                let submit = ui
                    .small_button(icon("\u{2713} Go", fs))
                    .on_hover_text("Schedule this run")
                    .clicked()
                    || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if submit && !schedule_text.trim().is_empty() {
                    actions.push((cue.id, CueAction::ScheduleRun(schedule_text.clone())));
                }
                if ui
                    .small_button("\u{2715}")
                    .on_hover_text("Cancel")
                    .clicked()
                {
                    cancel_schedule = true;
                }
            });
        }
        if cancel_schedule {
            self.schedule_inputs.remove(&cue.id);
        }
    }

    pub(in crate::app) fn render_reply_input(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let fs = self.settings.font_size;
        let is_running = cue.status == CueStatus::Ready;
        let Some(reply_text) = self.reply_inputs.get_mut(&cue.id) else {
            return;
        };
        ui.add_space(SPACE_XS);
        let hint = if is_running {
            "Follow-up prompt (sent when this run completes)..."
        } else {
            "Describe what needs to change..."
        };
        let response = ui.add(
            egui::TextEdit::multiline(reply_text)
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .hint_text(hint),
        );
        let (btn_label, btn_hover) = if is_running {
            (
                "\u{23F3} Queue",
                "Queue follow-up for when this run completes (also Cmd+Enter)",
            )
        } else {
            ("\u{25B6} Send", "Send feedback to Claude (also Cmd+Enter)")
        };
        let submit = ui
            .small_button(icon(btn_label, fs))
            .on_hover_text(btn_hover)
            .clicked()
            || (response.has_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command));
        if submit && !reply_text.trim().is_empty() {
            let action = if is_running {
                CueAction::QueueFollowUp(cue.id, reply_text.clone())
            } else {
                CueAction::ReplyReview(cue.id, reply_text.clone())
            };
            actions.push((cue.id, action));
        }
    }

    pub(in crate::app) fn render_tag_input(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let fs = self.settings.font_size;
        let mut cancel_tag = false;
        if let Some(tag_text) = self.tag_inputs.get_mut(&cue.id) {
            ui.add_space(SPACE_XS);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("\u{1F3F7}").color(self.semantic.accent));
                let response = ui.add(
                    egui::TextEdit::singleline(tag_text)
                        .desired_width(100.0)
                        .hint_text("Tag name"),
                );
                let submit = ui
                    .small_button(icon("\u{2713} Set", fs))
                    .on_hover_text("Set tag")
                    .clicked()
                    || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if submit {
                    let tag_val = if tag_text.trim().is_empty() {
                        None
                    } else {
                        Some(tag_text.trim().to_string())
                    };
                    actions.push((cue.id, CueAction::SetTag(tag_val)));
                }
                if ui
                    .small_button("\u{2715}")
                    .on_hover_text("Cancel")
                    .clicked()
                {
                    cancel_tag = true;
                }
            });
        }
        if cancel_tag {
            self.tag_inputs.remove(&cue.id);
        }
    }
}
