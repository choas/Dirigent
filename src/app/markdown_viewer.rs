use std::cell::RefCell;

use eframe::egui;

use super::markdown_parser::{MarkdownBlock, TextSegment};
use super::{DirigentApp, SPACE_MD, SPACE_SM, SPACE_XS};

impl DirigentApp {
    /// Render the parsed markdown blocks for the currently open file.
    pub(super) fn render_markdown_content(&mut self, ui: &mut egui::Ui) {
        let scroll_target = self.viewer.scroll_to_heading.take();
        let blocks_ref = self
            .viewer
            .active()
            .and_then(|t| t.markdown_blocks.as_ref());
        if let Some(blocks) = blocks_ref {
            let mut heading_counter = 0usize;
            let anchor_click: RefCell<Option<String>> = RefCell::new(None);
            let ctx = RenderCtx {
                font_size: self.settings.font_size,
                syntax_theme: &self.viewer.syntax_theme,
                semantic: &self.semantic,
                indent_level: 0,
                anchor_click: &anchor_click,
            };
            render_blocks(ui, blocks, &ctx, scroll_target, &mut heading_counter);

            // If an internal anchor link was clicked, resolve it to a heading index.
            if let Some(anchor) = anchor_click.into_inner() {
                if let Some(idx) = find_heading_index_by_slug(blocks, &anchor) {
                    self.viewer.scroll_to_heading = Some(idx);
                }
            }
        }
    }
}

/// Shared context passed to all block-rendering helpers.
struct RenderCtx<'a> {
    font_size: f32,
    syntax_theme: &'a egui_extras::syntax_highlighting::CodeTheme,
    semantic: &'a crate::settings::SemanticColors,
    indent_level: usize,
    /// Set when an internal `#anchor` link is clicked during rendering.
    anchor_click: &'a RefCell<Option<String>>,
}

impl<'a> RenderCtx<'a> {
    fn indent(&self) -> f32 {
        self.indent_level as f32 * SPACE_MD
    }

    fn nested(&self) -> Self {
        RenderCtx {
            font_size: self.font_size,
            syntax_theme: self.syntax_theme,
            semantic: self.semantic,
            indent_level: self.indent_level + 1,
            anchor_click: self.anchor_click,
        }
    }
}

fn render_blocks(
    ui: &mut egui::Ui,
    blocks: &[MarkdownBlock],
    ctx: &RenderCtx,
    scroll_to_heading: Option<usize>,
    heading_counter: &mut usize,
) {
    let indent = ctx.indent();

    for (block_idx, block) in blocks.iter().enumerate() {
        if indent > 0.0 {
            ui.add_space(0.0); // ensure layout is started
        }
        match block {
            MarkdownBlock::Heading { level, segments } => {
                let should_scroll = scroll_to_heading == Some(*heading_counter);
                *heading_counter += 1;
                render_heading(ui, segments, *level, should_scroll, ctx);
            }
            MarkdownBlock::Paragraph { segments } => {
                render_paragraph(ui, segments, ctx);
            }
            MarkdownBlock::CodeBlock { language, code } => {
                render_code_block(ui, language.as_deref(), code, ctx);
            }
            MarkdownBlock::List {
                ordered,
                start,
                items,
            } => {
                render_list(ui, *ordered, *start, items, ctx, heading_counter);
            }
            MarkdownBlock::BlockQuote { blocks } => {
                render_block_quote(ui, blocks, ctx, heading_counter);
            }
            MarkdownBlock::Table { headers, rows } => {
                render_table(ui, headers, rows, block_idx, ctx);
            }
            MarkdownBlock::ThematicBreak => {
                render_thematic_break(ui, ctx);
            }
            MarkdownBlock::Checkbox { checked, segments } => {
                render_checkbox(ui, *checked, segments, ctx);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Block-type helpers
// ---------------------------------------------------------------------------

fn heading_scale(level: u8) -> f32 {
    match level {
        1 => 2.0,
        2 => 1.6,
        3 => 1.35,
        4 => 1.15,
        5 => 1.05,
        _ => 1.0,
    }
}

fn render_heading(
    ui: &mut egui::Ui,
    segments: &[TextSegment],
    level: u8,
    should_scroll: bool,
    ctx: &RenderCtx,
) {
    let indent = ctx.indent();
    ui.add_space(SPACE_SM);
    let scale = heading_scale(level);
    let resp = ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if indent > 0.0 {
            ui.add_space(indent);
        }
        for seg in segments {
            render_segment(ui, seg, ctx.font_size * scale, true, ctx);
        }
    });
    if should_scroll {
        resp.response.scroll_to_me(Some(egui::Align::TOP));
    }
    if level <= 2 {
        ui.separator();
    }
    ui.add_space(SPACE_XS);
}

fn render_paragraph(ui: &mut egui::Ui, segments: &[TextSegment], ctx: &RenderCtx) {
    let indent = ctx.indent();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if indent > 0.0 {
            ui.add_space(indent);
        }
        for seg in segments {
            render_segment(ui, seg, ctx.font_size, false, ctx);
        }
    });
    ui.add_space(SPACE_SM);
}

fn render_code_block(ui: &mut egui::Ui, language: Option<&str>, code: &str, ctx: &RenderCtx) {
    let indent = ctx.indent();
    ui.add_space(SPACE_XS);
    let frame_fill = code_block_fill(ctx.semantic);
    egui::Frame::new()
        .fill(frame_fill)
        .corner_radius(4.0)
        .inner_margin(SPACE_SM)
        .outer_margin(egui::Margin {
            left: indent.round().min(i8::MAX as f32) as i8,
            ..Default::default()
        })
        .show(ui, |ui| {
            // Header row with optional language label and copy button.
            ui.horizontal(|ui| {
                if let Some(lang) = language {
                    ui.label(
                        egui::RichText::new(lang)
                            .size(ctx.font_size * 0.85)
                            .color(ctx.semantic.secondary_text)
                            .monospace(),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("\u{1F4CB}")
                        .on_hover_text("Copy code")
                        .clicked()
                    {
                        ui.ctx().copy_text(code.to_string());
                    }
                });
            });
            ui.add_space(SPACE_XS);
            render_code_block_body(ui, language, code, ctx);
        });
    ui.add_space(SPACE_SM);
}

fn code_block_fill(semantic: &crate::settings::SemanticColors) -> egui::Color32 {
    if semantic.is_dark() {
        egui::Color32::from_white_alpha(12)
    } else {
        egui::Color32::from_black_alpha(12)
    }
}

fn render_code_block_body(ui: &mut egui::Ui, language: Option<&str>, code: &str, ctx: &RenderCtx) {
    let ext = language.unwrap_or("");
    if !ext.is_empty() {
        for line in code.lines() {
            let job = crate::syntax::highlight(ui.ctx(), ui.style(), ctx.syntax_theme, line, ext);
            ui.label(job);
        }
    } else {
        ui.label(egui::RichText::new(code).monospace().size(ctx.font_size));
    }
}

fn render_list(
    ui: &mut egui::Ui,
    ordered: bool,
    start: Option<u64>,
    items: &[Vec<MarkdownBlock>],
    ctx: &RenderCtx,
    heading_counter: &mut usize,
) {
    let base_num = start.unwrap_or(1);
    for (idx, item_blocks) in items.iter().enumerate() {
        let prefix = list_prefix(ordered, base_num, idx);
        render_list_item(ui, &prefix, item_blocks, ctx, heading_counter);
    }
    ui.add_space(SPACE_XS);
}

fn list_prefix(ordered: bool, base_num: u64, idx: usize) -> String {
    if ordered {
        format!("{}.", base_num + idx as u64)
    } else {
        "\u{2022}".to_string()
    }
}

fn render_list_item(
    ui: &mut egui::Ui,
    prefix: &str,
    item_blocks: &[MarkdownBlock],
    ctx: &RenderCtx,
    heading_counter: &mut usize,
) {
    let first = item_blocks.first();
    let first_is_inline = matches!(
        first,
        Some(MarkdownBlock::Paragraph { .. }) | Some(MarkdownBlock::Checkbox { .. })
    );

    if !first_is_inline {
        render_list_item_fallback(ui, prefix, item_blocks, ctx, heading_counter);
        return;
    }

    let Some(first_block) = first else {
        return;
    };
    render_list_item_inline_first(ui, prefix, first_block, ctx);

    if item_blocks.len() > 1 {
        let nested = ctx.nested();
        render_blocks_with_ctx(ui, &item_blocks[1..], &nested, heading_counter);
    }
}

fn render_list_item_inline_first(
    ui: &mut egui::Ui,
    prefix: &str,
    first_block: &MarkdownBlock,
    ctx: &RenderCtx,
) {
    let indent = ctx.indent();
    match first_block {
        MarkdownBlock::Paragraph { segments } => {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(indent + SPACE_MD);
                ui.label(
                    egui::RichText::new(prefix)
                        .size(ctx.font_size)
                        .color(ctx.semantic.secondary_text),
                );
                ui.add_space(SPACE_XS);
                for seg in segments {
                    render_segment(ui, seg, ctx.font_size, false, ctx);
                }
            });
        }
        MarkdownBlock::Checkbox { checked, segments } => {
            render_checkbox_in_list(ui, prefix, *checked, segments, ctx);
        }
        _ => {}
    }
}

fn render_checkbox_in_list(
    ui: &mut egui::Ui,
    prefix: &str,
    checked: bool,
    segments: &[TextSegment],
    ctx: &RenderCtx,
) {
    let indent = ctx.indent();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(indent + SPACE_MD);
        ui.label(
            egui::RichText::new(prefix)
                .size(ctx.font_size)
                .color(ctx.semantic.secondary_text),
        );
        ui.add_space(SPACE_XS);
        let icon = checkbox_icon(checked);
        let color = checkbox_color(checked, ctx.semantic);
        ui.label(egui::RichText::new(icon).size(ctx.font_size).color(color));
        ui.add_space(SPACE_XS);
        for seg in segments {
            render_segment(ui, seg, ctx.font_size, false, ctx);
        }
    });
}

fn render_list_item_fallback(
    ui: &mut egui::Ui,
    prefix: &str,
    item_blocks: &[MarkdownBlock],
    ctx: &RenderCtx,
    heading_counter: &mut usize,
) {
    let indent = ctx.indent();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(indent + SPACE_MD);
        ui.label(
            egui::RichText::new(prefix)
                .size(ctx.font_size)
                .color(ctx.semantic.secondary_text),
        );
    });
    let nested = ctx.nested();
    render_blocks_with_ctx(ui, item_blocks, &nested, heading_counter);
}

fn render_block_quote(
    ui: &mut egui::Ui,
    blocks: &[MarkdownBlock],
    ctx: &RenderCtx,
    heading_counter: &mut usize,
) {
    let indent = ctx.indent();
    ui.add_space(SPACE_XS);
    let left_margin = indent + SPACE_SM;
    ui.horizontal(|ui| {
        ui.add_space(left_margin);
        let (bar_rect, _) = ui.allocate_exact_size(egui::vec2(3.0, 0.0), egui::Sense::hover());
        let start_y = bar_rect.min.y;

        ui.vertical(|ui| {
            ui.add_space(SPACE_XS);
            render_blocks_with_ctx(
                ui,
                blocks,
                &RenderCtx {
                    font_size: ctx.font_size,
                    syntax_theme: ctx.syntax_theme,
                    semantic: ctx.semantic,
                    indent_level: 0,
                    anchor_click: ctx.anchor_click,
                },
                heading_counter,
            );
            ui.add_space(SPACE_XS);

            let end_y = ui.min_rect().max.y;
            let bar = egui::Rect::from_min_max(
                egui::pos2(bar_rect.min.x, start_y),
                egui::pos2(bar_rect.min.x + 3.0, end_y),
            );
            ui.painter().rect_filled(bar, 1.0, ctx.semantic.accent);
        });
    });
    ui.add_space(SPACE_SM);
}

fn render_table(
    ui: &mut egui::Ui,
    headers: &[Vec<TextSegment>],
    rows: &[Vec<Vec<TextSegment>>],
    _block_idx: usize,
    ctx: &RenderCtx,
) {
    let indent = ctx.indent();
    ui.add_space(SPACE_XS);
    let border = table_border_color(ctx.semantic);
    let header_bg = table_header_bg(ctx.semantic);
    let col_count = headers.len();
    if col_count == 0 {
        return;
    }

    // Pre-compute column widths from actual cell content.
    let frame_overhead = 2.0 + 2.0 * SPACE_SM;
    let col_gap = SPACE_SM * col_count.saturating_sub(1) as f32;
    let available_for_cols =
        (ui.available_width() - indent - frame_overhead - col_gap).max(col_count as f32 * 40.0);
    let col_widths = compute_column_widths(
        ui,
        headers,
        rows,
        col_count,
        ctx.font_size,
        available_for_cols,
    );

    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, border))
        .corner_radius(4.0)
        .outer_margin(egui::Margin {
            left: indent.round().min(i8::MAX as f32) as i8,
            ..Default::default()
        })
        .show(ui, |ui| {
            render_table_header(ui, headers, header_bg, ctx, &col_widths);
            render_table_separator(ui, border);
            render_table_body(ui, rows, ctx, &col_widths);
        });
    ui.add_space(SPACE_SM);
}

fn table_border_color(semantic: &crate::settings::SemanticColors) -> egui::Color32 {
    if semantic.is_dark() {
        egui::Color32::from_white_alpha(30)
    } else {
        egui::Color32::from_black_alpha(30)
    }
}

fn table_header_bg(semantic: &crate::settings::SemanticColors) -> egui::Color32 {
    if semantic.is_dark() {
        egui::Color32::from_white_alpha(15)
    } else {
        egui::Color32::from_black_alpha(10)
    }
}

fn segments_plain_text(segments: &[TextSegment]) -> String {
    segments
        .iter()
        .map(|seg| match seg {
            TextSegment::Plain(t)
            | TextSegment::Bold(t)
            | TextSegment::Italic(t)
            | TextSegment::BoldItalic(t)
            | TextSegment::Code(t)
            | TextSegment::Strikethrough(t)
            | TextSegment::StrikethroughBold(t)
            | TextSegment::StrikethroughItalic(t)
            | TextSegment::StrikethroughBoldItalic(t) => t.as_str(),
            TextSegment::Link { text, .. } => text.as_str(),
            TextSegment::SoftBreak => " ",
            TextSegment::HardBreak => "\n",
        })
        .collect()
}

fn compute_column_widths(
    ui: &egui::Ui,
    headers: &[Vec<TextSegment>],
    rows: &[Vec<Vec<TextSegment>>],
    col_count: usize,
    font_size: f32,
    available: f32,
) -> Vec<f32> {
    let font_id = egui::FontId::proportional(font_size);
    let mut natural = vec![0.0f32; col_count];

    for (i, cell) in headers.iter().enumerate().take(col_count) {
        let text = segments_plain_text(cell);
        let galley = ui
            .painter()
            .layout_no_wrap(text, font_id.clone(), egui::Color32::WHITE);
        natural[i] = natural[i].max(galley.size().x);
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(col_count) {
            let text = segments_plain_text(cell);
            let galley = ui
                .painter()
                .layout_no_wrap(text, font_id.clone(), egui::Color32::WHITE);
            natural[i] = natural[i].max(galley.size().x);
        }
    }

    let total: f32 = natural.iter().sum();
    if total <= available {
        let extra = available - total;
        let share = extra / col_count as f32;
        natural.iter().map(|w| w + share).collect()
    } else {
        let min_width = 40.0f32;
        let total_min = min_width * col_count as f32;
        if available <= total_min {
            vec![available / col_count as f32; col_count]
        } else {
            let distributable = available - total_min;
            natural
                .iter()
                .map(|w| min_width + distributable * (w / total))
                .collect()
        }
    }
}

fn render_table_header(
    ui: &mut egui::Ui,
    headers: &[Vec<TextSegment>],
    header_bg: egui::Color32,
    ctx: &RenderCtx,
    col_widths: &[f32],
) {
    egui::Frame::new()
        .fill(header_bg)
        .inner_margin(egui::Margin::symmetric(SPACE_SM as i8, SPACE_XS as i8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, cell) in headers.iter().enumerate() {
                    let width = col_widths.get(i).copied().unwrap_or(60.0);
                    ui.vertical(|ui| {
                        ui.set_min_width(width);
                        ui.set_max_width(width);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            for seg in cell {
                                render_segment(ui, seg, ctx.font_size, true, ctx);
                            }
                        });
                    });
                }
            });
        });
}

fn render_table_separator(ui: &mut egui::Ui, border: egui::Color32) {
    let rect = ui.available_rect_before_wrap();
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.top()),
            egui::pos2(rect.right(), rect.top()),
        ],
        egui::Stroke::new(1.0, border),
    );
}

fn stripe_bg_color(is_dark: bool) -> egui::Color32 {
    if is_dark {
        egui::Color32::from_white_alpha(6)
    } else {
        egui::Color32::from_black_alpha(6)
    }
}

fn row_fill(row_idx: usize, stripe_bg: egui::Color32) -> egui::Color32 {
    if row_idx % 2 == 1 {
        stripe_bg
    } else {
        egui::Color32::TRANSPARENT
    }
}

fn render_table_body(
    ui: &mut egui::Ui,
    rows: &[Vec<Vec<TextSegment>>],
    ctx: &RenderCtx,
    col_widths: &[f32],
) {
    let stripe_bg = stripe_bg_color(ctx.semantic.is_dark());

    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(SPACE_SM as i8, SPACE_XS as i8))
        .show(ui, |ui| {
            for (row_idx, row) in rows.iter().enumerate() {
                render_table_row(ui, row, row_fill(row_idx, stripe_bg), ctx, col_widths);
            }
        });
}

fn render_table_row(
    ui: &mut egui::Ui,
    row: &[Vec<TextSegment>],
    fill: egui::Color32,
    ctx: &RenderCtx,
    col_widths: &[f32],
) {
    let empty_cell: Vec<TextSegment> = Vec::new();
    egui::Frame::new().fill(fill).show(ui, |ui| {
        ui.horizontal(|ui| {
            for (i, &width) in col_widths.iter().enumerate() {
                let cell = row.get(i).unwrap_or(&empty_cell);
                ui.vertical(|ui| {
                    ui.set_min_width(width);
                    ui.set_max_width(width);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for seg in cell {
                            render_segment(ui, seg, ctx.font_size, false, ctx);
                        }
                    });
                });
            }
        });
    });
}

fn render_thematic_break(ui: &mut egui::Ui, ctx: &RenderCtx) {
    let indent = ctx.indent();
    ui.add_space(SPACE_SM);
    if indent > 0.0 {
        ui.horizontal(|ui| {
            ui.add_space(indent);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.separator();
            });
        });
    } else {
        ui.separator();
    }
    ui.add_space(SPACE_SM);
}

fn render_checkbox(ui: &mut egui::Ui, checked: bool, segments: &[TextSegment], ctx: &RenderCtx) {
    let indent = ctx.indent();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(indent + SPACE_MD);
        let icon = checkbox_icon(checked);
        let color = checkbox_color(checked, ctx.semantic);
        ui.label(egui::RichText::new(icon).size(ctx.font_size).color(color));
        ui.add_space(SPACE_XS);
        for seg in segments {
            render_segment(ui, seg, ctx.font_size, false, ctx);
        }
    });
    ui.add_space(SPACE_XS);
}

// ---------------------------------------------------------------------------
// Small shared helpers
// ---------------------------------------------------------------------------

fn checkbox_icon(checked: bool) -> &'static str {
    if checked {
        "\u{2611}"
    } else {
        "\u{2610}"
    }
}

fn checkbox_color(checked: bool, semantic: &crate::settings::SemanticColors) -> egui::Color32 {
    if checked {
        semantic.success
    } else {
        semantic.secondary_text
    }
}

/// Render blocks using a pre-built `RenderCtx` (no scroll-to-heading support).
fn render_blocks_with_ctx(
    ui: &mut egui::Ui,
    blocks: &[MarkdownBlock],
    ctx: &RenderCtx,
    heading_counter: &mut usize,
) {
    render_blocks(ui, blocks, ctx, None, heading_counter);
}

// ---------------------------------------------------------------------------
// Inline segment renderer (unchanged)
// ---------------------------------------------------------------------------

fn render_segment(
    ui: &mut egui::Ui,
    seg: &TextSegment,
    font_size: f32,
    heading_bold: bool,
    ctx: &RenderCtx,
) {
    let semantic = ctx.semantic;
    match seg {
        TextSegment::Plain(t) => {
            let mut rt = egui::RichText::new(t).size(font_size);
            if heading_bold {
                rt = rt.strong();
            }
            ui.label(rt);
        }
        TextSegment::Bold(t) => {
            ui.label(egui::RichText::new(t).size(font_size).strong());
        }
        TextSegment::Italic(t) => {
            ui.label(egui::RichText::new(t).size(font_size).italics());
        }
        TextSegment::BoldItalic(t) => {
            ui.label(egui::RichText::new(t).size(font_size).strong().italics());
        }
        TextSegment::Code(t) => {
            let bg = if semantic.is_dark() {
                egui::Color32::from_white_alpha(20)
            } else {
                egui::Color32::from_black_alpha(15)
            };
            let rt = egui::RichText::new(t)
                .size(font_size)
                .monospace()
                .background_color(bg);
            ui.label(rt);
        }
        TextSegment::Link { text, url } => {
            if let Some(anchor) = url.strip_prefix('#') {
                // Internal anchor link — render as clickable text that scrolls to heading.
                let resp = ui.link(
                    egui::RichText::new(text)
                        .size(font_size)
                        .color(semantic.accent),
                );
                if resp.clicked() {
                    *ctx.anchor_click.borrow_mut() = Some(anchor.to_string());
                }
            } else {
                ui.hyperlink_to(
                    egui::RichText::new(text)
                        .size(font_size)
                        .color(semantic.accent),
                    url,
                );
            }
        }
        TextSegment::Strikethrough(t) => {
            ui.label(egui::RichText::new(t).size(font_size).strikethrough());
        }
        TextSegment::StrikethroughBold(t) => {
            ui.label(
                egui::RichText::new(t)
                    .size(font_size)
                    .strong()
                    .strikethrough(),
            );
        }
        TextSegment::StrikethroughItalic(t) => {
            ui.label(
                egui::RichText::new(t)
                    .size(font_size)
                    .italics()
                    .strikethrough(),
            );
        }
        TextSegment::StrikethroughBoldItalic(t) => {
            ui.label(
                egui::RichText::new(t)
                    .size(font_size)
                    .strong()
                    .italics()
                    .strikethrough(),
            );
        }
        TextSegment::SoftBreak => {
            ui.label(" ");
        }
        TextSegment::HardBreak => {
            ui.end_row();
        }
    }
}

// ---------------------------------------------------------------------------
// Internal anchor link helpers
// ---------------------------------------------------------------------------

/// Convert heading text into a GitHub-style anchor slug.
/// Lowercase, spaces → hyphens, strip non-alphanumeric (except hyphens).
fn heading_to_slug(segments: &[TextSegment]) -> String {
    let plain = segments_plain_text(segments);
    plain
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else if c == ' ' || c == '-' {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

/// Walk all blocks (recursively) and find the 0-based heading index whose slug matches `anchor`.
/// Handles duplicate headings using GitHub-style disambiguation: the first occurrence of a slug
/// is bare, the second gets `-1`, the third `-2`, etc.
fn find_heading_index_by_slug(blocks: &[MarkdownBlock], anchor: &str) -> Option<usize> {
    let mut counter = 0usize;
    let mut slug_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    find_heading_in_blocks(blocks, anchor, &mut counter, &mut slug_counts)
}

fn find_heading_in_blocks(
    blocks: &[MarkdownBlock],
    anchor: &str,
    counter: &mut usize,
    slug_counts: &mut std::collections::HashMap<String, usize>,
) -> Option<usize> {
    for block in blocks {
        match block {
            MarkdownBlock::Heading { segments, .. } => {
                let base_slug = heading_to_slug(segments);
                let occurrence = slug_counts.entry(base_slug.clone()).or_insert(0);
                let actual_slug = if *occurrence == 0 {
                    base_slug
                } else {
                    format!("{}-{}", base_slug, occurrence)
                };
                *occurrence += 1;
                if actual_slug == anchor {
                    return Some(*counter);
                }
                *counter += 1;
            }
            MarkdownBlock::List { items, .. } => {
                for item in items {
                    if let Some(idx) = find_heading_in_blocks(item, anchor, counter, slug_counts) {
                        return Some(idx);
                    }
                }
            }
            MarkdownBlock::BlockQuote { blocks } => {
                if let Some(idx) = find_heading_in_blocks(blocks, anchor, counter, slug_counts) {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}
