use eframe::egui;

use super::super::super::{CueAction, DirigentApp};
use super::utils::{source_label_color, tag_badge_color};
use crate::db::{Cue, CueStatus};

impl DirigentApp {
    pub(in crate::app) fn render_badge_row(&self, ui: &mut egui::Ui, cue: &Cue) {
        // Show rate-limit / usage-limit warning prominently.
        if let Some(warning) = self.cue_warnings.get(&cue.id) {
            ui.horizontal(|ui| {
                let badge = egui::RichText::new(format!("\u{26A0} {}", warning))
                    .small()
                    .color(egui::Color32::from_rgb(180, 60, 30));
                ui.label(badge);
            });
        }

        let has_badge =
            cue.source_label.is_some() || cue.tag.is_some() || !cue.attached_images.is_empty();
        if !has_badge {
            return;
        }
        ui.horizontal(|ui| {
            if let Some(ref label) = cue.source_label {
                let badge_color = source_label_color(label);
                let badge = egui::RichText::new(label)
                    .small()
                    .background_color(badge_color)
                    .color(self.semantic.badge_text);
                ui.label(badge);
            }
            if let Some(ref tag) = cue.tag {
                let badge_color = tag_badge_color(tag);
                let badge = egui::RichText::new(format!("\u{1F3F7} {}", tag))
                    .small()
                    .background_color(badge_color)
                    .color(self.semantic.badge_text);
                ui.label(badge);
            }
            if !cue.attached_images.is_empty() {
                let plural = if cue.attached_images.len() == 1 {
                    ""
                } else {
                    "s"
                };
                ui.label(
                    egui::RichText::new(format!("{} image{}", cue.attached_images.len(), plural))
                        .small()
                        .color(self.semantic.accent),
                );
            }
        });
    }

    pub(in crate::app) fn render_file_location(
        &self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        if cue.file_path.is_empty() {
            ui.label(
                egui::RichText::new("Global")
                    .small()
                    .color(self.semantic.global_label()),
            );
            return;
        }
        let location = if let Some(end) = cue.line_number_end {
            format!("{}:{}-{}", cue.file_path, cue.line_number, end)
        } else {
            format!("{}:{}", cue.file_path, cue.line_number)
        };
        if ui
            .small_button(&location)
            .on_hover_text("Navigate to this location")
            .clicked()
        {
            actions.push((
                cue.id,
                CueAction::Navigate(cue.file_path.clone(), cue.line_number, cue.line_number_end),
            ));
        }
    }

    pub(in crate::app) fn render_run_metrics(
        &self,
        ui: &mut egui::Ui,
        cue: &Cue,
        status: CueStatus,
    ) {
        if !matches!(
            status,
            CueStatus::Review | CueStatus::Done | CueStatus::Archived
        ) {
            return;
        }
        let Some(metrics) = self.latest_exec_cache.get(&cue.id) else {
            return;
        };
        let mut parts = Vec::new();
        if let Some(turns) = metrics.num_turns {
            parts.push(format!("{} turns", turns));
        }
        if let Some(ms) = metrics.duration_ms {
            parts.push(format!("{:.1}s", ms as f64 / 1000.0));
        }
        if let Some(cost) = metrics.cost_usd {
            parts.push(format!("${:.4}", cost));
        }
        if !parts.is_empty() {
            ui.label(
                egui::RichText::new(parts.join("  "))
                    .small()
                    .color(self.semantic.muted_text()),
            );
        }
    }
}
