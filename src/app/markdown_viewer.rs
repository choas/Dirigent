use eframe::egui;

use super::markdown_parser::{MarkdownBlock, TextSegment};
use super::{DirigentApp, SPACE_MD, SPACE_SM, SPACE_XS};

impl DirigentApp {
    /// Render the parsed markdown blocks for the currently open file.
    pub(super) fn render_markdown_content(&self, ui: &mut egui::Ui) {
        if let Some(ref blocks) = self.viewer.markdown_blocks {
            render_blocks(
                ui,
                blocks,
                self.settings.font_size,
                &self.viewer.syntax_theme,
                &self.semantic,
                0,
            );
        }
    }
}

fn render_blocks(
    ui: &mut egui::Ui,
    blocks: &[MarkdownBlock],
    font_size: f32,
    syntax_theme: &egui_extras::syntax_highlighting::CodeTheme,
    semantic: &crate::settings::SemanticColors,
    indent_level: usize,
) {
    let indent = indent_level as f32 * SPACE_MD;

    for (block_idx, block) in blocks.iter().enumerate() {
        if indent > 0.0 {
            ui.add_space(0.0); // ensure layout is started
        }
        match block {
            MarkdownBlock::Heading { level, segments } => {
                ui.add_space(SPACE_SM);
                let scale = match level {
                    1 => 2.0,
                    2 => 1.6,
                    3 => 1.35,
                    4 => 1.15,
                    5 => 1.05,
                    _ => 1.0,
                };
                ui.horizontal_wrapped(|ui| {
                    if indent > 0.0 {
                        ui.add_space(indent);
                    }
                    for seg in segments {
                        render_segment(ui, seg, font_size * scale, true, semantic);
                    }
                });
                if *level <= 2 {
                    ui.separator();
                }
                ui.add_space(SPACE_XS);
            }
            MarkdownBlock::Paragraph { segments } => {
                ui.horizontal_wrapped(|ui| {
                    if indent > 0.0 {
                        ui.add_space(indent);
                    }
                    for seg in segments {
                        render_segment(ui, seg, font_size, false, semantic);
                    }
                });
                ui.add_space(SPACE_SM);
            }
            MarkdownBlock::CodeBlock { language, code } => {
                ui.add_space(SPACE_XS);
                let frame_fill = if semantic.is_dark() {
                    egui::Color32::from_white_alpha(12)
                } else {
                    egui::Color32::from_black_alpha(12)
                };
                egui::Frame::new()
                    .fill(frame_fill)
                    .corner_radius(4.0)
                    .inner_margin(SPACE_SM)
                    .outer_margin(egui::Margin {
                        left: indent as i8,
                        ..Default::default()
                    })
                    .show(ui, |ui| {
                        let ext = language.as_deref().unwrap_or("");
                        if !ext.is_empty() {
                            // Use syntect highlighting for known languages
                            for line in code.lines() {
                                let job = egui_extras::syntax_highlighting::highlight(
                                    ui.ctx(),
                                    ui.style(),
                                    syntax_theme,
                                    line,
                                    ext,
                                );
                                ui.label(job);
                            }
                        } else {
                            ui.label(
                                egui::RichText::new(code.as_str())
                                    .monospace()
                                    .size(font_size),
                            );
                        }
                    });
                ui.add_space(SPACE_SM);
            }
            MarkdownBlock::List {
                ordered,
                start,
                items,
            } => {
                let base_num = start.unwrap_or(1);
                for (idx, item_blocks) in items.iter().enumerate() {
                    let prefix = if *ordered {
                        format!("{}.", base_num + idx as u64)
                    } else {
                        "\u{2022}".to_string() // bullet
                    };
                    // Render bullet + first paragraph inline on the same line
                    let first_is_paragraph = matches!(
                        item_blocks.first(),
                        Some(MarkdownBlock::Paragraph { .. })
                            | Some(MarkdownBlock::Checkbox { .. })
                    );
                    if first_is_paragraph {
                        match item_blocks.first() {
                            Some(MarkdownBlock::Paragraph { segments }) => {
                                ui.horizontal_wrapped(|ui| {
                                    ui.add_space(indent + SPACE_MD);
                                    ui.label(
                                        egui::RichText::new(&prefix)
                                            .size(font_size)
                                            .color(semantic.secondary_text),
                                    );
                                    for seg in segments {
                                        render_segment(ui, seg, font_size, false, semantic);
                                    }
                                });
                            }
                            Some(MarkdownBlock::Checkbox { checked, segments }) => {
                                ui.horizontal_wrapped(|ui| {
                                    ui.add_space(indent + SPACE_MD);
                                    let icon = if *checked { "\u{2611}" } else { "\u{2610}" };
                                    ui.label(egui::RichText::new(icon).size(font_size).color(
                                        if *checked {
                                            semantic.success
                                        } else {
                                            semantic.secondary_text
                                        },
                                    ));
                                    for seg in segments {
                                        render_segment(ui, seg, font_size, false, semantic);
                                    }
                                });
                            }
                            _ => {}
                        }
                        // Render remaining blocks with indent
                        if item_blocks.len() > 1 {
                            render_blocks(
                                ui,
                                &item_blocks[1..],
                                font_size,
                                syntax_theme,
                                semantic,
                                indent_level + 1,
                            );
                        }
                    } else {
                        // Fallback: bullet on its own, content below
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space(indent + SPACE_MD);
                            ui.label(
                                egui::RichText::new(&prefix)
                                    .size(font_size)
                                    .color(semantic.secondary_text),
                            );
                        });
                        render_blocks(
                            ui,
                            item_blocks,
                            font_size,
                            syntax_theme,
                            semantic,
                            indent_level + 1,
                        );
                    }
                }
                ui.add_space(SPACE_XS);
            }
            MarkdownBlock::BlockQuote { blocks } => {
                ui.add_space(SPACE_XS);
                let left_margin = indent + SPACE_SM;
                // Draw a colored left border using a frame
                ui.horizontal(|ui| {
                    ui.add_space(left_margin);
                    // Vertical accent bar
                    let (bar_rect, _) = ui.allocate_exact_size(
                        egui::vec2(3.0, 0.0), // width; height will be determined by content
                        egui::Sense::hover(),
                    );
                    // We'll paint the bar after knowing the height
                    let start_y = bar_rect.min.y;

                    ui.vertical(|ui| {
                        ui.add_space(SPACE_XS);
                        render_blocks(ui, blocks, font_size, syntax_theme, semantic, 0);
                        ui.add_space(SPACE_XS);

                        // Paint the accent bar spanning the full content height
                        let end_y = ui.min_rect().max.y;
                        let bar = egui::Rect::from_min_max(
                            egui::pos2(bar_rect.min.x, start_y),
                            egui::pos2(bar_rect.min.x + 3.0, end_y),
                        );
                        ui.painter().rect_filled(bar, 1.0, semantic.accent);
                    });
                });
                ui.add_space(SPACE_SM);
            }
            MarkdownBlock::Table { headers, rows } => {
                ui.add_space(SPACE_XS);
                let border = if semantic.is_dark() {
                    egui::Color32::from_white_alpha(30)
                } else {
                    egui::Color32::from_black_alpha(30)
                };
                egui::Frame::new()
                    .stroke(egui::Stroke::new(1.0, border))
                    .corner_radius(2.0)
                    .outer_margin(egui::Margin {
                        left: indent as i8,
                        ..Default::default()
                    })
                    .show(ui, |ui| {
                        egui::Grid::new(ui.id().with(("md_table", block_idx)))
                            .striped(true)
                            .min_col_width(40.0)
                            .show(ui, |ui| {
                                // Header row
                                for cell in headers {
                                    ui.horizontal_wrapped(|ui| {
                                        for seg in cell {
                                            render_segment(ui, seg, font_size, true, semantic);
                                        }
                                    });
                                }
                                ui.end_row();
                                // Data rows
                                for row in rows {
                                    for cell in row {
                                        ui.horizontal_wrapped(|ui| {
                                            for seg in cell {
                                                render_segment(ui, seg, font_size, false, semantic);
                                            }
                                        });
                                    }
                                    ui.end_row();
                                }
                            });
                    });
                ui.add_space(SPACE_SM);
            }
            MarkdownBlock::ThematicBreak => {
                ui.add_space(SPACE_SM);
                ui.separator();
                ui.add_space(SPACE_SM);
            }
            MarkdownBlock::Checkbox { checked, segments } => {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(indent + SPACE_MD);
                    let icon = if *checked { "\u{2611}" } else { "\u{2610}" };
                    ui.label(
                        egui::RichText::new(icon)
                            .size(font_size)
                            .color(if *checked {
                                semantic.success
                            } else {
                                semantic.secondary_text
                            }),
                    );
                    for seg in segments {
                        render_segment(ui, seg, font_size, false, semantic);
                    }
                });
                ui.add_space(SPACE_XS);
            }
        }
    }
}

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
        TextSegment::SoftBreak => {
            ui.label(" ");
        }
        TextSegment::HardBreak => {
            ui.end_row();
        }
    }
}
