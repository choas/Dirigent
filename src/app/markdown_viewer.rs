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
            let ctx = RenderCtx {
                font_size: self.settings.font_size,
                syntax_theme: &self.viewer.syntax_theme,
                semantic: &self.semantic,
                indent_level: 0,
            };
            render_blocks(ui, blocks, &ctx, scroll_target, &mut heading_counter);
        }
    }
}

/// Shared context passed to all block-rendering helpers.
struct RenderCtx<'a> {
    font_size: f32,
    syntax_theme: &'a egui_extras::syntax_highlighting::CodeTheme,
    semantic: &'a crate::settings::SemanticColors,
    indent_level: usize,
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
        if indent > 0.0 {
            ui.add_space(indent);
        }
        for seg in segments {
            render_segment(ui, seg, ctx.font_size * scale, true, ctx.semantic);
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
        if indent > 0.0 {
            ui.add_space(indent);
        }
        for seg in segments {
            render_segment(ui, seg, ctx.font_size, false, ctx.semantic);
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
            let job = egui_extras::syntax_highlighting::highlight(
                ui.ctx(),
                ui.style(),
                ctx.syntax_theme,
                line,
                ext,
            );
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

    render_list_item_inline_first(ui, prefix, first.unwrap(), ctx);

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
                ui.add_space(indent + SPACE_MD);
                ui.label(
                    egui::RichText::new(prefix)
                        .size(ctx.font_size)
                        .color(ctx.semantic.secondary_text),
                );
                for seg in segments {
                    render_segment(ui, seg, ctx.font_size, false, ctx.semantic);
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
        ui.add_space(indent + SPACE_MD);
        ui.label(
            egui::RichText::new(prefix)
                .size(ctx.font_size)
                .color(ctx.semantic.secondary_text),
        );
        let icon = checkbox_icon(checked);
        let color = checkbox_color(checked, ctx.semantic);
        ui.label(egui::RichText::new(icon).size(ctx.font_size).color(color));
        for seg in segments {
            render_segment(ui, seg, ctx.font_size, false, ctx.semantic);
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
    block_idx: usize,
    ctx: &RenderCtx,
) {
    let indent = ctx.indent();
    ui.add_space(SPACE_XS);
    let border = table_border_color(ctx.semantic);
    let header_bg = table_header_bg(ctx.semantic);
    let col_count = headers.len();

    ui.horizontal_wrapped(|ui| {
        if indent > 0.0 {
            ui.add_space(indent);
        }
        egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, border))
            .corner_radius(4.0)
            .show(ui, |ui| {
                render_table_header(ui, headers, col_count, block_idx, header_bg, ctx);
                render_table_separator(ui, border);
                render_table_body(ui, rows, col_count, block_idx, ctx);
            });
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

fn render_table_header(
    ui: &mut egui::Ui,
    headers: &[Vec<TextSegment>],
    col_count: usize,
    block_idx: usize,
    header_bg: egui::Color32,
    ctx: &RenderCtx,
) {
    egui::Frame::new()
        .fill(header_bg)
        .inner_margin(egui::Margin::symmetric(SPACE_SM as i8, SPACE_XS as i8))
        .show(ui, |ui| {
            egui::Grid::new(ui.id().with(("md_th", block_idx)))
                .num_columns(col_count)
                .min_col_width(60.0)
                .spacing(egui::vec2(SPACE_MD, SPACE_XS))
                .show(ui, |ui| {
                    for cell in headers {
                        ui.horizontal_wrapped(|ui| {
                            for seg in cell {
                                render_segment(ui, seg, ctx.font_size, true, ctx.semantic);
                            }
                        });
                    }
                    ui.end_row();
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

fn render_table_body(
    ui: &mut egui::Ui,
    rows: &[Vec<Vec<TextSegment>>],
    col_count: usize,
    block_idx: usize,
    ctx: &RenderCtx,
) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(SPACE_SM as i8, SPACE_XS as i8))
        .show(ui, |ui| {
            egui::Grid::new(ui.id().with(("md_td", block_idx)))
                .num_columns(col_count)
                .striped(true)
                .min_col_width(60.0)
                .spacing(egui::vec2(SPACE_MD, SPACE_XS))
                .show(ui, |ui| {
                    for row in rows {
                        for cell in row {
                            ui.horizontal_wrapped(|ui| {
                                for seg in cell {
                                    render_segment(ui, seg, ctx.font_size, false, ctx.semantic);
                                }
                            });
                        }
                        ui.end_row();
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
        ui.add_space(indent + SPACE_MD);
        let icon = checkbox_icon(checked);
        let color = checkbox_color(checked, ctx.semantic);
        ui.label(egui::RichText::new(icon).size(ctx.font_size).color(color));
        for seg in segments {
            render_segment(ui, seg, ctx.font_size, false, ctx.semantic);
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
    semantic: &crate::settings::SemanticColors,
) {
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
            ui.hyperlink_to(
                egui::RichText::new(text)
                    .size(font_size)
                    .color(semantic.accent),
                url,
            );
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
