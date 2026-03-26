use eframe::egui;

use super::super::super::DirigentApp;
use super::utils::{detect_agent_kind, format_agent_output};
use crate::db::Cue;

impl DirigentApp {
    pub(in crate::app) fn render_activity_logbook(&mut self, ui: &mut egui::Ui, cue: &Cue) {
        if !self.logbook_expanded.contains(&cue.id) {
            return;
        }
        let entries = match self.db.get_activities(cue.id) {
            Ok(e) => e,
            Err(_) => return,
        };
        if entries.is_empty() {
            ui.label(
                egui::RichText::new("No activity yet")
                    .small()
                    .color(self.semantic.muted_text()),
            );
            return;
        }
        let agent_runs = self.db.get_agent_runs_for_cue(cue.id).unwrap_or_default();
        for entry in &entries {
            let agent_kind = detect_agent_kind(&entry.event);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&entry.timestamp)
                        .small()
                        .color(self.semantic.muted_text()),
                );
                if agent_kind.is_some() {
                    self.render_agent_event_toggle(ui, cue, entry);
                } else {
                    ui.label(egui::RichText::new(&entry.event).small());
                }
            });

            if let Some(ref kind_label) = agent_kind {
                self.render_agent_output_block(ui, cue, entry, kind_label, &agent_runs);
            }
        }
    }

    pub(in crate::app) fn render_agent_event_toggle(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        entry: &crate::db::ActivityEntry,
    ) {
        let key = (cue.id, entry.timestamp.clone());
        let is_expanded = self.agent_output_expanded.contains(&key);
        let arrow = if is_expanded { "\u{25BE}" } else { "\u{25B8}" };
        let clicked = ui
            .add(
                egui::Label::new(
                    egui::RichText::new(format!("{} {}", arrow, &entry.event)).small(),
                )
                .sense(egui::Sense::click()),
            )
            .clicked();
        if clicked {
            if is_expanded {
                self.agent_output_expanded.remove(&key);
            } else {
                self.agent_output_expanded.insert(key);
            }
        }
    }

    pub(in crate::app) fn render_agent_output_block(
        &self,
        ui: &mut egui::Ui,
        cue: &Cue,
        entry: &crate::db::ActivityEntry,
        kind_label: &str,
        agent_runs: &[crate::db::AgentRunEntry],
    ) {
        let key = (cue.id, entry.timestamp.clone());
        if !self.agent_output_expanded.contains(&key) {
            return;
        }
        let kind_str = kind_label.to_lowercase();
        let run = agent_runs
            .iter()
            .find(|r| r.agent_kind == kind_str && r.started_at == entry.timestamp)
            .or_else(|| agent_runs.iter().find(|r| r.agent_kind == kind_str));
        let Some(run) = run else { return };
        let output = format_agent_output(&run.output);
        egui::Frame::NONE
            .inner_margin(egui::Margin::same(4))
            .corner_radius(4)
            .fill(self.semantic.selection_bg())
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(&output)
                                .monospace()
                                .small()
                                .color(self.semantic.muted_text()),
                        );
                    });
            });
    }

    pub(in crate::app) fn render_transition_flash(
        &self,
        ui: &mut egui::Ui,
        cue: &Cue,
        frame_resp: &egui::InnerResponse<()>,
    ) {
        let Some(&when) = self.cue_move_flash.get(&cue.id) else {
            return;
        };
        let elapsed = when.elapsed().as_secs_f32();
        if elapsed >= 0.6 {
            return;
        }
        let alpha = ((0.6 - elapsed) / 0.6 * 50.0) as u8;
        let [r, g, b, _] = self.semantic.accent.to_array();
        ui.painter().rect_filled(
            frame_resp.response.rect,
            8,
            egui::Color32::from_rgba_premultiplied(r, g, b, alpha),
        );
        ui.ctx().request_repaint();
    }
}
