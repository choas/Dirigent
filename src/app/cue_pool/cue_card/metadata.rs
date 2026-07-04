use eframe::egui;

use super::super::super::{CueAction, DirigentApp};
use super::utils::{source_label_color, tag_badge_color};
use crate::db::{Cue, CueStatus};

impl DirigentApp {
    pub(in crate::app) fn render_badge_row(&self, ui: &mut egui::Ui, cue: &Cue) {
        self.render_cue_warning_badge(ui, cue);

        if !has_metadata_badge(cue) {
            return;
        }
        ui.horizontal(|ui| {
            self.render_cue_metadata_badges(ui, cue);
        });
    }

    fn render_cue_warning_badge(&self, ui: &mut egui::Ui, cue: &Cue) {
        let Some(warning) = self.cue_warnings.get(&cue.id) else {
            return;
        };
        ui.horizontal(|ui| {
            let badge = egui::RichText::new(format!("\u{26A0} {}", warning))
                .small()
                .color(ui.visuals().warn_fg_color);
            ui.label(badge);
        });
    }

    fn render_cue_metadata_badges(&self, ui: &mut egui::Ui, cue: &Cue) {
        if cue.workflow {
            self.render_static_badge(ui, "Workflow", egui::Color32::from_rgb(44, 138, 186));
        }
        if cue.auto_commit {
            self.render_static_badge(ui, "Auto-commit", egui::Color32::from_rgb(38, 154, 108));
        }
        if let Some(ref label) = cue.source_label {
            self.render_static_badge(ui, label, source_label_color(label));
        }
        if let Some(ref tag) = cue.tag {
            self.render_static_badge(ui, &format!("\u{1F3F7} {}", tag), tag_badge_color(tag));
        }
        if !cue.attached_images.is_empty() {
            self.render_image_badge(ui, cue.attached_images.len());
        }
    }

    fn render_static_badge(&self, ui: &mut egui::Ui, label: &str, color: egui::Color32) {
        let badge = egui::RichText::new(label)
            .small()
            .background_color(color)
            .color(self.semantic.badge_text);
        ui.label(badge);
    }

    fn render_image_badge(&self, ui: &mut egui::Ui, count: usize) {
        let plural = if count == 1 { "" } else { "s" };
        ui.label(
            egui::RichText::new(format!("{} image{}", count, plural))
                .small()
                .color(self.semantic.accent),
        );
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

fn has_metadata_badge(cue: &Cue) -> bool {
    cue.source_label.is_some()
        || cue.tag.is_some()
        || !cue.attached_images.is_empty()
        || cue.workflow
        || cue.auto_commit
}
