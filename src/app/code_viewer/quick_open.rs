use std::path::PathBuf;

use eframe::egui;

use super::goto_definition::collect_file_paths;
use crate::app::{icon, DirigentApp, SPACE_MD};

impl DirigentApp {
    /// Quick-open file overlay (Cmd+P).
    pub(in crate::app) fn render_quick_open_overlay(&mut self, ui: &mut egui::Ui) {
        let fs = self.settings.font_size;
        ui.vertical(|ui| {
            ui.add_space(SPACE_MD);
            self.render_quick_open_header(ui, fs);
            self.render_quick_open_input(ui);
            ui.separator();

            let matches = self.collect_quick_open_matches();
            self.clamp_quick_open_selection(matches.len());
            self.handle_quick_open_keys(ui, matches.len());

            let navigate_to = self.render_quick_open_results(ui, &matches);
            if let Some(path) = navigate_to {
                self.push_nav_history();
                self.load_file(path);
                self.viewer.quick_open_active = false;
            }
        });
    }

    /// Render the quick-open header row with title and close button.
    fn render_quick_open_header(&mut self, ui: &mut egui::Ui, fs: f32) {
        ui.horizontal(|ui| {
            ui.add_space(SPACE_MD);
            ui.label(egui::RichText::new("Open File").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(SPACE_MD);
                if ui.small_button(icon("\u{2715}", fs)).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    self.viewer.quick_open_active = false;
                }
            });
        });
    }

    /// Render the quick-open search text input.
    fn render_quick_open_input(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_space(SPACE_MD);
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.viewer.quick_open_query)
                    .desired_width(ui.available_width() - SPACE_MD * 2.0)
                    .hint_text("Type to search files...")
                    .font(egui::TextStyle::Monospace),
            );
            resp.request_focus();
            if resp.changed() {
                self.viewer.quick_open_selected = 0;
            }
        });
    }

    /// Collect matching files for the quick-open query.
    fn collect_quick_open_matches(&self) -> Vec<(String, PathBuf)> {
        let query = self.viewer.quick_open_query.to_lowercase();
        let mut matches: Vec<(String, PathBuf)> = Vec::new();
        let tree = match self.file_tree {
            Some(ref t) => t,
            None => return matches,
        };
        let mut all_files = Vec::new();
        collect_file_paths(&tree.entries, &self.project_root, &mut all_files);
        for (rel, abs) in all_files {
            if crate::app::search::is_binary_ext(&abs) {
                continue;
            }
            let is_match = query.is_empty() || fuzzy_match(&rel.to_lowercase(), &query);
            if is_match {
                matches.push((rel, abs));
            }
            if matches.len() >= 30 {
                break;
            }
        }
        matches
    }

    /// Clamp the quick-open selection to the current match count.
    fn clamp_quick_open_selection(&mut self, count: usize) {
        if count > 0 {
            self.viewer.quick_open_selected = self.viewer.quick_open_selected.min(count - 1);
        } else {
            self.viewer.quick_open_selected = 0;
        }
    }

    /// Handle arrow key navigation for quick-open.
    fn handle_quick_open_keys(&mut self, ui: &mut egui::Ui, count: usize) {
        let (arrow_up, arrow_down) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
            )
        });
        if arrow_up && count > 0 {
            if self.viewer.quick_open_selected > 0 {
                self.viewer.quick_open_selected -= 1;
            } else {
                self.viewer.quick_open_selected = count - 1;
            }
        }
        if arrow_down && count > 0 {
            if self.viewer.quick_open_selected + 1 < count {
                self.viewer.quick_open_selected += 1;
            } else {
                self.viewer.quick_open_selected = 0;
            }
        }
    }

    /// Render the quick-open results list and return which file to navigate to (if any).
    fn render_quick_open_results(
        &self,
        ui: &mut egui::Ui,
        matches: &[(String, PathBuf)],
    ) -> Option<PathBuf> {
        let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let sel = self.viewer.quick_open_selected;
        let query = &self.viewer.quick_open_query;
        let mut navigate_to: Option<PathBuf> = None;

        egui::ScrollArea::vertical()
            .id_salt("quick_open_scroll")
            .max_height(ui.available_height() - SPACE_MD)
            .show(ui, |ui| {
                for (i, (rel, abs)) in matches.iter().enumerate() {
                    let is_selected = i == sel;
                    let clicked = ui
                        .selectable_label(is_selected, egui::RichText::new(rel).monospace().small())
                        .clicked();
                    if clicked || (is_selected && enter_pressed) {
                        navigate_to = Some(abs.clone());
                    }
                }
                if matches.is_empty() && !query.is_empty() {
                    ui.label(
                        egui::RichText::new("No files found")
                            .italics()
                            .color(self.semantic.tertiary_text),
                    );
                }
            });

        navigate_to
    }
}

/// Simple fuzzy (subsequence) matching.
pub(crate) fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    let mut needle_chars = needle.chars();
    let mut current = needle_chars.next();
    for ch in haystack.chars() {
        if let Some(n) = current {
            if ch == n {
                current = needle_chars.next();
            }
        } else {
            return true;
        }
    }
    current.is_none()
}
