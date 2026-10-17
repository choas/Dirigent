use eframe::egui;

use super::super::super::DirigentApp;
use super::utils::{detect_agent_event, format_agent_output, nearest_agent_run};
use crate::agents::AgentKind;
use crate::db::Cue;

/// Key identifying one activity entry in the expanded-output set. Includes
/// the event text so two entries logged in the same second stay independent.
fn entry_key(cue: &Cue, entry: &crate::db::ActivityEntry) -> (i64, String) {
    (cue.id, format!("{}\u{1F}{}", entry.timestamp, entry.event))
}

impl DirigentApp {
    pub(in crate::app) fn render_activity_logbook(&mut self, ui: &mut egui::Ui, cue: &Cue) {
        if !self.logbook_expanded.contains(&cue.id) {
            return;
        }
        // Use cached activity data to avoid DB queries every frame.
        // Cap cache size to prevent unbounded memory growth.
        const ACTIVITY_CACHE_CAP: usize = 50;
        if !self.activity_cache.contains_key(&cue.id) {
            if self.activity_cache.len() >= ACTIVITY_CACHE_CAP {
                self.activity_cache.clear();
            }
            let entries = self.db.get_activities(cue.id).unwrap_or_default();
            let agent_runs = self.db.get_agent_runs_for_cue(cue.id).unwrap_or_default();
            self.activity_cache.insert(cue.id, (entries, agent_runs));
        }
        // Clone out of cache to avoid holding a borrow on self during rendering.
        let Some(cached) = self.activity_cache.get(&cue.id) else {
            return;
        };
        let entries = cached.0.clone();
        let agent_runs = cached.1.clone();
        if entries.is_empty() {
            ui.label(
                egui::RichText::new("No activity yet")
                    .small()
                    .color(self.semantic.muted_text()),
            );
            return;
        }
        let agent_names = self.agent_event_names();
        for entry in &entries {
            let agent_key = detect_agent_event(&entry.event, &agent_names);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&entry.timestamp)
                        .small()
                        .color(self.semantic.muted_text()),
                );
                if agent_key.is_some() {
                    self.render_agent_event_toggle(ui, cue, entry);
                } else {
                    ui.label(egui::RichText::new(&entry.event).small());
                }
            });

            if let Some(ref db_key) = agent_key {
                self.render_agent_output_block(ui, cue, entry, db_key, &agent_runs);
            }
        }
    }

    /// Display name -> db key pairs used to recognize agent events in the
    /// activity log. Configured agents come first (they may have custom
    /// names); builtin kind labels are kept as fallback for historical
    /// entries logged before a rename or after an agent was removed.
    fn agent_event_names(&self) -> Vec<(String, String)> {
        let mut names: Vec<(String, String)> = self
            .settings
            .agents
            .iter()
            .map(|a| (a.display_name().to_string(), a.kind.db_key()))
            .collect();
        for kind in [
            AgentKind::Format,
            AgentKind::Lint,
            AgentKind::Build,
            AgentKind::Test,
            AgentKind::Outdated,
            AgentKind::Audit,
        ] {
            names.push((kind.label().to_string(), kind.db_key()));
        }
        names
    }

    pub(in crate::app) fn render_agent_event_toggle(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        entry: &crate::db::ActivityEntry,
    ) {
        let key = entry_key(cue, entry);
        let is_expanded = self.agent_output_expanded.contains(&key);
        let arrow = if is_expanded { "\u{25BE}" } else { "\u{25B8}" };
        let clicked = ui
            .add(
                egui::Label::new(egui::RichText::new(format!("{} {}", arrow, entry.event)).small())
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
        agent_db_key: &str,
        agent_runs: &[crate::db::AgentRunEntry],
    ) {
        let key = entry_key(cue, entry);
        if !self.agent_output_expanded.contains(&key) {
            return;
        }
        let run = agent_runs
            .iter()
            .find(|r| r.agent_kind == agent_db_key && r.started_at == entry.timestamp)
            .or_else(|| nearest_agent_run(agent_runs, agent_db_key, &entry.timestamp))
            .or_else(|| agent_runs.iter().find(|r| r.agent_kind == agent_db_key));
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
