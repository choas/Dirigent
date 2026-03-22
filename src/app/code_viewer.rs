use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use eframe::egui;

use super::{
    icon, symbols, DiffReview, DirigentApp, FONT_SCALE_LINE_NUM, FONT_SCALE_SMALL,
    FONT_SCALE_SUBHEADING, SPACE_MD, SPACE_SM,
};
use crate::agents::Severity;
use crate::diff_view::{self, DiffViewMode};
use crate::file_tree::FileEntry;
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
            // Quick-open overlay (Cmd+P)
            if self.viewer.quick_open_active {
                self.render_quick_open_overlay(ui);
                return;
            }

            let active_idx = match self.viewer.active_tab {
                Some(idx) if idx < self.viewer.tabs.len() => idx,
                _ => {
                    // No file open — show splash
                    // But still show tab bar if tabs exist (shouldn't happen, but defensive)
                    self.ensure_logo_texture(ctx);
                    ui.vertical_centered(|ui| {
                        let available = ui.available_height();
                        ui.add_space(available * 0.3);
                        if let Some(ref tex) = self.logo_texture {
                            ui.add(egui::Image::new(tex).max_size(egui::vec2(96.0, 96.0)));
                        }
                        ui.add_space(SPACE_SM);
                        ui.heading("Dirigent");
                        ui.label(format!("Version {}", env!("BUILD_VERSION")));
                        ui.add_space(SPACE_MD);
                        ui.label(
                            egui::RichText::new("Select a file from the tree to view  \u{2022}  \u{2318}P to quick open")
                                .weak(),
                        );
                    });
                    return;
                }
            };

            // -- Tab bar --
            if self.viewer.tabs.len() > 1 {
                let mut tab_to_close: Option<usize> = None;
                let mut tab_to_activate: Option<usize> = None;

                ui.horizontal(|ui| {
                    for (i, tab) in self.viewer.tabs.iter().enumerate() {
                        let is_active = i == active_idx;
                        let filename = tab
                            .file_path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "untitled".to_string());

                        let text = if is_active {
                            egui::RichText::new(&filename)
                                .small()
                                .strong()
                                .color(self.semantic.accent)
                        } else {
                            egui::RichText::new(&filename)
                                .small()
                                .color(self.semantic.secondary_text)
                        };

                        let frame = if is_active {
                            egui::Frame::NONE
                                .inner_margin(egui::Margin::symmetric(6, 3))
                                .fill(self.semantic.selection_bg())
                                .corner_radius(3)
                        } else {
                            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 3))
                        };

                        frame.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let rel = tab
                                    .file_path
                                    .strip_prefix(&self.project_root)
                                    .unwrap_or(&tab.file_path)
                                    .to_string_lossy()
                                    .to_string();
                                if ui
                                    .add(egui::Label::new(text).sense(egui::Sense::click()))
                                    .on_hover_text(&rel)
                                    .clicked()
                                {
                                    tab_to_activate = Some(i);
                                }
                                if ui
                                    .add(
                                        egui::Label::new(
                                            egui::RichText::new("\u{00D7}")
                                                .small()
                                                .color(self.semantic.tertiary_text),
                                        )
                                        .sense(egui::Sense::click()),
                                    )
                                    .on_hover_text("Close tab")
                                    .clicked()
                                {
                                    tab_to_close = Some(i);
                                }
                            });
                        });
                    }
                });

                if let Some(idx) = tab_to_close {
                    self.viewer.close_tab(idx);
                    return;
                }
                if let Some(idx) = tab_to_activate {
                    self.viewer.active_tab = Some(idx);
                    // Reset search when switching tabs
                    self.search.in_file_active = false;
                    self.search.in_file_query.clear();
                    self.search.in_file_matches.clear();
                    self.search.in_file_current = None;
                    return;
                }

                // Thin separator below tab bar
                ui.separator();
            }

            // Re-check active_idx after potential close
            let active_idx = match self.viewer.active_tab {
                Some(idx) if idx < self.viewer.tabs.len() => idx,
                _ => return,
            };

            let file_path = self.viewer.tabs[active_idx].file_path.clone();
            let rel_path = self.relative_path(&file_path);

            let mut close_file = false;
            let mut show_file_diff = false;
            let mut toggle_markdown = false;
            let is_dirty = self.git.dirty_files.contains_key(&rel_path);
            let is_markdown = self.viewer.tabs[active_idx].markdown_blocks.is_some();

            // -- Breadcrumb navigation bar --
            ui.horizontal(|ui| {
                // Split path into segments
                let segments: Vec<&str> = rel_path.split('/').collect();
                for (i, segment) in segments.iter().enumerate() {
                    if i > 0 {
                        ui.label(
                            egui::RichText::new("\u{203A}")
                                .color(self.semantic.tertiary_text),
                        );
                    }
                    let is_last = i == segments.len() - 1;
                    let text = if is_last {
                        egui::RichText::new(*segment)
                            .size(self.settings.font_size * FONT_SCALE_SUBHEADING)
                            .strong()
                    } else {
                        egui::RichText::new(*segment)
                            .size(self.settings.font_size * FONT_SCALE_SMALL)
                            .color(self.semantic.secondary_text)
                    };
                    if ui
                        .add(egui::Label::new(text).sense(egui::Sense::click()))
                        .on_hover_text(if is_last {
                            "Scroll to top".to_string()
                        } else {
                            format!("Expand directory {}", segments[..=i].join("/"))
                        })
                        .clicked()
                    {
                        if is_last {
                            self.viewer.scroll_to_line = Some(1);
                        } else {
                            // Expand this directory in the file tree
                            let dir_path = self.project_root.join(segments[..=i].join("/"));
                            self.expanded_dirs.insert(dir_path);
                        }
                    }
                }

                // Show current symbol in breadcrumb (from scroll position)
                // Use selection or an estimated visible line
                let current_line = self.viewer.tabs[active_idx]
                    .selection_start
                    .unwrap_or_else(|| {
                        let offset = self.viewer.tabs[active_idx].scroll_offset;
                        if offset > 0.0 {
                            (offset / 16.0) as usize + 1
                        } else {
                            1
                        }
                    });
                if let Some(sym) =
                    symbols::enclosing_symbol(&self.viewer.tabs[active_idx].symbols, current_line)
                {
                    ui.label(
                        egui::RichText::new("\u{203A}")
                            .color(self.semantic.tertiary_text),
                    );
                    ui.label(
                        egui::RichText::new(format!("{} {}", sym.kind.label(), sym.name))
                            .small()
                            .color(self.semantic.accent),
                    );
                }

                // Copy path button
                if ui
                    .small_button(icon("\u{1F4CB}", self.settings.font_size))
                    .on_hover_text("Copy file path")
                    .clicked()
                {
                    ui.ctx().copy_text(rel_path.clone());
                }
                if is_dirty {
                    ui.label(
                        egui::RichText::new("\u{25CF}")
                            .color(self.semantic.warning),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(icon("\u{2715}", self.settings.font_size))
                        .on_hover_text("Close tab (\u{2318}W)")
                        .clicked()
                    {
                        close_file = true;
                    }
                    // Show diagnostic count if any
                    {
                        let mut err_count = 0usize;
                        let mut warn_count = 0usize;
                        for diags in self.agent_state.latest_diagnostics.values() {
                            for d in diags {
                                let (rel, diag_p) =
                                    (Path::new(&rel_path), Path::new(&d.file));
                                if diag_p == rel
                                    || rel.ends_with(diag_p)
                                    || diag_p.ends_with(rel)
                                {
                                    match d.severity {
                                        Severity::Error => err_count += 1,
                                        Severity::Warning => warn_count += 1,
                                        _ => {}
                                    }
                                }
                            }
                        }
                        if err_count > 0 {
                            ui.label(
                                egui::RichText::new(format!("{} errors", err_count))
                                    .small()
                                    .color(self.semantic.danger),
                            );
                        }
                        if warn_count > 0 {
                            ui.label(
                                egui::RichText::new(format!("{} warnings", warn_count))
                                    .small()
                                    .color(self.semantic.warning),
                            );
                        }
                    }
                    ui.label(format!(
                        "{} lines",
                        self.viewer.tabs[active_idx].content.len()
                    ));
                    // Markdown Raw/Rendered toggle
                    if is_markdown {
                        let label = if self.viewer.tabs[active_idx].markdown_rendered {
                            "Raw"
                        } else {
                            "Rendered"
                        };
                        if ui
                            .small_button(label)
                            .on_hover_text("Toggle between raw Markdown and rendered view")
                            .clicked()
                        {
                            toggle_markdown = true;
                        }
                    }
                    if is_dirty
                        && ui
                            .small_button("Show Diff")
                            .on_hover_text("Show uncommitted changes for this file")
                            .clicked()
                    {
                        show_file_diff = true;
                    }
                });
            });
            if toggle_markdown {
                self.viewer.tabs[active_idx].markdown_rendered =
                    !self.viewer.tabs[active_idx].markdown_rendered;
            }
            if close_file {
                self.viewer.close_active_tab();
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

            // Render Markdown view if applicable
            if is_markdown && self.viewer.tabs[active_idx].markdown_rendered {
                egui::ScrollArea::both()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.add_space(SPACE_SM);
                        self.render_markdown_content(ui);
                        ui.add_space(SPACE_MD);
                    });
                return;
            }

            // Render in-file search bar (Cmd+F)
            if self.search.in_file_active {
                self.render_search_in_file_bar(ui);
            }

            let lines_with_cues = self.lines_with_cues();
            let num_lines = self.viewer.tabs[active_idx].content.len();
            let line_height = 16.0;

            // Build diagnostic lookup: line_num -> worst severity for this file
            let diag_lines: HashMap<usize, Severity> = {
                let mut map: HashMap<usize, Severity> = HashMap::new();
                for diags in self.agent_state.latest_diagnostics.values() {
                    for d in diags {
                        let (rel, diag_p) =
                            (Path::new(&rel_path), Path::new(&d.file));
                        if diag_p == rel
                            || rel.ends_with(diag_p)
                            || diag_p.ends_with(rel)
                        {
                            let entry = map.entry(d.line).or_insert(Severity::Info);
                            match (&entry, &d.severity) {
                                (Severity::Info, Severity::Warning | Severity::Error) => {
                                    *entry = d.severity
                                }
                                (Severity::Warning, Severity::Error) => *entry = d.severity,
                                _ => {}
                            }
                        }
                    }
                }
                map
            };
            // Collect diagnostic messages for tooltip per line
            let diag_messages: HashMap<usize, Vec<String>> = {
                let mut map: HashMap<usize, Vec<String>> = HashMap::new();
                for diags in self.agent_state.latest_diagnostics.values() {
                    for d in diags {
                        let (rel, diag_p) =
                            (Path::new(&rel_path), Path::new(&d.file));
                        if diag_p == rel
                            || rel.ends_with(diag_p)
                            || diag_p.ends_with(rel)
                        {
                            map.entry(d.line).or_default().push(d.message.clone());
                        }
                    }
                }
                map
            };

            let ext = file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            let sel_start = self.viewer.tabs[active_idx].selection_start;
            let sel_end = self.viewer.tabs[active_idx].selection_end;
            let mut new_sel_start = sel_start;
            let mut new_sel_end = sel_end;
            let mut submit_cue = false;
            let mut clear_selection = false;
            let mut fix_diagnostic_line: Option<usize> = None;
            let mut goto_def_word: Option<String> = None;

            // Handle scroll-to-line requests (from search, cue navigation, etc.)
            let scroll_offset = self.viewer.scroll_to_line.take().map(|target_line| {
                (target_line.saturating_sub(1)) as f32 * line_height
            });

            // Check if Cmd is held (for go-to-definition)
            let cmd_held = ui.input(|i| i.modifiers.command);

            macro_rules! render_lines {
                ($ui:expr, $range:expr) => {
                    for line_idx in $range {
                        let line_num = line_idx + 1;
                        let line_text = self
                            .viewer
                            .tabs[active_idx]
                            .content
                            .get(line_idx)
                            .map(|s| s.as_str())
                            .unwrap_or("");

                        let is_in_selection = match (sel_start, sel_end) {
                            (Some(s), Some(e)) => line_num >= s && line_num <= e,
                            _ => false,
                        };
                        let is_selection_end = sel_end == Some(line_num);
                        let cue_state = lines_with_cues.get(&line_num);
                        let is_search_match =
                            self.search.in_file_matches.contains(&line_num);
                        let is_current_search_match = self
                            .search
                            .in_file_current
                            .map(|i| self.search.in_file_matches.get(i) == Some(&line_num))
                            .unwrap_or(false);

                        let response = $ui.horizontal(|ui| {
                            match cue_state {
                                Some(&true) => {
                                    ui.label(
                                        icon("\u{25CF}", self.settings.font_size)
                                            .color(self.semantic.secondary_text),
                                    );
                                }
                                Some(&false) => {
                                    ui.label(
                                        icon("\u{25CF}", self.settings.font_size)
                                            .color(self.semantic.warning),
                                    );
                                }
                                None => {
                                    if let Some(sev) = diag_lines.get(&line_num) {
                                        let (sym, color) = match sev {
                                            Severity::Error => {
                                                ("\u{25CF}", self.semantic.danger)
                                            }
                                            Severity::Warning => {
                                                ("\u{25CF}", self.semantic.warning)
                                            }
                                            Severity::Info => {
                                                ("\u{25CB}", self.semantic.accent)
                                            }
                                        };
                                        let resp = ui.add(
                                            egui::Label::new(
                                                icon(sym, self.settings.font_size).color(color),
                                            )
                                            .sense(egui::Sense::click()),
                                        );
                                        if let Some(msgs) = diag_messages.get(&line_num) {
                                            let tooltip = format!(
                                                "{}\n\nClick to create a Fix cue",
                                                msgs.join("\n")
                                            );
                                            resp.clone().on_hover_text(tooltip);
                                        }
                                        if resp.clicked() {
                                            fix_diagnostic_line = Some(line_num);
                                        }
                                    } else {
                                        ui.label(" ");
                                    }
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
                            let code_resp = ui.label(layout_job);

                            let rect = code_resp.rect.union(ui.available_rect_before_wrap());
                            let response = ui.interact(
                                rect,
                                egui::Id::new(("code_line", line_idx)),
                                egui::Sense::click(),
                            );

                            // Show underline hint when Cmd is held (go-to-definition)
                            if cmd_held && response.hovered() {
                                ui.painter().hline(
                                    code_resp.rect.x_range(),
                                    code_resp.rect.bottom(),
                                    egui::Stroke::new(1.0, self.semantic.accent),
                                );
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }

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
                                if let Some(when) = self.search.in_file_nav_flash {
                                    let elapsed = when.elapsed().as_secs_f32();
                                    if elapsed < 0.4 {
                                        let alpha = ((0.4 - elapsed) / 0.4 * 100.0) as u8;
                                        ui.painter().rect_filled(
                                            rect,
                                            0.0,
                                            egui::Color32::from_rgba_premultiplied(
                                                255, 200, 50, alpha,
                                            ),
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

                            (response, code_resp)
                        });

                        let (line_response, code_resp) = response.inner;

                        if line_response.clicked() {
                            let shift_held = $ui.input(|i| i.modifiers.shift);
                            let cmd = $ui.input(|i| i.modifiers.command);

                            if cmd && !shift_held {
                                // Cmd+click: Go to definition
                                // Walk characters to find the byte offset under the pointer
                                if let Some(pos) = $ui.ctx().pointer_latest_pos() {
                                    let x_offset = (pos.x - code_resp.rect.left()).max(0.0);
                                    let approx_char_width = self.settings.font_size * 0.6;
                                    let mut accumulated = 0.0_f32;
                                    let mut byte_offset = line_text.len(); // fallback: past end
                                    for (idx, ch) in line_text.char_indices() {
                                        let ch_width = if ch == '\t' {
                                            approx_char_width * 4.0
                                        } else {
                                            approx_char_width * (ch.len_utf8().max(1) as f32).min(2.0)
                                        };
                                        if accumulated + ch_width > x_offset {
                                            byte_offset = idx;
                                            break;
                                        }
                                        accumulated += ch_width;
                                    }
                                    if let Some(word) =
                                        symbols::word_at_offset(line_text, byte_offset)
                                    {
                                        goto_def_word = Some(word.to_string());
                                    }
                                }
                            } else if shift_held {
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
                            if !self.viewer.tabs[active_idx].cue_images.is_empty() {
                                $ui.horizontal_wrapped(|ui| {
                                    ui.label("     ");
                                    ui.label(
                                        egui::RichText::new("Images:")
                                            .small()
                                            .color(self.semantic.accent),
                                    );
                                    let mut remove_idx = None;
                                    for (i, path) in self.viewer.tabs[active_idx]
                                        .cue_images
                                        .iter()
                                        .enumerate()
                                    {
                                        let name = path
                                            .file_name()
                                            .map(|n| n.to_string_lossy().to_string())
                                            .unwrap_or_else(|| {
                                                path.to_string_lossy().to_string()
                                            });
                                        ui.label(
                                            egui::RichText::new(&name).monospace().small(),
                                        );
                                        if ui.small_button("\u{2715}").clicked() {
                                            remove_idx = Some(i);
                                        }
                                    }
                                    if let Some(i) = remove_idx {
                                        self.viewer.tabs[active_idx].cue_images.remove(i);
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
                                        .add_filter(
                                            "Images",
                                            &["png", "jpg", "jpeg", "gif", "webp", "bmp"],
                                        )
                                        .pick_files()
                                    {
                                        self.viewer.tabs[active_idx]
                                            .cue_images
                                            .extend(paths);
                                    }
                                }
                                let input_response = ui.add(
                                    egui::TextEdit::singleline(
                                        &mut self.viewer.tabs[active_idx].cue_input,
                                    )
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
                                if ui
                                    .button(icon("\u{2715}", self.settings.font_size))
                                    .clicked()
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
                } else if self.viewer.tabs[active_idx].scroll_offset > 0.0 {
                    scroll_area = scroll_area.vertical_scroll_offset(
                        self.viewer.tabs[active_idx].scroll_offset,
                    );
                }
                let output = scroll_area.show_rows(ui, line_height, num_lines, |ui, row_range| {
                    render_lines!(ui, row_range);
                });
                // Save current scroll position so switching tabs preserves it.
                self.viewer.tabs[active_idx].scroll_offset = output.state.offset.y;
            }

            if clear_selection {
                new_sel_start = None;
                new_sel_end = None;
            }

            // Handle diagnostic "Fix" click
            if let Some(line) = fix_diagnostic_line {
                if let Some(msgs) = diag_messages.get(&line) {
                    let fix_text = format!("Fix: {}", msgs.join("; "));
                    new_sel_start = Some(line);
                    new_sel_end = Some(line);
                    self.viewer.tabs[active_idx].selection_start = Some(line);
                    self.viewer.tabs[active_idx].selection_end = Some(line);
                    self.viewer.tabs[active_idx].cue_input = fix_text;
                    self.viewer.tabs[active_idx].cue_images.clear();
                }
            }

            if new_sel_start != self.viewer.tabs[active_idx].selection_start
                || new_sel_end != self.viewer.tabs[active_idx].selection_end
            {
                self.viewer.tabs[active_idx].selection_start = new_sel_start;
                self.viewer.tabs[active_idx].selection_end = new_sel_end;
                self.viewer.tabs[active_idx].cue_input.clear();
                self.viewer.tabs[active_idx].cue_images.clear();
            }

            if submit_cue && !self.viewer.tabs[active_idx].cue_input.is_empty() {
                if let Some(start) = self.viewer.tabs[active_idx].selection_start {
                    let end = self.viewer.tabs[active_idx]
                        .selection_end
                        .unwrap_or(start);
                    let line_end = if end > start { Some(end) } else { None };
                    let text = self.viewer.tabs[active_idx].cue_input.clone();
                    let images: Vec<String> = self.viewer.tabs[active_idx]
                        .cue_images
                        .drain(..)
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();
                    let _ =
                        self.db
                            .insert_cue(&text, &rel_path, start, line_end, &images);
                    self.viewer.tabs[active_idx].cue_input.clear();
                    self.reload_cues();
                }
            }

            // Handle go-to-definition (deferred to avoid borrow issues)
            if let Some(word) = goto_def_word {
                self.goto_definition(&word);
            }
        });
    }

    /// Quick-open file overlay (Cmd+P).
    fn render_quick_open_overlay(&mut self, ui: &mut egui::Ui) {
        let fs = self.settings.font_size;
        ui.vertical(|ui| {
            ui.add_space(SPACE_MD);
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
            ui.separator();

            // Collect matching files
            let query = self.viewer.quick_open_query.to_lowercase();
            let mut matches: Vec<(String, PathBuf)> = Vec::new();
            if let Some(ref tree) = self.file_tree {
                let mut all_files = Vec::new();
                collect_file_paths(&tree.entries, &self.project_root, &mut all_files);
                for (rel, abs) in all_files {
                    if crate::app::search::is_binary_ext(&abs) {
                        continue;
                    }
                    if query.is_empty() || fuzzy_match(&rel.to_lowercase(), &query) {
                        matches.push((rel, abs));
                    }
                    if matches.len() >= 30 {
                        break;
                    }
                }
            }

            // Clamp selection index to the current match list
            if !matches.is_empty() {
                self.viewer.quick_open_selected =
                    self.viewer.quick_open_selected.min(matches.len() - 1);
            } else {
                self.viewer.quick_open_selected = 0;
            }

            // Handle arrow key navigation
            let (arrow_up, arrow_down, enter_pressed) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::ArrowUp),
                    i.key_pressed(egui::Key::ArrowDown),
                    i.key_pressed(egui::Key::Enter),
                )
            });
            if arrow_up && !matches.is_empty() {
                if self.viewer.quick_open_selected > 0 {
                    self.viewer.quick_open_selected -= 1;
                } else {
                    self.viewer.quick_open_selected = matches.len() - 1;
                }
            }
            if arrow_down && !matches.is_empty() {
                if self.viewer.quick_open_selected + 1 < matches.len() {
                    self.viewer.quick_open_selected += 1;
                } else {
                    self.viewer.quick_open_selected = 0;
                }
            }

            let mut navigate_to: Option<PathBuf> = None;
            let sel = self.viewer.quick_open_selected;

            egui::ScrollArea::vertical()
                .id_salt("quick_open_scroll")
                .max_height(ui.available_height() - SPACE_MD)
                .show(ui, |ui| {
                    for (i, (rel, abs)) in matches.iter().enumerate() {
                        let is_selected = i == sel;
                        if ui
                            .selectable_label(
                                is_selected,
                                egui::RichText::new(rel).monospace().small(),
                            )
                            .clicked()
                            || (is_selected && enter_pressed)
                        {
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

            if let Some(path) = navigate_to {
                self.push_nav_history();
                self.load_file(path);
                self.viewer.quick_open_active = false;
            }
        });
    }

    /// Go to definition of a symbol: search project files for definition patterns.
    fn goto_definition(&mut self, word: &str) {
        if word.is_empty() || word.len() < 2 {
            return;
        }

        let patterns = symbols::definition_patterns(word);
        if patterns.is_empty() {
            self.set_status_message(format!("No definition found for `{}`", word));
            return;
        }

        // First, search in the current file
        if let Some(tab) = self.viewer.active() {
            let mut in_block_comment = false;
            for (idx, line) in tab.content.iter().enumerate() {
                let trimmed = line.trim();
                if is_comment_line(trimmed, &mut in_block_comment) {
                    continue;
                }
                for re in &patterns {
                    if re.is_match(line) {
                        // Found in current file — jump to it
                        self.push_nav_history();
                        self.viewer.scroll_to_line = Some(idx + 1);
                        self.set_status_message(format!(
                            "Definition: `{}` at line {}",
                            word,
                            idx + 1
                        ));
                        return;
                    }
                }
            }
        }

        // Search in all project files (background thread)
        let mut all_files = Vec::new();
        if let Some(ref tree) = self.file_tree {
            crate::app::search::collect_files(&tree.entries, &mut all_files);
        }

        // Cancel any previous in-flight search and bump generation
        self.goto_def_cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.goto_def_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.goto_def_gen = self.goto_def_gen.wrapping_add(1);
        let gen = self.goto_def_gen;

        let current_file = self.viewer.current_file().cloned();
        let project_root = self.project_root.clone();
        let word_owned = word.to_string();
        let tx = self.goto_def_tx.clone();
        let cancelled = self.goto_def_cancel.clone();

        self.set_status_message(format!("Searching for `{}`...", word));

        std::thread::spawn(move || {
            for file_path in &all_files {
                if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                if crate::app::search::is_binary_ext(file_path) {
                    continue;
                }
                if current_file.as_ref() == Some(file_path) {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(file_path) {
                    let mut in_block_comment = false;
                    for (idx, line) in content.lines().enumerate() {
                        let trimmed = line.trim();
                        if is_comment_line(trimmed, &mut in_block_comment) {
                            continue;
                        }
                        for re in &patterns {
                            if re.is_match(line) {
                                let target_line = idx + 1;
                                let msg = format!(
                                    "Definition: `{}` at {}:{}",
                                    word_owned,
                                    file_path
                                        .strip_prefix(&project_root)
                                        .unwrap_or(file_path)
                                        .display(),
                                    target_line
                                );
                                let _ = tx.send((gen, file_path.clone(), target_line, msg));
                                return;
                            }
                        }
                    }
                }
            }
            let _ = tx.send((
                gen,
                PathBuf::new(),
                0,
                format!("No definition found for `{}`", word_owned),
            ));
        });
    }
}

/// Delegate to the shared comment detector in symbols.
fn is_comment_line(trimmed: &str, in_block_comment: &mut bool) -> bool {
    symbols::is_comment_line(trimmed, in_block_comment)
}

/// Recursively collect all file paths with their relative paths.
fn collect_file_paths(
    entries: &[FileEntry],
    project_root: &std::path::Path,
    out: &mut Vec<(String, PathBuf)>,
) {
    for entry in entries {
        if entry.is_dir {
            collect_file_paths(&entry.children, project_root, out);
        } else {
            let rel = entry
                .path
                .strip_prefix(project_root)
                .unwrap_or(&entry.path)
                .to_string_lossy()
                .to_string();
            out.push((rel, entry.path.clone()));
        }
    }
}

/// Simple fuzzy (subsequence) matching.
fn fuzzy_match(haystack: &str, needle: &str) -> bool {
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
