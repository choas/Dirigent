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

/// Highlight state for a single code line.
struct LineHighlight {
    in_selection: bool,
    current_search_match: bool,
    search_match: bool,
}

/// Accumulated actions from code line rendering, applied after the UI pass.
struct CodeLineActions {
    new_sel_start: Option<usize>,
    new_sel_end: Option<usize>,
    submit_cue: bool,
    clear_selection: bool,
    fix_diagnostic_line: Option<usize>,
    goto_def_word: Option<String>,
    implement_click_line: Option<usize>,
}

/// Per-render-pass context shared across all code lines.
struct CodeLineContext<'a> {
    active_idx: usize,
    sel_start: Option<usize>,
    sel_end: Option<usize>,
    lines_with_cues: &'a HashMap<usize, bool>,
    diag_lines: &'a HashMap<usize, Severity>,
    diag_messages: &'a HashMap<usize, Vec<String>>,
    ext: &'a str,
    symbol_lines: &'a HashMap<usize, (String, String)>,
    cmd_held: bool,
}

/// Result of tab bar rendering: what action, if any, to apply.
enum TabBarAction {
    None,
    CloseAll,
    CloseOthers(usize),
    CloseToRight(usize),
    CloseOne(usize),
    Activate(usize),
}

/// Result of breadcrumb bar rendering: what action, if any, to apply.
enum BreadcrumbAction {
    None,
    CloseFile,
    ShowFileDiff,
    ToggleMarkdown,
}

impl DirigentApp {
    pub(super) fn render_code_viewer(&mut self, ctx: &egui::Context) {
        if self.should_render_central_overlay(ctx) {
            return;
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_code_viewer_panel(ctx, ui);
        });
    }

    /// Dispatch to any full-screen central overlay. Returns true if one was rendered.
    fn should_render_central_overlay(&mut self, ctx: &egui::Context) -> bool {
        if self.show_settings {
            self.render_settings_panel(ctx);
            return true;
        }
        if self.diff_review.is_some() {
            self.render_diff_review_central(ctx);
            return true;
        }
        if self.claude.show_log.is_some() {
            self.render_running_log_central(ctx);
            return true;
        }
        if self.agent_state.show_output.is_some() {
            self.render_agent_log_central(ctx);
            return true;
        }
        if self.show_agent_runs_for_cue.is_some() {
            self.render_cue_agent_runs_central(ctx);
            return true;
        }
        false
    }

    /// Main code viewer panel body, rendered inside CentralPanel.
    fn render_code_viewer_panel(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        if self.viewer.quick_open_active {
            self.render_quick_open_overlay(ui);
            return;
        }

        let active_idx = match self.viewer.active_tab {
            Some(idx) if idx < self.viewer.tabs.len() => idx,
            _ => {
                self.render_splash(ctx, ui);
                return;
            }
        };

        if self.viewer.tabs.len() > 1 {
            let action = self.render_tab_bar(ui, active_idx);
            if self.apply_tab_bar_action(action) {
                return;
            }
        }

        let active_idx = match self.viewer.active_tab {
            Some(idx) if idx < self.viewer.tabs.len() => idx,
            _ => return,
        };

        let file_path = self.viewer.tabs[active_idx].file_path.clone();
        let rel_path = self.relative_path(&file_path);
        let is_dirty = self.git.dirty_files.contains_key(&rel_path);
        let is_markdown = self.viewer.tabs[active_idx].markdown_blocks.is_some();

        let bc_action =
            self.render_breadcrumb_bar(ui, active_idx, &rel_path, is_dirty, is_markdown);
        if self.apply_breadcrumb_action(bc_action, active_idx, &rel_path) {
            return;
        }

        ui.separator();

        let should_render_md = is_markdown && self.viewer.tabs[active_idx].markdown_rendered;
        if should_render_md {
            self.render_markdown_scroll(ui, active_idx);
            return;
        }

        if self.search.in_file_active {
            self.render_search_in_file_bar(ui);
        }

        self.render_code_lines(ui, active_idx, &file_path, &rel_path);
    }

    /// Render the virtualized code lines (scroll area + per-line rendering + post-actions).
    fn render_code_lines(
        &mut self,
        ui: &mut egui::Ui,
        active_idx: usize,
        file_path: &Path,
        rel_path: &str,
    ) {
        let lines_with_cues = self.lines_with_cues();
        let num_lines = self.viewer.tabs[active_idx].content.len();
        let line_height = 16.0;
        let diag_lines = build_diagnostic_lookup(&self.agent_state.latest_diagnostics, rel_path);
        let diag_messages =
            build_diagnostic_messages(&self.agent_state.latest_diagnostics, rel_path);
        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();
        let symbol_lines: HashMap<usize, (String, String)> = self.viewer.tabs[active_idx]
            .symbols
            .iter()
            .map(|s| (s.line, (s.kind.label().to_string(), s.name.clone())))
            .collect();
        let sel_start = self.viewer.tabs[active_idx].selection_start;
        let sel_end = self.viewer.tabs[active_idx].selection_end;

        let scroll_offset = self.viewer.scroll_to_line.take().map(|target_line| {
            let row_height_with_spacing = line_height + ui.spacing().item_spacing.y;
            (target_line.saturating_sub(1)) as f32 * row_height_with_spacing
        });
        let cmd_held = ui.input(|i| i.modifiers.command);

        let mut actions = CodeLineActions {
            new_sel_start: sel_start,
            new_sel_end: sel_end,
            submit_cue: false,
            clear_selection: false,
            fix_diagnostic_line: None,
            goto_def_word: None,
            implement_click_line: None,
        };

        let scroll_area =
            build_scroll_area(scroll_offset, self.viewer.tabs[active_idx].scroll_offset);
        let ctx = CodeLineContext {
            active_idx,
            sel_start,
            sel_end,
            lines_with_cues: &lines_with_cues,
            diag_lines: &diag_lines,
            diag_messages: &diag_messages,
            ext: &ext,
            symbol_lines: &symbol_lines,
            cmd_held,
        };
        let output = scroll_area.show_rows(ui, line_height, num_lines, |ui, row_range| {
            for line_idx in row_range {
                render_code_line(ui, self, line_idx, &ctx, &mut actions);
            }
        });
        self.viewer.tabs[active_idx].scroll_offset = output.state.offset.y;

        self.apply_code_line_actions(
            &mut actions,
            active_idx,
            rel_path,
            &diag_messages,
            &symbol_lines,
        );
    }

    /// Render the splash screen shown when no file is open.
    fn render_splash(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
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
                egui::RichText::new(
                    "Select a file from the tree to view  \u{2022}  \u{2318}P to quick open",
                )
                .weak(),
            );
        });
    }

    /// Render the tab bar and return the action to take.
    fn render_tab_bar(&mut self, ui: &mut egui::Ui, active_idx: usize) -> TabBarAction {
        let mut action = TabBarAction::None;

        ui.horizontal(|ui| {
            for i in 0..self.viewer.tabs.len() {
                self.render_single_tab(ui, i, i == active_idx, &mut action);
            }
        });

        ui.separator();
        action
    }

    /// Render one tab in the tab bar.
    fn render_single_tab(
        &self,
        ui: &mut egui::Ui,
        i: usize,
        is_active: bool,
        action: &mut TabBarAction,
    ) {
        let tab = &self.viewer.tabs[i];
        let filename = tab
            .file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "untitled".to_string());

        let text = tab_label_text(
            &filename,
            is_active,
            self.semantic.accent,
            self.semantic.secondary_text,
        );

        let frame = if is_active {
            egui::Frame::NONE
                .inner_margin(egui::Margin::symmetric(6, 3))
                .fill(self.semantic.selection_bg())
                .corner_radius(3)
        } else {
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 3))
        };

        let rel = tab
            .file_path
            .strip_prefix(&self.project_root)
            .unwrap_or(&tab.file_path)
            .to_string_lossy()
            .to_string();

        let tab_resp = frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .add(egui::Label::new(text).sense(egui::Sense::click()))
                    .on_hover_text(&rel)
                    .clicked()
                {
                    *action = TabBarAction::Activate(i);
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
                    *action = TabBarAction::CloseOne(i);
                }
            });
        });

        let ctx_resp = ui.interact(
            tab_resp.response.rect,
            ui.id().with(("tab_ctx", i)),
            egui::Sense::click(),
        );
        if ctx_resp.clicked() {
            *action = TabBarAction::Activate(i);
        }
        self.render_tab_context_menu(&ctx_resp, i, action);
    }

    /// Show the right-click context menu on a tab.
    fn render_tab_context_menu(
        &self,
        ctx_resp: &egui::Response,
        tab_index: usize,
        action: &mut TabBarAction,
    ) {
        ctx_resp.context_menu(|ui| {
            if ui.button("Close").clicked() {
                *action = TabBarAction::CloseOne(tab_index);
                ui.close();
            }
            if ui.button("Close Others").clicked() {
                *action = TabBarAction::CloseOthers(tab_index);
                ui.close();
            }
            if ui.button("Close All").clicked() {
                *action = TabBarAction::CloseAll;
                ui.close();
            }
            if ui.button("Close Tabs to the Right").clicked() {
                *action = TabBarAction::CloseToRight(tab_index);
                ui.close();
            }
        });
    }

    /// Apply a tab bar action. Returns true if the caller should return early.
    fn apply_tab_bar_action(&mut self, action: TabBarAction) -> bool {
        match action {
            TabBarAction::None => false,
            TabBarAction::CloseAll => {
                self.viewer.close_all_tabs();
                self.search.in_file_active = false;
                self.search.in_file_query.clear();
                self.search.in_file_matches.clear();
                self.search.in_file_current = None;
                true
            }
            TabBarAction::CloseOthers(idx) => {
                self.viewer.close_other_tabs(idx);
                self.search.in_file_active = false;
                self.search.in_file_query.clear();
                self.search.in_file_matches.clear();
                self.search.in_file_current = None;
                true
            }
            TabBarAction::CloseToRight(idx) => {
                self.viewer.close_tabs_to_right(idx);
                self.search.in_file_active = false;
                self.search.in_file_query.clear();
                self.search.in_file_matches.clear();
                self.search.in_file_current = None;
                true
            }
            TabBarAction::CloseOne(idx) => {
                self.viewer.close_tab(idx);
                self.search.in_file_active = false;
                self.search.in_file_query.clear();
                self.search.in_file_matches.clear();
                self.search.in_file_current = None;
                true
            }
            TabBarAction::Activate(idx) => {
                self.viewer.active_tab = Some(idx);
                self.search.in_file_active = false;
                self.search.in_file_query.clear();
                self.search.in_file_matches.clear();
                self.search.in_file_current = None;
                true
            }
        }
    }

    /// Render the breadcrumb navigation bar and the right-side controls.
    fn render_breadcrumb_bar(
        &mut self,
        ui: &mut egui::Ui,
        active_idx: usize,
        rel_path: &str,
        is_dirty: bool,
        is_markdown: bool,
    ) -> BreadcrumbAction {
        let mut bc_action = BreadcrumbAction::None;

        ui.horizontal(|ui| {
            self.render_breadcrumb_segments(ui, active_idx, rel_path);

            if ui
                .small_button(icon("\u{1F4CB}", self.settings.font_size))
                .on_hover_text("Copy file path")
                .clicked()
            {
                ui.ctx().copy_text(rel_path.to_string());
            }
            if is_dirty {
                ui.label(egui::RichText::new("\u{25CF}").color(self.semantic.warning));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                bc_action = self.render_breadcrumb_right_controls(
                    ui,
                    active_idx,
                    rel_path,
                    is_dirty,
                    is_markdown,
                );
            });
        });

        bc_action
    }

    /// Render the right-aligned controls in the breadcrumb bar (close, diagnostics, markdown toggle, diff).
    fn render_breadcrumb_right_controls(
        &mut self,
        ui: &mut egui::Ui,
        active_idx: usize,
        rel_path: &str,
        is_dirty: bool,
        is_markdown: bool,
    ) -> BreadcrumbAction {
        if ui
            .small_button(icon("\u{2715}", self.settings.font_size))
            .on_hover_text("Close tab (\u{2318}W)")
            .clicked()
        {
            return BreadcrumbAction::CloseFile;
        }
        self.render_breadcrumb_diagnostics(ui, rel_path);
        ui.label(format!(
            "{} lines",
            self.viewer.tabs[active_idx].content.len()
        ));
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
                return BreadcrumbAction::ToggleMarkdown;
            }
        }
        if is_dirty
            && ui
                .small_button("Show Diff")
                .on_hover_text("Show uncommitted changes for this file")
                .clicked()
        {
            return BreadcrumbAction::ShowFileDiff;
        }
        BreadcrumbAction::None
    }

    /// Render path segments and the current symbol in the breadcrumb bar.
    fn render_breadcrumb_segments(&mut self, ui: &mut egui::Ui, active_idx: usize, rel_path: &str) {
        let segments: Vec<&str> = rel_path.split('/').collect();
        for (i, segment) in segments.iter().enumerate() {
            if i > 0 {
                ui.label(egui::RichText::new("\u{203A}").color(self.semantic.tertiary_text));
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
            let hover = if is_last {
                "Scroll to top".to_string()
            } else {
                format!("Expand directory {}", segments[..=i].join("/"))
            };
            if ui
                .add(egui::Label::new(text).sense(egui::Sense::click()))
                .on_hover_text(hover)
                .clicked()
            {
                self.handle_breadcrumb_segment_click(is_last, &segments[..=i]);
            }
        }

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
            ui.label(egui::RichText::new("\u{203A}").color(self.semantic.tertiary_text));
            ui.label(
                egui::RichText::new(format!("{} {}", sym.kind.label(), sym.name))
                    .small()
                    .color(self.semantic.accent),
            );
        }
    }

    fn handle_breadcrumb_segment_click(&mut self, is_last: bool, path_segments: &[&str]) {
        if is_last {
            self.viewer.scroll_to_line = Some(1);
        } else {
            let dir_path = self.project_root.join(path_segments.join("/"));
            self.expanded_dirs.insert(dir_path);
        }
    }

    /// Render the diagnostic error/warning counts in the breadcrumb right side.
    fn render_breadcrumb_diagnostics(&self, ui: &mut egui::Ui, rel_path: &str) {
        let mut err_count = 0usize;
        let mut warn_count = 0usize;
        for diags in self.agent_state.latest_diagnostics.values() {
            for d in diags {
                if path_matches_diagnostic(rel_path, &d.file) {
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

    /// Apply a breadcrumb action. Returns true if the caller should return early.
    fn apply_breadcrumb_action(
        &mut self,
        action: BreadcrumbAction,
        active_idx: usize,
        rel_path: &str,
    ) -> bool {
        match action {
            BreadcrumbAction::ToggleMarkdown => {
                self.viewer.tabs[active_idx].markdown_rendered =
                    !self.viewer.tabs[active_idx].markdown_rendered;
                false
            }
            BreadcrumbAction::CloseFile => {
                self.viewer.close_active_tab();
                self.search.in_file_active = false;
                self.search.in_file_query.clear();
                self.search.in_file_matches.clear();
                self.search.in_file_current = None;
                true
            }
            BreadcrumbAction::ShowFileDiff => {
                self.open_file_diff(rel_path);
                true
            }
            BreadcrumbAction::None => false,
        }
    }

    /// Open a diff review for a dirty file.
    fn open_file_diff(&mut self, rel_path: &str) {
        let files = vec![rel_path.to_string()];
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
    }

    /// Render the markdown scroll area, converting scroll_to_line into scroll_to_heading.
    fn render_markdown_scroll(&mut self, ui: &mut egui::Ui, active_idx: usize) {
        if let Some(target_line) = self.viewer.scroll_to_line.take() {
            let symbols = &self.viewer.tabs[active_idx].symbols;
            for (heading_idx, sym) in symbols.iter().enumerate() {
                if sym.line == target_line {
                    self.viewer.scroll_to_heading = Some(heading_idx);
                    break;
                }
            }
        }
        egui::ScrollArea::both()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.add_space(SPACE_SM);
                self.render_markdown_content(ui);
                ui.add_space(SPACE_MD);
            });
    }

    /// Apply accumulated actions from code line rendering.
    fn apply_code_line_actions(
        &mut self,
        actions: &mut CodeLineActions,
        active_idx: usize,
        rel_path: &str,
        diag_messages: &HashMap<usize, Vec<String>>,
        symbol_lines: &HashMap<usize, (String, String)>,
    ) {
        if actions.clear_selection {
            actions.new_sel_start = None;
            actions.new_sel_end = None;
        }

        if let Some(line) = actions.fix_diagnostic_line {
            self.apply_fix_diagnostic(active_idx, line, diag_messages, actions);
        }

        if actions.new_sel_start != self.viewer.tabs[active_idx].selection_start
            || actions.new_sel_end != self.viewer.tabs[active_idx].selection_end
        {
            self.viewer.tabs[active_idx].selection_start = actions.new_sel_start;
            self.viewer.tabs[active_idx].selection_end = actions.new_sel_end;
            self.viewer.tabs[active_idx].cue_input.clear();
            self.viewer.tabs[active_idx].cue_images.clear();
        }

        if actions.submit_cue && !self.viewer.tabs[active_idx].cue_input.is_empty() {
            self.submit_inline_cue(active_idx, rel_path);
        }

        if let Some(line) = actions.implement_click_line {
            self.apply_implement_click(active_idx, line, symbol_lines);
        }

        if let Some(ref word) = actions.goto_def_word {
            let w = word.clone();
            self.goto_definition(&w);
        }
    }

    /// Apply the diagnostic "Fix" click: pre-fill cue input.
    fn apply_fix_diagnostic(
        &mut self,
        active_idx: usize,
        line: usize,
        diag_messages: &HashMap<usize, Vec<String>>,
        actions: &mut CodeLineActions,
    ) {
        if let Some(msgs) = diag_messages.get(&line) {
            let fix_text = format!("Fix: {}", msgs.join("; "));
            actions.new_sel_start = Some(line);
            actions.new_sel_end = Some(line);
            self.viewer.tabs[active_idx].selection_start = Some(line);
            self.viewer.tabs[active_idx].selection_end = Some(line);
            self.viewer.tabs[active_idx].cue_input = fix_text;
            self.viewer.tabs[active_idx].cue_images.clear();
        }
    }

    /// Submit the inline cue from the code viewer selection.
    fn submit_inline_cue(&mut self, active_idx: usize, rel_path: &str) {
        if let Some(start) = self.viewer.tabs[active_idx].selection_start {
            let end = self.viewer.tabs[active_idx].selection_end.unwrap_or(start);
            let line_end = if end > start { Some(end) } else { None };
            let text = self.viewer.tabs[active_idx].cue_input.clone();
            let images: Vec<String> = self.viewer.tabs[active_idx]
                .cue_images
                .drain(..)
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            let _ = self
                .db
                .insert_cue(&text, rel_path, start, line_end, &images);
            self.viewer.tabs[active_idx].cue_input.clear();
            self.reload_cues();
        }
    }

    /// Pre-fill cue input with "Implement <name>" when clicking a symbol line.
    fn apply_implement_click(
        &mut self,
        active_idx: usize,
        line: usize,
        symbol_lines: &HashMap<usize, (String, String)>,
    ) {
        if let Some((kind_label, sym_name)) = symbol_lines.get(&line) {
            let text = if kind_label.is_empty() {
                format!("Implement `{}`", sym_name)
            } else {
                format!("Implement {} `{}`", kind_label, sym_name)
            };
            self.viewer.tabs[active_idx].cue_input = text;
            self.viewer.tabs[active_idx].cue_images.clear();
        }
    }

    /// Quick-open file overlay (Cmd+P).
    fn render_quick_open_overlay(&mut self, ui: &mut egui::Ui) {
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

        if self.search_definition_in_current_file(word, &patterns) {
            return;
        }

        self.spawn_definition_search(word, patterns);
    }

    /// Search for a definition in the current file. Returns true if found.
    fn search_definition_in_current_file(&mut self, word: &str, patterns: &[regex::Regex]) -> bool {
        let tab = match self.viewer.active() {
            Some(t) => t,
            None => return false,
        };
        let mut in_block_comment = false;
        for (idx, line) in tab.content.iter().enumerate() {
            let trimmed = line.trim();
            if is_comment_line(trimmed, &mut in_block_comment) {
                continue;
            }
            let found = patterns.iter().any(|re| re.is_match(line));
            if found {
                self.push_nav_history();
                self.viewer.scroll_to_line = Some(idx + 1);
                self.set_status_message(format!("Definition: `{}` at line {}", word, idx + 1));
                return true;
            }
        }
        false
    }

    /// Spawn a background thread to search for a symbol definition across project files.
    fn spawn_definition_search(&mut self, word: &str, patterns: Vec<regex::Regex>) {
        let mut all_files = Vec::new();
        if let Some(ref tree) = self.file_tree {
            crate::app::search::collect_files(&tree.entries, &mut all_files);
        }

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
            let result = search_files_for_definition(
                &all_files,
                &patterns,
                &cancelled,
                current_file.as_ref(),
                &project_root,
                &word_owned,
                gen,
            );
            let _ = tx.send(result.unwrap_or_else(|| {
                (
                    gen,
                    PathBuf::new(),
                    0,
                    format!("No definition found for `{}`", word_owned),
                )
            }));
        });
    }
}

/// Build the tab label text with proper styling.
fn tab_label_text(
    filename: &str,
    is_active: bool,
    accent: egui::Color32,
    secondary: egui::Color32,
) -> egui::RichText {
    if is_active {
        egui::RichText::new(filename).small().strong().color(accent)
    } else {
        egui::RichText::new(filename).small().color(secondary)
    }
}

/// Check whether a relative path matches a diagnostic file path.
fn path_matches_diagnostic(rel_path: &str, diag_file: &str) -> bool {
    let (rel, diag_p) = (Path::new(rel_path), Path::new(diag_file));
    diag_p == rel || rel.ends_with(diag_p) || diag_p.ends_with(rel)
}

/// Build a scroll area with the correct vertical offset.
fn build_scroll_area(scroll_offset: Option<f32>, tab_offset: f32) -> egui::ScrollArea {
    let mut scroll_area = egui::ScrollArea::both().auto_shrink([false; 2]);
    if let Some(offset) = scroll_offset {
        scroll_area = scroll_area.vertical_scroll_offset(offset);
    } else if tab_offset > 0.0 {
        scroll_area = scroll_area.vertical_scroll_offset(tab_offset);
    }
    scroll_area
}

/// Build diagnostic severity lookup: line_num -> worst severity for this file.
fn build_diagnostic_lookup(
    diagnostics: &HashMap<crate::agents::AgentKind, Vec<crate::agents::Diagnostic>>,
    rel_path: &str,
) -> HashMap<usize, Severity> {
    let mut map: HashMap<usize, Severity> = HashMap::new();
    for diags in diagnostics.values() {
        for d in diags {
            if !path_matches_diagnostic(rel_path, &d.file) {
                continue;
            }
            let entry = map.entry(d.line).or_insert(Severity::Info);
            match (&*entry, &d.severity) {
                (Severity::Info, Severity::Warning | Severity::Error) => *entry = d.severity,
                (Severity::Warning, Severity::Error) => *entry = d.severity,
                _ => {}
            }
        }
    }
    map
}

/// Collect diagnostic messages per line for tooltip display.
fn build_diagnostic_messages(
    diagnostics: &HashMap<crate::agents::AgentKind, Vec<crate::agents::Diagnostic>>,
    rel_path: &str,
) -> HashMap<usize, Vec<String>> {
    let mut map: HashMap<usize, Vec<String>> = HashMap::new();
    for diags in diagnostics.values() {
        for d in diags {
            if path_matches_diagnostic(rel_path, &d.file) {
                map.entry(d.line).or_default().push(d.message.clone());
            }
        }
    }
    map
}

/// Render a single code line (gutter, line number, syntax-highlighted text, overlays).
fn render_code_line(
    ui: &mut egui::Ui,
    app: &mut DirigentApp,
    active_idx: usize,
    line_idx: usize,
    sel_start: Option<usize>,
    sel_end: Option<usize>,
    lines_with_cues: &HashMap<usize, bool>,
    diag_lines: &HashMap<usize, Severity>,
    diag_messages: &HashMap<usize, Vec<String>>,
    ext: &str,
    symbol_lines: &HashMap<usize, (String, String)>,
    cmd_held: bool,
    actions: &mut CodeLineActions,
) {
    let line_num = line_idx + 1;
    let line_text = app.viewer.tabs[active_idx]
        .content
        .get(line_idx)
        .map(|s| s.as_str())
        .unwrap_or("");

    let is_in_selection = matches!(
        (sel_start, sel_end),
        (Some(s), Some(e)) if line_num >= s && line_num <= e
    );
    let is_selection_end = sel_end == Some(line_num);
    let cue_state = lines_with_cues.get(&line_num);
    let is_search_match = app.search.in_file_matches.contains(&line_num);
    let is_current_search_match = app
        .search
        .in_file_current
        .map(|i| app.search.in_file_matches.get(i) == Some(&line_num))
        .unwrap_or(false);

    let response = ui.horizontal(|ui| {
        render_gutter_indicator(
            ui,
            app,
            cue_state,
            line_num,
            diag_lines,
            diag_messages,
            actions,
        );

        let num_text = format!("{:>4} ", line_num);
        ui.label(
            egui::RichText::new(num_text)
                .monospace()
                .size(app.settings.font_size * FONT_SCALE_LINE_NUM)
                .color(app.semantic.tertiary_text),
        );

        let layout_job = egui_extras::syntax_highlighting::highlight(
            ui.ctx(),
            ui.style(),
            &app.viewer.syntax_theme,
            line_text,
            ext,
        );
        let code_resp = ui.label(layout_job);

        let rect = code_resp.rect.union(ui.available_rect_before_wrap());
        let line_response = ui.interact(
            rect,
            egui::Id::new(("code_line", line_idx)),
            egui::Sense::click(),
        );

        render_cmd_hover_underline(
            ui,
            cmd_held,
            &line_response,
            &code_resp,
            app.semantic.accent,
        );
        render_line_background(
            ui,
            rect,
            &LineHighlight {
                in_selection: is_in_selection,
                current_search_match: is_current_search_match,
                search_match: is_search_match,
            },
            app,
        );

        (line_response, code_resp)
    });

    let (line_response, code_resp) = response.inner;

    if line_response.clicked() {
        handle_line_click(
            ui,
            app,
            line_num,
            line_text,
            &code_resp,
            sel_start,
            symbol_lines,
            actions,
        );
    }

    if is_selection_end {
        render_cue_input(ui, app, active_idx, sel_start, sel_end, actions);
    }
}

/// Render the gutter indicator (cue dot, diagnostic dot, or blank).
fn render_gutter_indicator(
    ui: &mut egui::Ui,
    app: &DirigentApp,
    cue_state: Option<&bool>,
    line_num: usize,
    diag_lines: &HashMap<usize, Severity>,
    diag_messages: &HashMap<usize, Vec<String>>,
    actions: &mut CodeLineActions,
) {
    match cue_state {
        Some(&true) => {
            ui.label(icon("\u{25CF}", app.settings.font_size).color(app.semantic.secondary_text));
        }
        Some(&false) => {
            ui.label(icon("\u{25CF}", app.settings.font_size).color(app.semantic.warning));
        }
        None => {
            render_diagnostic_gutter(ui, app, line_num, diag_lines, diag_messages, actions);
        }
    }
}

/// Render a diagnostic indicator in the gutter, or a blank space.
fn render_diagnostic_gutter(
    ui: &mut egui::Ui,
    app: &DirigentApp,
    line_num: usize,
    diag_lines: &HashMap<usize, Severity>,
    diag_messages: &HashMap<usize, Vec<String>>,
    actions: &mut CodeLineActions,
) {
    let sev = match diag_lines.get(&line_num) {
        Some(s) => s,
        None => {
            ui.label(" ");
            return;
        }
    };
    let (sym, color) = match sev {
        Severity::Error => ("\u{25CF}", app.semantic.danger),
        Severity::Warning => ("\u{25CF}", app.semantic.warning),
        Severity::Info => ("\u{25CB}", app.semantic.accent),
    };
    let resp = ui.add(
        egui::Label::new(icon(sym, app.settings.font_size).color(color))
            .sense(egui::Sense::click()),
    );
    if let Some(msgs) = diag_messages.get(&line_num) {
        let tooltip = format!("{}\n\nClick to create a Fix cue", msgs.join("\n"));
        resp.clone().on_hover_text(tooltip);
    }
    if resp.clicked() {
        actions.fix_diagnostic_line = Some(line_num);
    }
}

/// Show underline hint when Cmd is held (go-to-definition).
fn render_cmd_hover_underline(
    ui: &egui::Ui,
    cmd_held: bool,
    line_response: &egui::Response,
    code_resp: &egui::Response,
    accent: egui::Color32,
) {
    if cmd_held && line_response.hovered() {
        ui.painter().hline(
            code_resp.rect.x_range(),
            code_resp.rect.bottom(),
            egui::Stroke::new(1.0, accent),
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

/// Render the line background: selection, search highlights, flash animation.
fn render_line_background(ui: &egui::Ui, rect: egui::Rect, hl: &LineHighlight, app: &DirigentApp) {
    if hl.in_selection {
        ui.painter()
            .rect_filled(rect, 0.0, app.semantic.selection_bg());
    }

    if hl.current_search_match {
        ui.painter()
            .rect_filled(rect, 0.0, app.semantic.code_search_current());
        render_search_flash(ui, rect, app);
    } else if hl.search_match {
        ui.painter()
            .rect_filled(rect, 0.0, app.semantic.code_search_match());
    }
}

/// Render the search navigation flash animation.
fn render_search_flash(ui: &egui::Ui, rect: egui::Rect, app: &DirigentApp) {
    let when = match app.search.in_file_nav_flash {
        Some(w) => w,
        None => return,
    };
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

/// Handle a click on a code line.
fn handle_line_click(
    ui: &mut egui::Ui,
    app: &DirigentApp,
    line_num: usize,
    line_text: &str,
    code_resp: &egui::Response,
    sel_start: Option<usize>,
    symbol_lines: &HashMap<usize, (String, String)>,
    actions: &mut CodeLineActions,
) {
    let shift_held = ui.input(|i| i.modifiers.shift);
    let cmd = ui.input(|i| i.modifiers.command);

    if cmd && !shift_held {
        handle_goto_def_click(ui, app, line_text, code_resp, actions);
    } else if shift_held {
        handle_shift_click(line_num, sel_start, actions);
    } else {
        actions.new_sel_start = Some(line_num);
        actions.new_sel_end = Some(line_num);
        if symbol_lines.contains_key(&line_num) {
            actions.implement_click_line = Some(line_num);
        }
    }
}

/// Handle Cmd+click for go-to-definition.
fn handle_goto_def_click(
    ui: &mut egui::Ui,
    app: &DirigentApp,
    line_text: &str,
    code_resp: &egui::Response,
    actions: &mut CodeLineActions,
) {
    let pos = match ui.ctx().pointer_latest_pos() {
        Some(p) => p,
        None => return,
    };
    let x_offset = (pos.x - code_resp.rect.left()).max(0.0);
    let approx_char_width = app.settings.font_size * 0.6;
    let byte_offset = compute_byte_offset(line_text, x_offset, approx_char_width);
    if let Some(word) = symbols::word_at_offset(line_text, byte_offset) {
        actions.goto_def_word = Some(word.to_string());
    }
}

/// Compute the byte offset from a pixel x-offset within a line of text.
fn compute_byte_offset(line_text: &str, x_offset: f32, approx_char_width: f32) -> usize {
    let mut accumulated = 0.0_f32;
    for (idx, ch) in line_text.char_indices() {
        let ch_width = if ch == '\t' {
            approx_char_width * 4.0
        } else {
            approx_char_width * (ch.len_utf8().max(1) as f32).min(2.0)
        };
        if accumulated + ch_width > x_offset {
            return idx;
        }
        accumulated += ch_width;
    }
    line_text.len()
}

/// Handle Shift+click to extend selection.
fn handle_shift_click(line_num: usize, sel_start: Option<usize>, actions: &mut CodeLineActions) {
    if let Some(anchor) = sel_start {
        let lo = anchor.min(line_num);
        let hi = anchor.max(line_num);
        actions.new_sel_start = Some(lo);
        actions.new_sel_end = Some(hi);
    } else {
        actions.new_sel_start = Some(line_num);
        actions.new_sel_end = Some(line_num);
    }
}

/// Render the cue input row (images + text field) after the last selected line.
fn render_cue_input(
    ui: &mut egui::Ui,
    app: &mut DirigentApp,
    active_idx: usize,
    sel_start: Option<usize>,
    sel_end: Option<usize>,
    actions: &mut CodeLineActions,
) {
    let range_label = if sel_start == sel_end {
        format!("L{}", sel_start.unwrap_or(0))
    } else {
        format!("L{}-{}", sel_start.unwrap_or(0), sel_end.unwrap_or(0))
    };

    render_cue_images_row(ui, app, active_idx);
    render_cue_text_input(ui, app, active_idx, &range_label, actions);
}

/// Render attached cue images row (if any).
fn render_cue_images_row(ui: &mut egui::Ui, app: &mut DirigentApp, active_idx: usize) {
    if app.viewer.tabs[active_idx].cue_images.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label("     ");
        ui.label(
            egui::RichText::new("Images:")
                .small()
                .color(app.semantic.accent),
        );
        let mut remove_idx = None;
        for (i, path) in app.viewer.tabs[active_idx].cue_images.iter().enumerate() {
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
            app.viewer.tabs[active_idx].cue_images.remove(i);
        }
    });
}

/// Render the cue text input field with Add/Close buttons.
fn render_cue_text_input(
    ui: &mut egui::Ui,
    app: &mut DirigentApp,
    active_idx: usize,
    range_label: &str,
    actions: &mut CodeLineActions,
) {
    ui.horizontal(|ui| {
        ui.label("     ");
        ui.label(
            egui::RichText::new(range_label)
                .monospace()
                .color(app.semantic.success),
        );
        if ui.button("+").on_hover_text("Attach images").clicked() {
            if let Some(paths) = rfd::FileDialog::new()
                .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "bmp"])
                .pick_files()
            {
                app.viewer.tabs[active_idx].cue_images.extend(paths);
            }
        }
        let input_response = ui.add(
            egui::TextEdit::singleline(&mut app.viewer.tabs[active_idx].cue_input)
                .desired_width(ui.available_width() - 80.0)
                .hint_text("Add a cue...")
                .font(egui::TextStyle::Monospace),
        );
        let enter = input_response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("Add").clicked() || enter {
            actions.submit_cue = true;
        }
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if ui
            .button(icon("\u{2715}", app.settings.font_size))
            .clicked()
            || esc
        {
            actions.clear_selection = true;
        }
    });
}

/// Search project files for a definition in a background thread.
fn search_files_for_definition(
    all_files: &[PathBuf],
    patterns: &[regex::Regex],
    cancelled: &std::sync::atomic::AtomicBool,
    current_file: Option<&PathBuf>,
    project_root: &Path,
    word: &str,
    gen: u64,
) -> Option<(u64, PathBuf, usize, String)> {
    for file_path in all_files {
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        if crate::app::search::is_binary_ext(file_path) {
            continue;
        }
        if current_file == Some(file_path) {
            continue;
        }
        let result =
            search_single_file_for_definition(file_path, patterns, project_root, word, gen);
        if result.is_some() {
            return result;
        }
    }
    None
}

/// Search a single file for a definition matching the given patterns.
fn search_single_file_for_definition(
    file_path: &Path,
    patterns: &[regex::Regex],
    project_root: &Path,
    word: &str,
    gen: u64,
) -> Option<(u64, PathBuf, usize, String)> {
    let content = std::fs::read_to_string(file_path).ok()?;
    let mut in_block_comment = false;
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if is_comment_line(trimmed, &mut in_block_comment) {
            continue;
        }
        let found = patterns.iter().any(|re| re.is_match(line));
        if found {
            let target_line = idx + 1;
            let msg = format!(
                "Definition: `{}` at {}:{}",
                word,
                file_path
                    .strip_prefix(project_root)
                    .unwrap_or(file_path)
                    .display(),
                target_line
            );
            return Some((gen, file_path.to_path_buf(), target_line, msg));
        }
    }
    None
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
