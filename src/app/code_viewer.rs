use std::collections::HashSet;

use eframe::egui;

use super::{icon, DirigentApp, DiffReview};
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
        if self.show_running_log.is_some() {
            self.render_running_log_central(ctx);
            return;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.current_file.is_none() {
                // Ensure logo texture is loaded
                if self.logo_texture.is_none() {
                    let png_bytes = include_bytes!("../../assets/logo.png");
                    let img = image::load_from_memory_with_format(
                        png_bytes,
                        image::ImageFormat::Png,
                    )
                    .expect("failed to decode logo.png")
                    .into_rgba8();
                    let size = [img.width() as usize, img.height() as usize];
                    let pixels = img.into_raw();
                    let color_image =
                        egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
                    self.logo_texture = Some(ctx.load_texture(
                        "dirigent_logo",
                        color_image,
                        egui::TextureOptions::LINEAR,
                    ));
                }

                ui.vertical_centered(|ui| {
                    let available = ui.available_height();
                    ui.add_space(available * 0.3);
                    if let Some(ref tex) = self.logo_texture {
                        ui.add(
                            egui::Image::new(tex).max_size(egui::vec2(96.0, 96.0)),
                        );
                    }
                    ui.add_space(8.0);
                    ui.heading("Dirigent");
                    ui.label(format!("Version {}", env!("BUILD_VERSION")));
                    ui.add_space(16.0);
                    ui.label(
                        egui::RichText::new("Select a file from the tree to view")
                            .weak(),
                    );
                });
                return;
            }

            let file_path = self.current_file.clone().unwrap();
            let rel_path = self.relative_path(&file_path);

            let mut close_file = false;
            let mut show_file_diff = false;
            let is_dirty = self.dirty_files.contains(&rel_path);
            ui.horizontal(|ui| {
                ui.strong(&rel_path);
                if is_dirty {
                    ui.label(
                        egui::RichText::new("\u{25CF}")
                            .color(egui::Color32::from_rgb(200, 160, 50)),
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
                    ui.label(format!("{} lines", self.current_file_content.len()));
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
                self.current_file = None;
                self.current_file_content.clear();
                self.selection_start = None;
                self.selection_end = None;
                self.cue_input.clear();
                return;
            }
            if show_file_diff {
                let files = vec![rel_path.clone()];
                if let Some(diff_text) = git::get_working_diff(&self.project_root, &files) {
                    let parsed = diff_view::parse_unified_diff(&diff_text);
                    self.diff_review = Some(DiffReview {
                        cue_id: 0,
                        diff: diff_text,
                        cue_text: format!("Uncommitted changes: {}", rel_path),
                        parsed,
                        view_mode: DiffViewMode::Inline,
                        read_only: true,
                        collapsed_files: HashSet::new(),
                        prompt_expanded: false,
                    });
                }
                return;
            }
            ui.separator();

            // Render in-file search bar (Cmd+F)
            if self.search_in_file_active {
                self.render_search_in_file_bar(ui);
            }

            let lines_with_cues = self.lines_with_cues();
            let num_lines = self.current_file_content.len();
            let line_height = 16.0;

            let ext = file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            let sel_start = self.selection_start;
            let sel_end = self.selection_end;
            let mut new_sel_start = sel_start;
            let mut new_sel_end = sel_end;
            let mut submit_cue = false;
            let mut clear_selection = false;

            let mut scroll_area = egui::ScrollArea::vertical().auto_shrink([false; 2]);

            // Handle scroll-to-line requests (from search, cue navigation, etc.)
            if let Some(target_line) = self.scroll_to_line.take() {
                let target_offset = (target_line.saturating_sub(1)) as f32 * line_height;
                scroll_area = scroll_area.vertical_scroll_offset(target_offset);
            }

            scroll_area.show_rows(ui, line_height, num_lines, |ui, row_range| {
                for line_idx in row_range {
                    let line_num = line_idx + 1;
                    let line_text = self
                        .current_file_content
                        .get(line_idx)
                        .map(|s| s.as_str())
                        .unwrap_or("");

                    let is_in_selection = match (sel_start, sel_end) {
                        (Some(s), Some(e)) => line_num >= s && line_num <= e,
                        _ => false,
                    };
                    let is_selection_end = sel_end == Some(line_num);
                    let cue_state = lines_with_cues.get(&line_num);
                    let is_search_match = self.search_in_file_matches.contains(&line_num);
                    let is_current_search_match = self
                        .search_in_file_current
                        .map(|i| self.search_in_file_matches.get(i) == Some(&line_num))
                        .unwrap_or(false);

                    let response = ui.horizontal(|ui| {
                        match cue_state {
                            Some(&true) => {
                                // Archived cue: grey dot
                                ui.label(icon("\u{2022}", self.settings.font_size).color(egui::Color32::from_gray(140)));
                            }
                            Some(&false) => {
                                // Active cue: yellow dot
                                ui.label(icon("\u{2022}", self.settings.font_size).color(egui::Color32::from_rgb(255, 180, 50)));
                            }
                            None => {
                                ui.label(" ");
                            }
                        }

                        let num_text = format!("{:>4} ", line_num);
                        ui.label(
                            egui::RichText::new(num_text)
                                .monospace()
                                .color(egui::Color32::from_gray(100)),
                        );

                        let layout_job = egui_extras::syntax_highlighting::highlight(
                            ui.ctx(),
                            ui.style(),
                            &self.syntax_theme,
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
                                egui::Color32::from_rgba_premultiplied(60, 60, 120, 80),
                            );
                        }

                        if is_current_search_match {
                            // Current search match: bright orange highlight
                            ui.painter().rect_filled(
                                rect,
                                0.0,
                                egui::Color32::from_rgba_premultiplied(200, 120, 0, 60),
                            );
                        } else if is_search_match {
                            // Other search matches: subtle yellow highlight
                            ui.painter().rect_filled(
                                rect,
                                0.0,
                                egui::Color32::from_rgba_premultiplied(180, 160, 0, 35),
                            );
                        }

                        response
                    });

                    if response.inner.clicked() {
                        let shift_held = ui.input(|i| i.modifiers.shift);
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
                        ui.horizontal(|ui| {
                            ui.label("     ");
                            ui.label(
                                egui::RichText::new(range_label)
                                    .monospace()
                                    .color(egui::Color32::from_rgb(100, 200, 100)),
                            );
                            let input_response = ui.add(
                                egui::TextEdit::singleline(&mut self.cue_input)
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
            });

            if clear_selection {
                new_sel_start = None;
                new_sel_end = None;
            }

            if new_sel_start != self.selection_start || new_sel_end != self.selection_end {
                self.selection_start = new_sel_start;
                self.selection_end = new_sel_end;
                self.cue_input.clear();
            }

            if submit_cue && !self.cue_input.is_empty() {
                if let Some(start) = self.selection_start {
                    let end = self.selection_end.unwrap_or(start);
                    let line_end = if end > start { Some(end) } else { None };
                    let text = self.cue_input.clone();
                    let _ = self.db.insert_cue(&text, &rel_path, start, line_end);
                    self.cue_input.clear();
                    self.reload_cues();
                }
            }
        });
    }
}
