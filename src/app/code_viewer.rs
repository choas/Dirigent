use std::collections::HashSet;

use eframe::egui;

use super::{
    icon, DiffReview, DirigentApp, FONT_SCALE_HEADING, FONT_SCALE_LINE_NUM, SPACE_MD, SPACE_SM,
};
use crate::diff_view::{self, DiffViewMode};
use crate::git;

impl DirigentApp {
    pub(super) fn render_code_viewer(&mut self, ctx: &egui::Context) {
        if self.show_settings {
            self.render_settings_panel(ctx);
            return;
        }

        // Diff Review in central panel
        if self.diff_review.is_some() {
            self.render_diff_review_central(ctx);
            return;
        }

        // Claude Progress in central panel
        if self.claude.show_log.is_some() {
            self.render_running_log_central(ctx);
            return;
        }

        // Agent run log in central panel
        if self.agent_state.show_output.is_some() {
            self.render_agent_log_central(ctx);
            return;
        }

        // Per-cue agent runs in central panel
        if self.show_agent_runs_for_cue.is_some() {
            self.render_cue_agent_runs_central(ctx);
            return;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.viewer.current_file.is_none() {
                self.ensure_logo_texture(ctx);

                ui.vertical_centered(|ui| {
                    let available = ui.available_height();
                    ui.add_space(available * 0.3);
                    if let Some(ref tex) = self.logo_texture {
                        ui.add(
                            egui::Image::new(tex).max_size(egui::vec2(96.0, 96.0)),
                        );
                    }
                    ui.add_space(SPACE_SM);
                    ui.heading("Dirigent");
                    ui.label(format!("Version {}", env!("BUILD_VERSION")));
                    ui.add_space(SPACE_MD);
                    ui.label(
                        egui::RichText::new("Select a file from the tree to view")
                            .weak(),
                    );
                });
                return;
            }

            let file_path = self.viewer.current_file.clone().unwrap();
            let rel_path = self.relative_path(&file_path);

            let mut close_file = false;
            let mut show_file_diff = false;
            let is_dirty = self.git.dirty_files.contains_key(&rel_path);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&rel_path).size(self.settings.font_size * FONT_SCALE_HEADING).strong());
                if is_dirty {
                    ui.label(
                        egui::RichText::new("\u{25CF}")
                            .color(self.semantic.warning),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(icon("\u{2715}", self.settings.font_size))
                        .on_hover_text("Close file")
                        .clicked()
                    {
                        close_file = true;
                    }
                    ui.label(format!("{} lines", self.viewer.content.len()));
                    if is_dirty {
                        if ui
                            .small_button("Show Diff")
                            .on_hover_text("Show uncommitted changes for this file")
                            .clicked()
                        {
                            show_file_diff = true;
                        }
                    }
                });
            });
            if close_file {
                self.viewer.current_file = None;
                self.viewer.content.clear();
                self.viewer.selection_start = None;
                self.viewer.selection_end = None;
                self.viewer.cue_input.clear();
                return;
            }
            if show_file_diff {
                let files = vec![rel_path.clone()];
                if let Some(diff_text) = git::get_working_diff(&self.project_root, &files) {
                    let parsed = diff_view::parse_unified_diff(&diff_text);
                    self.dismiss_central_overlays();
                    self.diff_review = Some(DiffReview {
                        cue_id: 0,
                        diff: diff_text,
                        cue_text: format!("Uncommitted changes: {}", rel_path),
                        parsed,
                        view_mode: DiffViewMode::Inline,
                        read_only: true,
                        collapsed_files: HashSet::new(),
                        prompt_expanded: false,
                        reply_text: String::new(),
                        search_active: false,
                        search_query: String::new(),
                        search_matches: Vec::new(),
                        search_current: None,
                    });
                }
                return;
            }
            ui.separator();

            // Render in-file search bar (Cmd+F)
            if self.search.in_file_active {
                self.render_search_in_file_bar(ui);
            }

            let lines_with_cues = self.lines_with_cues();
            let num_lines = self.viewer.content.len();
            let line_height = 16.0;

            let ext = file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            let sel_start = self.viewer.selection_start;
            let sel_end = self.viewer.selection_end;
            let mut new_sel_start = sel_start;
            let mut new_sel_end = sel_end;
            let mut submit_cue = false;
            let mut clear_selection = false;

            // Handle scroll-to-line requests (from search, cue navigation, etc.)
            let scroll_offset = self.viewer.scroll_to_line.take().map(|target_line| {
                (target_line.saturating_sub(1)) as f32 * line_height
            });

            macro_rules! render_lines {
                ($ui:expr, $range:expr) => {
                    for line_idx in $range {
                        let line_num = line_idx + 1;
                        let line_text = self
                            .viewer.content
                            .get(line_idx)
                            .map(|s| s.as_str())
                            .unwrap_or("");

                        let is_in_selection = match (sel_start, sel_end) {
                            (Some(s), Some(e)) => line_num >= s && line_num <= e,
                            _ => false,
                        };
                        let is_selection_end = sel_end == Some(line_num);
                        let cue_state = lines_with_cues.get(&line_num);
                        let is_search_match = self.search.in_file_matches.contains(&line_num);
                        let is_current_search_match = self
                            .search.in_file_current
                            .map(|i| self.search.in_file_matches.get(i) == Some(&line_num))
                            .unwrap_or(false);

                        let response = $ui.horizontal(|ui| {
                            match cue_state {
                                Some(&true) => {
                                    // Archived cue: grey dot
                                    ui.label(icon("\u{25CF}", self.settings.font_size).color(self.semantic.secondary_text));
                                }
                                Some(&false) => {
                                    // Active cue: yellow dot
                                    ui.label(icon("\u{25CF}", self.settings.font_size).color(self.semantic.warning));
                                }
                                None => {
                                    ui.label(" ");
                                }
                            }

                            let num_text = format!("{:>4} ", line_num);
                            ui.label(
                                egui::RichText::new(num_text)
                                    .monospace()
                                    .size(self.settings.font_size * FONT_SCALE_LINE_NUM)
                                    .color(self.semantic.tertiary_text),
                            );

                            let layout_job = egui_extras::syntax_highlighting::highlight(
                                ui.ctx(),
                                ui.style(),
                                &self.viewer.syntax_theme,
                                line_text,
                                ext,
                            );
                            let response = ui.label(layout_job);

                            let rect = response.rect.union(ui.available_rect_before_wrap());
                            let response = ui.interact(
                                rect,
                                egui::Id::new(("code_line", line_idx)),
                                egui::Sense::click(),
                            );

                            if is_in_selection {
                                ui.painter().rect_filled(
                                    rect,
                                    0.0,
                                    self.semantic.selection_bg(),
                                );
                            }

                            if is_current_search_match {
                                ui.painter().rect_filled(
                                    rect,
                                    0.0,
                                    self.semantic.code_search_current(),
                                );
                                // Flash brighter when navigating between matches
                                if let Some(when) = self.search.in_file_nav_flash {
                                    let elapsed = when.elapsed().as_secs_f32();
                                    if elapsed < 0.4 {
                                        let alpha = ((0.4 - elapsed) / 0.4 * 100.0) as u8;
                                        ui.painter().rect_filled(
                                            rect,
                                            0.0,
                                            egui::Color32::from_rgba_premultiplied(255, 200, 50, alpha),
                                        );
                                        ui.ctx().request_repaint();
                                    }
                                }
                            } else if is_search_match {
                                ui.painter().rect_filled(
                                    rect,
                                    0.0,
                                    self.semantic.code_search_match(),
                                );
                            }

                            response
                        });

                        if response.inner.clicked() {
                            let shift_held = $ui.input(|i| i.modifiers.shift);
                            if shift_held {
                                // Shift-click: extend selection from existing start (or set new range)
                                if let Some(anchor) = sel_start {
                                    let lo = anchor.min(line_num);
                                    let hi = anchor.max(line_num);
                                    new_sel_start = Some(lo);
                                    new_sel_end = Some(hi);
                                } else {
                                    new_sel_start = Some(line_num);
                                    new_sel_end = Some(line_num);
                                }
                            } else {
                                // Plain click: select single line
                                new_sel_start = Some(line_num);
                                new_sel_end = Some(line_num);
                            }
                        }

                        // Show cue input after the last line of the selection
                        if is_selection_end {
                            let range_label = if sel_start == sel_end {
                                format!("L{}", sel_start.unwrap_or(0))
                            } else {
                                format!(
                                    "L{}-{}",
                                    sel_start.unwrap_or(0),
                                    sel_end.unwrap_or(0)
                                )
                            };
                            // Show attached images for cue input
                            if !self.viewer.cue_images.is_empty() {
                                $ui.horizontal_wrapped(|ui| {
                                    ui.label("     ");
                                    ui.label(
                                        egui::RichText::new("Images:")
                                            .small()
                                            .color(self.semantic.accent),
                                    );
                                    let mut remove_idx = None;
                                    for (i, path) in self.viewer.cue_images.iter().enumerate() {
                                        let name = path
                                            .file_name()
                                            .map(|n| n.to_string_lossy().to_string())
                                            .unwrap_or_else(|| path.to_string_lossy().to_string());
                                        ui.label(egui::RichText::new(&name).monospace().small());
                                        if ui.small_button("\u{2715}").clicked() {
                                            remove_idx = Some(i);
                                        }
                                    }
                                    if let Some(i) = remove_idx {
                                        self.viewer.cue_images.remove(i);
                                    }
                                });
                            }
                            $ui.horizontal(|ui| {
                                ui.label("     ");
                                ui.label(
                                    egui::RichText::new(range_label)
                                        .monospace()
                                        .color(self.semantic.success),
                                );
                                if ui
                                    .button("+")
                                    .on_hover_text("Attach images")
                                    .clicked()
                                {
                                    if let Some(paths) = rfd::FileDialog::new()
                                        .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "bmp"])
                                        .pick_files()
                                    {
                                        self.viewer.cue_images.extend(paths);
                                    }
                                }
                                let input_response = ui.add(
                                    egui::TextEdit::singleline(&mut self.viewer.cue_input)
                                        .desired_width(ui.available_width() - 80.0)
                                        .hint_text("Add a cue...")
                                        .font(egui::TextStyle::Monospace),
                                );
                                if ui.button("Add").clicked()
                                    || (input_response.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                                {
                                    submit_cue = true;
                                }
                                if ui.button(icon("\u{2715}", self.settings.font_size)).clicked()
                                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                                {
                                    clear_selection = true;
                                }
                            });
                        }
                    }
                };
            }

            {
                // Source code: horizontal scrollbar, virtualized rows
                let mut scroll_area = egui::ScrollArea::both().auto_shrink([false; 2]);
                if let Some(offset) = scroll_offset {
                    scroll_area = scroll_area.vertical_scroll_offset(offset);
                }
                scroll_area.show_rows(ui, line_height, num_lines, |ui, row_range| {
                    render_lines!(ui, row_range);
                });
            }

            if clear_selection {
                new_sel_start = None;
                new_sel_end = None;
            }

            if new_sel_start != self.viewer.selection_start || new_sel_end != self.viewer.selection_end {
                self.viewer.selection_start = new_sel_start;
                self.viewer.selection_end = new_sel_end;
                self.viewer.cue_input.clear();
                self.viewer.cue_images.clear();
            }

            if submit_cue && !self.viewer.cue_input.is_empty() {
                if let Some(start) = self.viewer.selection_start {
                    let end = self.viewer.selection_end.unwrap_or(start);
                    let line_end = if end > start { Some(end) } else { None };
                    let text = self.viewer.cue_input.clone();
                    let images: Vec<String> = self
                        .viewer
                        .cue_images
                        .drain(..)
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();
                    let _ = self.db.insert_cue(&text, &rel_path, start, line_end, &images);
                    self.viewer.cue_input.clear();
                    self.reload_cues();
                }
            }
        });
    }
}
