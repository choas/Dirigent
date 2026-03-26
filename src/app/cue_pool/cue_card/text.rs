use eframe::egui;

use super::super::super::{icon, CueAction, DirigentApp};
use super::utils::compute_display_text;
use crate::db::{Cue, CueStatus};

impl DirigentApp {
    pub(in crate::app) fn render_cue_text(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        status: CueStatus,
    ) {
        let fs = self.settings.font_size;
        let is_editing = self.editing_cue.as_ref().map(|e| e.id) == Some(cue.id);
        if is_editing {
            self.render_editing_cue(ui, cue, actions, fs);
        } else {
            self.render_display_cue(ui, cue, actions, status);
        }
    }

    pub(in crate::app) fn render_editing_cue(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        fs: f32,
    ) {
        let editing = self.editing_cue.as_mut().unwrap();
        let response = ui.text_edit_multiline(&mut editing.text);
        ui.horizontal(|ui| {
            if ui.small_button(icon("\u{2713} Save", fs)).clicked() {
                actions.push((
                    cue.id,
                    CueAction::SaveEdit(self.editing_cue.as_ref().unwrap().text.clone()),
                ));
            }
            if ui.small_button(icon("\u{2715} Cancel", fs)).clicked() {
                actions.push((cue.id, CueAction::CancelEdit));
            }
        });
        let editing = self.editing_cue.as_mut().unwrap();
        if !editing.focus_requested {
            response.request_focus();
            editing.focus_requested = true;
        }
    }

    pub(in crate::app) fn render_display_cue(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        status: CueStatus,
    ) {
        let display_text = compute_display_text(cue, self.cue_text_expanded.contains(&cue.id));
        let label_response = ui.add(egui::Label::new(&display_text).wrap());

        if matches!(status, CueStatus::Inbox | CueStatus::Backlog)
            && label_response.double_clicked()
        {
            actions.push((cue.id, CueAction::StartEdit(cue.text.clone())));
        }
        if matches!(
            status,
            CueStatus::Review | CueStatus::Done | CueStatus::Archived
        ) && label_response.clicked()
        {
            actions.push((cue.id, CueAction::ShowDiff(cue.id)));
        }

        self.render_expand_collapse_toggle(ui, cue);
    }

    pub(in crate::app) fn render_expand_collapse_toggle(&mut self, ui: &mut egui::Ui, cue: &Cue) {
        let line_count = cue.text.lines().count();
        let word_count = cue.text.split_whitespace().count();
        let is_long = line_count > 10 || word_count > 50;
        if !is_long {
            return;
        }
        let is_expanded = self.cue_text_expanded.contains(&cue.id);
        let toggle_label = if is_expanded {
            "\u{25B4} Show less"
        } else {
            "\u{25BE} Show more"
        };
        let clicked = ui
            .add(
                egui::Label::new(
                    egui::RichText::new(toggle_label)
                        .small()
                        .color(self.semantic.accent),
                )
                .sense(egui::Sense::click()),
            )
            .clicked();
        if clicked {
            if is_expanded {
                self.cue_text_expanded.remove(&cue.id);
            } else {
                self.cue_text_expanded.insert(cue.id);
            }
        }
    }
}
