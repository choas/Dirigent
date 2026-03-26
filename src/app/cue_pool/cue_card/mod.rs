mod activity;
mod buttons;
mod inputs;
mod metadata;
mod overflow;
mod text;
pub(in crate::app) mod utils;

use eframe::egui;

use super::super::{CueAction, DirigentApp, SPACE_XS};
use crate::db::{Cue, CueStatus};

impl DirigentApp {
    pub(in crate::app) fn render_cue_card(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        status: CueStatus,
    ) {
        let avail_w = ui.available_width();
        let frame_resp = self.semantic.card_frame().show(ui, |ui| {
            ui.set_min_width(avail_w - 22.0);
            self.render_cue_text(ui, cue, actions, status);
            self.render_badge_row(ui, cue);
            self.render_file_location(ui, cue, actions);
            self.render_run_metrics(ui, cue, status);
            ui.add_space(SPACE_XS);
            ui.horizontal_wrapped(|ui| {
                self.render_status_buttons(ui, cue, actions);
                self.render_overflow_menu(ui, cue, actions);
            });
            self.render_schedule_input(ui, cue, actions);
            self.render_reply_input(ui, cue, actions);
            self.render_tag_input(ui, cue, actions);
            self.render_activity_logbook(ui, cue);
        });

        self.render_transition_flash(ui, cue, &frame_resp);
        ui.add_space(SPACE_XS);
    }
}
