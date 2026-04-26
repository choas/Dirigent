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

    /// Render the quick-open header row with title, gitignore toggle, and close button.
    fn render_quick_open_header(&mut self, ui: &mut egui::Ui, fs: f32) {
        ui.horizontal(|ui| {
            ui.add_space(SPACE_MD);
            ui.label(egui::RichText::new("Open File").strong());
            ui.checkbox(
                &mut self.viewer.quick_open_show_ignored,
                "Show .gitignore'd",
            );
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
        let show_ignored = self.viewer.quick_open_show_ignored;
        let mut matches: Vec<(String, PathBuf)> = Vec::new();
        let tree = match self.file_tree {
            Some(ref t) => t,
            None => return matches,
        };
        let mut all_files = Vec::new();
        collect_file_paths(&tree.entries, &self.project_root, &mut all_files);
        for (rel, abs, is_ignored) in all_files {
            if !show_ignored && is_ignored {
                continue;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_match_exact() {
        assert!(fuzzy_match("hello", "hello"));
    }

    #[test]
    fn fuzzy_match_subsequence() {
        assert!(fuzzy_match("src/app/mod.rs", "sam"));
        assert!(fuzzy_match("hello_world", "hwd"));
    }

    #[test]
    fn fuzzy_match_no_match() {
        assert!(!fuzzy_match("abc", "z"));
        assert!(!fuzzy_match("hello", "hx"));
    }

    #[test]
    fn fuzzy_match_empty_needle() {
        assert!(fuzzy_match("anything", ""));
        assert!(fuzzy_match("", ""));
    }

    #[test]
    fn fuzzy_match_empty_haystack() {
        assert!(!fuzzy_match("", "a"));
    }

    #[test]
    fn fuzzy_match_needle_longer_than_haystack() {
        assert!(!fuzzy_match("ab", "abc"));
    }

    #[test]
    fn fuzzy_match_case_sensitive() {
        assert!(!fuzzy_match("Hello", "hello"));
        assert!(fuzzy_match("Hello", "Hlo"));
    }

    #[test]
    fn fuzzy_match_non_ascii() {
        assert!(fuzzy_match("café_latté", "clt"));
        assert!(fuzzy_match("日本語テスト", "本テ"));
        assert!(!fuzzy_match("日本語", "英"));
    }

    #[test]
    fn fuzzy_match_unicode_emoji() {
        assert!(fuzzy_match("src/🚀/main.rs", "🚀"));
        assert!(fuzzy_match("file_✅_done", "f✅d"));
    }

    #[test]
    fn fuzzy_match_repeated_chars() {
        assert!(fuzzy_match("aabbc", "abc"));
        assert!(!fuzzy_match("aab", "bba"));
    }
}
