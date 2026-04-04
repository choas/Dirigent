mod actions;
mod bulk_actions;
mod cue_card;
mod helpers;
mod history;
mod markdown_import;

use std::collections::HashMap;

use eframe::egui;

use super::{CueAction, DirigentApp, PendingPlay, FONT_SCALE_SUBHEADING};
use crate::db::{Cue, CueStatus};

/// (text, file_path, line_number, line_number_end, attached_images)
type ReuseCueData = (String, String, usize, Option<usize>, Vec<String>);

use crate::settings;

use helpers::{
    build_heading_text, build_section_header, collect_unique_labels,
    filter_cues_by_status_and_source, format_import_message, render_cue_pool_buttons,
};
use markdown_import::{parse_markdown_sections, pick_markdown_file};

impl DirigentApp {
    pub(super) fn render_cue_pool(&mut self, ui: &mut egui::Ui) {
        // Clean up expired transition flashes
        self.cue_move_flash
            .retain(|_, when| when.elapsed().as_secs_f32() < 1.0);

        egui::Panel::right("cue_pool")
            .default_size(250.0)
            .min_size(200.0)
            .max_size(500.0)
            .show_inside(ui, |ui| {
                let (selected_play_prompt, custom_cue_requested, import_requested) =
                    self.render_cue_pool_header(ui);
                self.handle_playbook_selection(selected_play_prompt);
                self.handle_custom_cue_request(custom_cue_requested);
                self.handle_import_request(import_requested);
                self.render_source_filter(ui);
                let reuse_cue = self.render_prompt_history(ui);
                self.handle_reuse_cue(reuse_cue);

                let panel_rect = ui.max_rect();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut actions: Vec<(i64, CueAction)> = Vec::new();
                    let mut load_more_archived = false;

                    let cues_snapshot = self.cues.clone();
                    let source_filter = self.sources.filter.clone();
                    let filtered_archived_count = self.cached_filtered_archived_count;
                    for &status in CueStatus::all() {
                        let section_cues: Vec<&Cue> = filter_cues_by_status_and_source(
                            &cues_snapshot,
                            status,
                            &source_filter,
                        );
                        self.render_cue_section(
                            ui,
                            status,
                            &section_cues,
                            &mut actions,
                            &mut load_more_archived,
                            filtered_archived_count,
                        );
                    }

                    if load_more_archived {
                        self.archived_cue_limit += 50;
                        self.reload_cues();
                    }

                    self.claude.expand_running = false;

                    for (id, action) in actions {
                        self.process_cue_action(id, action);
                    }
                });

                self.render_lava_lamp(ui, panel_rect);
            });
    }

    fn render_cue_pool_header(&mut self, ui: &mut egui::Ui) -> (Option<String>, bool, bool) {
        let mut result = (None, false, false);
        let heading_text = build_heading_text(&self.cues);
        let font_size = self.settings.font_size;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(heading_text)
                    .size(font_size * FONT_SCALE_SUBHEADING)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                result = render_cue_pool_buttons(ui, &self.settings.playbook);
            });
        });
        result
    }

    fn handle_playbook_selection(&mut self, selected_play_prompt: Option<String>) {
        let Some(prompt) = selected_play_prompt else {
            return;
        };
        let vars = settings::parse_play_variables(&prompt);
        if vars.is_empty() {
            let _ = self.db.insert_cue(&prompt, "", 0, None, &[]);
            self.reload_cues();
            return;
        }
        let mut auto_resolved = HashMap::new();
        let mut selected = Vec::new();
        let mut custom_text = Vec::new();
        for (i, var) in vars.iter().enumerate() {
            if var.name.eq_ignore_ascii_case("LICENSE") {
                let has_license = [
                    "LICENSE",
                    "LICENSE.md",
                    "LICENSE.txt",
                    "LICENCE",
                    "LICENCE.md",
                ]
                .iter()
                .any(|f| self.project_root.join(f).exists());
                if has_license {
                    auto_resolved.insert(i, "already present".to_string());
                }
            }
            selected.push(0);
            custom_text.push(String::new());
        }
        if auto_resolved.len() == vars.len() {
            let resolved: Vec<(String, String)> = vars
                .iter()
                .enumerate()
                .map(|(i, v)| (v.token.clone(), auto_resolved[&i].clone()))
                .collect();
            let final_prompt = settings::substitute_play_variables(&prompt, &resolved);
            let _ = self.db.insert_cue(&final_prompt, "", 0, None, &[]);
            self.reload_cues();
        } else {
            self.pending_play = Some(PendingPlay {
                prompt,
                variables: vars,
                selected,
                custom_text,
                auto_resolved,
            });
        }
    }

    fn handle_custom_cue_request(&mut self, requested: bool) {
        if requested {
            self.global_prompt_input.clear();
        }
    }

    fn handle_import_request(&mut self, import_requested: bool) {
        if !import_requested {
            return;
        }
        let Some(path) = pick_markdown_file(&self.project_root) else {
            return;
        };
        let Ok(content) = std::fs::read_to_string(&path) else {
            return;
        };
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("import")
            .to_string();
        let sections = parse_markdown_sections(&content);
        let (new_count, updated_count) = self.import_sections(&sections, &path, &stem);
        self.reload_cues();
        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("document");
        let msg = format_import_message(new_count, updated_count, filename);
        self.set_status_message(msg);
    }

    fn import_sections(
        &self,
        sections: &[markdown_import::ImportedSection],
        path: &std::path::Path,
        stem: &str,
    ) -> (usize, usize) {
        let mut new_count = 0usize;
        let mut updated_count = 0usize;
        for section in sections {
            let source_ref = format!("{}#{}", path.display(), section.number);
            let text = format!("{}\n\n{}", section.title, section.body);
            match self.db.cue_exists_by_source_ref(&source_ref) {
                Ok(true) => {
                    if self
                        .db
                        .update_cue_by_source_ref(&source_ref, &text, "", 0)
                        .is_ok()
                    {
                        updated_count += 1;
                    }
                }
                Ok(false) => {
                    if self
                        .db
                        .insert_cue_from_source(&text, stem, "", &source_ref, "", 0)
                        .is_ok()
                    {
                        new_count += 1;
                    }
                }
                Err(e) => {
                    eprintln!("Failed to check source ref {}: {}", source_ref, e);
                }
            }
        }
        (new_count, updated_count)
    }

    fn render_source_filter(&mut self, ui: &mut egui::Ui) {
        let unique_labels = collect_unique_labels(&self.cues, &self.settings.sources);
        if unique_labels.is_empty() {
            return;
        }
        ui.horizontal(|ui| {
            let current = self.sources.filter.as_deref().unwrap_or("All");
            egui::ComboBox::from_id_salt("source_filter")
                .selected_text(current)
                .width(ui.available_width() - 8.0)
                .show_ui(ui, |ui| {
                    let is_all = self.sources.filter.is_none();
                    if ui.selectable_label(is_all, "All").clicked() {
                        self.sources.filter = None;
                        self.cached_filtered_archived_count = self.archived_cue_count;
                    }
                    for label in &unique_labels {
                        let count = self
                            .cues
                            .iter()
                            .filter(|c| c.source_label.as_deref() == Some(label.as_str()))
                            .count();
                        let display = format!("{} ({})", label, count);
                        let selected = self.sources.filter.as_deref() == Some(label.as_str());
                        if ui.selectable_label(selected, &display).clicked() {
                            self.sources.filter = Some(label.clone());
                            self.cached_filtered_archived_count =
                                self.db.archived_cue_count_by_source(label).unwrap_or(0);
                        }
                    }
                });
        });
    }

    fn render_cue_section(
        &mut self,
        ui: &mut egui::Ui,
        status: CueStatus,
        section_cues: &[&Cue],
        actions: &mut Vec<(i64, CueAction)>,
        load_more_archived: &mut bool,
        filtered_archived_count: usize,
    ) {
        let header = build_section_header(status, section_cues.len(), filtered_archived_count);
        let header_rt = egui::RichText::new(header)
            .size(self.settings.font_size * FONT_SCALE_SUBHEADING)
            .strong();
        let mut collapsing = egui::CollapsingHeader::new(header_rt)
            .id_salt(status.label())
            .default_open(status == CueStatus::Inbox || status == CueStatus::Review);
        if status == CueStatus::Ready && self.claude.expand_running {
            collapsing = collapsing.open(Some(true));
        }
        collapsing.show(ui, |ui| {
            if section_cues.is_empty() {
                ui.label(
                    egui::RichText::new("(empty)")
                        .italics()
                        .color(self.semantic.tertiary_text),
                );
            }
            self.render_section_bulk_actions(ui, status, section_cues, actions);
            for cue in section_cues {
                self.render_cue_card(ui, cue, actions, status);
            }
            if status == CueStatus::Archived && filtered_archived_count > section_cues.len() {
                let remaining = filtered_archived_count - section_cues.len();
                let label = format!("Load more ({} remaining)", remaining);
                if ui.small_button(label).clicked() {
                    *load_more_archived = true;
                }
            }
        });
    }

    fn render_lava_lamp(&mut self, ui: &mut egui::Ui, panel_rect: egui::Rect) {
        if !self.settings.lava_lamp_enabled {
            return;
        }
        if !self.cues.iter().any(|c| c.status == CueStatus::Ready) {
            return;
        }
        let margin = 8.0;
        let scale = if self.lava_lamp_big { 3.0 } else { 1.0 };
        let (lamp_w, lamp_h) = super::lava_lamp::size(scale);
        let origin = egui::pos2(
            panel_rect.right() - lamp_w - margin,
            panel_rect.bottom() - lamp_h - margin,
        );
        let lamp_rect = egui::Rect::from_min_size(origin, egui::vec2(lamp_w, lamp_h));
        let resp = ui.allocate_rect(lamp_rect, egui::Sense::click());
        if resp.clicked() {
            self.lava_lamp_big = !self.lava_lamp_big;
        }
        super::lava_lamp::paint_at(
            ui.painter(),
            ui.ctx(),
            origin,
            self.semantic.accent,
            self.settings.theme.is_dark(),
            scale,
        );
    }
}
