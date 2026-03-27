mod breadcrumb;
mod cue_input;
mod goto_definition;
mod line_rendering;
mod quick_open;
mod tab_bar;
pub(crate) mod types;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use eframe::egui;

use super::{DiffReview, DirigentApp, SPACE_MD, SPACE_SM};
use crate::diff_view::{self, DiffViewMode};
use crate::git;

use line_rendering::{
    build_diagnostic_lookup, build_diagnostic_messages, build_scroll_area, render_code_line,
};
use types::{CodeLineActions, CodeLineContext};

impl DirigentApp {
    pub(super) fn render_code_viewer(&mut self, ui: &mut egui::Ui) {
        if self.should_render_central_overlay(ui) {
            return;
        }
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.render_code_viewer_panel(&ctx, ui);
        });
    }

    /// Dispatch to any full-screen central overlay. Returns true if one was rendered.
    fn should_render_central_overlay(&mut self, ui: &mut egui::Ui) -> bool {
        if self.show_settings {
            self.render_settings_panel(ui);
            return true;
        }
        if self.diff_review.is_some() {
            self.render_diff_review_central(ui);
            return true;
        }
        if self.claude.show_log.is_some() {
            self.render_running_log_central(ui);
            return true;
        }
        if self.agent_state.show_output.is_some() {
            self.render_agent_log_central(ui);
            return true;
        }
        if self.show_agent_runs_for_cue.is_some() {
            self.render_cue_agent_runs_central(ui);
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
        let mut diag_lines =
            build_diagnostic_lookup(&self.agent_state.latest_diagnostics, rel_path);
        let mut diag_messages =
            build_diagnostic_messages(&self.agent_state.latest_diagnostics, rel_path);

        // Merge LSP diagnostics into the lookup
        if let Some(lsp_diags) = self.lsp.diagnostics.get(file_path) {
            use crate::agents::Severity;
            use crate::lsp::LspDiagSeverity;
            for d in lsp_diags {
                let sev = match d.severity {
                    LspDiagSeverity::Error => Severity::Error,
                    LspDiagSeverity::Warning => Severity::Warning,
                    LspDiagSeverity::Info | LspDiagSeverity::Hint => Severity::Info,
                };
                let entry = diag_lines.entry(d.line).or_insert(Severity::Info);
                match (&*entry, &sev) {
                    (Severity::Info, Severity::Warning | Severity::Error) => *entry = sev,
                    (Severity::Warning, Severity::Error) => *entry = sev,
                    _ => {}
                }
                let source_prefix = if d.source.is_empty() {
                    String::new()
                } else {
                    format!("[{}] ", d.source)
                };
                diag_messages
                    .entry(d.line)
                    .or_default()
                    .push(format!("{}{}", source_prefix, d.message));
            }
        }
        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();
        // Prefer LSP document symbols over regex-based ones when available
        let symbol_lines: HashMap<usize, (String, String)> =
            if let Some(lsp_syms) = self.lsp.document_symbols.get(file_path) {
                lsp_syms
                    .iter()
                    .map(|s| {
                        let kind_label = lsp_symbol_kind_label(s.kind);
                        (s.line, (kind_label.to_string(), s.name.clone()))
                    })
                    .collect()
            } else {
                self.viewer.tabs[active_idx]
                    .symbols
                    .iter()
                    .map(|s| (s.line, (s.kind.label().to_string(), s.name.clone())))
                    .collect()
            };
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
            lsp_hover_position: None,
            lsp_goto_def_position: None,
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
            file_path,
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
        file_path: &Path,
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
            // Try LSP go-to-definition first, fall back to regex
            if let Some((line_0based, char_0based)) = actions.lsp_goto_def_position {
                self.lsp_goto_definition(file_path, line_0based, char_0based, &w);
            } else {
                self.goto_definition(&w);
            }
        }

        // LSP hover: request hover info for the hovered position
        if let Some((line, character)) = actions.lsp_hover_position {
            if self.settings.lsp_enabled {
                self.lsp.hover(file_path, line, character);
            }
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
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            match self
                .db
                .insert_cue(&text, rel_path, start, line_end, &images)
            {
                Ok(_) => {
                    self.viewer.tabs[active_idx].cue_input.clear();
                    self.viewer.tabs[active_idx].cue_images.clear();
                    self.reload_cues();
                }
                Err(e) => {
                    eprintln!("Failed to insert cue: {e}");
                }
            }
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
}

/// Map LSP SymbolKind to a human-readable label.
fn lsp_symbol_kind_label(kind: lsp_types::SymbolKind) -> &'static str {
    match kind {
        lsp_types::SymbolKind::FUNCTION | lsp_types::SymbolKind::METHOD => "fn",
        lsp_types::SymbolKind::STRUCT => "struct",
        lsp_types::SymbolKind::ENUM => "enum",
        lsp_types::SymbolKind::INTERFACE => "interface",
        lsp_types::SymbolKind::CLASS => "class",
        lsp_types::SymbolKind::CONSTANT => "const",
        lsp_types::SymbolKind::MODULE | lsp_types::SymbolKind::NAMESPACE => "mod",
        lsp_types::SymbolKind::TYPE_PARAMETER => "type",
        lsp_types::SymbolKind::FIELD | lsp_types::SymbolKind::PROPERTY => "field",
        lsp_types::SymbolKind::VARIABLE => "var",
        lsp_types::SymbolKind::CONSTRUCTOR => "constructor",
        lsp_types::SymbolKind::ENUM_MEMBER => "variant",
        _ => "",
    }
}
