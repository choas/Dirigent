use eframe::egui;

use super::super::super::{icon, CueAction, DirigentApp};
use crate::db::Cue;

impl DirigentApp {
    pub(in crate::app) fn render_overflow_menu(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let fs = self.settings.font_size;
        let more_btn = ui.small_button("\u{2026}").on_hover_text("More actions");
        egui::Popup::menu(&more_btn)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(140.0);
                self.render_overflow_activity_toggle(ui, cue);
                self.render_overflow_tag_items(ui, cue, actions);
                self.render_overflow_create_pr(ui, cue, actions);
                ui.separator();
                let split_enabled = !self.split_cue_generating
                    && matches!(
                        cue.status,
                        crate::db::CueStatus::Inbox | crate::db::CueStatus::Backlog
                    );
                let split_btn =
                    ui.add_enabled(split_enabled, egui::Button::new(icon("\u{2702} Split", fs)));
                if split_btn.clicked() {
                    actions.push((cue.id, CueAction::SplitCue));
                }
                if ui.button(icon("\u{2715} Delete", fs)).clicked() {
                    actions.push((cue.id, CueAction::Delete));
                }
            });
    }

    fn render_overflow_activity_toggle(&mut self, ui: &mut egui::Ui, cue: &Cue) {
        let is_expanded = self.logbook_expanded.contains(&cue.id);
        let activity_label = if is_expanded {
            "\u{25BE} Activity"
        } else {
            "\u{25B8} Activity"
        };
        if ui.button(activity_label).clicked() {
            if is_expanded {
                self.logbook_expanded.remove(&cue.id);
            } else {
                // Invalidate cache so we fetch fresh data when expanding.
                self.activity_cache.remove(&cue.id);
                self.logbook_expanded.insert(cue.id);
            }
        }
    }

    fn render_overflow_tag_items(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let tag_label = if cue.tag.is_some() {
            "\u{1F3F7} Edit Tag"
        } else {
            "\u{1F3F7} Add Tag"
        };
        if ui.button(tag_label).clicked() {
            let current = cue.tag.clone().unwrap_or_default();
            self.tag_inputs.insert(cue.id, current);
        }
        if cue.tag.is_some() && ui.button("\u{2715} Remove Tag").clicked() {
            actions.push((cue.id, CueAction::SetTag(None)));
        }
    }

    fn render_overflow_create_pr(
        &self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let is_default_branch = self
            .git
            .info
            .as_ref()
            .map(|i| i.branch == "main" || i.branch == "master")
            .unwrap_or(true);
        if !is_default_branch && !self.git.creating_pr && ui.button("Create PR").clicked() {
            actions.push((cue.id, CueAction::CreatePR));
        }
    }
}
