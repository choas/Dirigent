use std::collections::HashMap;
use std::path::Path;

use eframe::egui;

use super::cue_input::render_cue_input;
use super::types::{CodeLineActions, CodeLineContext, LineHighlight};
use crate::agents::Severity;
use crate::app::{icon, symbols, DirigentApp, FONT_SCALE_LINE_NUM};

/// Check whether a relative path matches a diagnostic file path.
pub(crate) fn path_matches_diagnostic(rel_path: &str, diag_file: &str) -> bool {
    let (rel, diag_p) = (Path::new(rel_path), Path::new(diag_file));
    diag_p == rel || rel.ends_with(diag_p) || diag_p.ends_with(rel)
}

/// Build a scroll area with the correct vertical offset.
pub(crate) fn build_scroll_area(scroll_offset: Option<f32>, tab_offset: f32) -> egui::ScrollArea {
    let offset = scroll_offset.unwrap_or(tab_offset);
    egui::ScrollArea::both()
        .auto_shrink([false; 2])
        .vertical_scroll_offset(offset)
}

/// Build diagnostic severity lookup: line_num -> worst severity for this file.
pub(crate) fn build_diagnostic_lookup(
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
pub(crate) fn build_diagnostic_messages(
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
pub(crate) fn render_code_line(
    ui: &mut egui::Ui,
    app: &mut DirigentApp,
    line_idx: usize,
    ctx: &CodeLineContext,
    actions: &mut CodeLineActions,
) {
    let line_num = line_idx + 1;
    let line_text = app.viewer.tabs[ctx.active_idx]
        .content
        .get(line_idx)
        .map(|s| s.as_str())
        .unwrap_or("");

    let is_in_selection = matches!(
        (ctx.sel_start, ctx.sel_end),
        (Some(s), Some(e)) if line_num >= s && line_num <= e
    );
    let is_selection_end = ctx.sel_end == Some(line_num);
    let cue_state = ctx.lines_with_cues.get(&line_num);
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
            ctx.diag_lines,
            ctx.diag_messages,
            actions,
        );

        let num_text = format!("{:>4} ", line_num);
        ui.label(
            egui::RichText::new(num_text)
                .monospace()
                .size(app.settings.font_size * FONT_SCALE_LINE_NUM)
                .color(app.semantic.tertiary_text),
        );

        let layout_job = crate::syntax::highlight(
            ui.ctx(),
            ui.style(),
            &app.viewer.syntax_theme,
            line_text,
            ctx.ext,
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
            ctx.cmd_held,
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
            ctx.sel_start,
            ctx.symbol_lines,
            actions,
        );
    }

    // LSP hover: track hovered position for tooltip requests
    if line_response.hovered() && !ctx.cmd_held {
        if let Some(pos) = ui.ctx().pointer_latest_pos() {
            let x_offset = (pos.x - code_resp.rect.left()).max(0.0);
            let approx_char_width = app.settings.font_size * 0.6;
            let byte_offset = compute_byte_offset(line_text, x_offset, approx_char_width);
            // Convert byte offset to character column (0-based)
            let character = line_text[..byte_offset.min(line_text.len())]
                .chars()
                .count() as u32;
            actions.lsp_hover_position = Some((line_idx as u32, character));
        }
    }

    // Show LSP hover tooltip if available for this line
    if line_response.hovered() {
        if let Some(ref hover_text) = app.lsp.hover_result {
            if app
                .lsp
                .hover_file
                .as_ref()
                .and_then(|f| app.viewer.active().map(|t| t.file_path == *f))
                .unwrap_or(false)
                && app.lsp.hover_line == line_idx as u32
            {
                line_response.on_hover_text(hover_text);
            }
        }
    }

    if is_selection_end {
        render_cue_input(ui, app, ctx.active_idx, ctx.sel_start, ctx.sel_end, actions);
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_line_click(
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
        handle_goto_def_click(ui, app, line_num, line_text, code_resp, actions);
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
    line_num: usize,
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
        // Also set LSP position for go-to-definition (0-based line and character)
        let character = line_text[..byte_offset.min(line_text.len())]
            .chars()
            .count() as u32;
        actions.lsp_goto_def_position = Some(((line_num - 1) as u32, character));
    }
}

/// Compute the byte offset from a pixel x-offset within a line of text.
pub(crate) fn compute_byte_offset(line_text: &str, x_offset: f32, approx_char_width: f32) -> usize {
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
