use std::collections::HashSet;

use eframe::egui;

use crate::app::{SPACE_SM, SPACE_XS};
use crate::settings::SemanticColors;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DiffViewMode {
    Inline,
    SideBySide,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedDiff {
    pub files: Vec<FileDiff>,
}

#[derive(Debug, Clone)]
pub(crate) struct FileDiff {
    pub new_path: String,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone)]
pub(crate) struct DiffHunk {
    pub new_start: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone)]
pub(crate) struct DiffLine {
    pub kind: DiffLineKind,
    pub old_lineno: Option<usize>,
    pub new_lineno: Option<usize>,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DiffLineKind {
    Context,
    Addition,
    Deletion,
}

pub(crate) fn parse_unified_diff(diff_text: &str) -> ParsedDiff {
    let mut files = Vec::new();
    let lines: Vec<&str> = diff_text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        if lines[i].starts_with("--- ") && i + 1 < lines.len() && lines[i + 1].starts_with("+++ ") {
            // Skip the "--- a/..." line (old path not needed).
            let new_path = lines[i + 1]
                .strip_prefix("+++ b/")
                .or_else(|| lines[i + 1].strip_prefix("+++ "))
                .unwrap_or("")
                .to_string();
            i += 2;

            let mut hunks = Vec::new();
            while i < lines.len() && !lines[i].starts_with("--- ") {
                if lines[i].starts_with("@@ ") {
                    let (old_start, new_start) = parse_hunk_header(lines[i]);
                    i += 1;

                    let mut hunk_lines = Vec::new();
                    let mut old_line = old_start;
                    let mut new_line = new_start;

                    while i < lines.len() {
                        let line = lines[i];
                        if line.starts_with("@@ ") || line.starts_with("--- ") {
                            break;
                        }
                        if line.starts_with('+') {
                            hunk_lines.push(DiffLine {
                                kind: DiffLineKind::Addition,
                                old_lineno: None,
                                new_lineno: Some(new_line),
                                content: line[1..].to_string(),
                            });
                            new_line += 1;
                        } else if line.starts_with('-') {
                            hunk_lines.push(DiffLine {
                                kind: DiffLineKind::Deletion,
                                old_lineno: Some(old_line),
                                new_lineno: None,
                                content: line[1..].to_string(),
                            });
                            old_line += 1;
                        } else if line.starts_with(' ') || line.is_empty() {
                            let content = if line.is_empty() { "" } else { &line[1..] };
                            hunk_lines.push(DiffLine {
                                kind: DiffLineKind::Context,
                                old_lineno: Some(old_line),
                                new_lineno: Some(new_line),
                                content: content.to_string(),
                            });
                            old_line += 1;
                            new_line += 1;
                        } else {
                            break;
                        }
                        i += 1;
                    }

                    hunks.push(DiffHunk {
                        new_start,
                        lines: hunk_lines,
                    });
                } else {
                    i += 1;
                }
            }

            files.push(FileDiff { new_path, hunks });
        } else {
            i += 1;
        }
    }

    ParsedDiff { files }
}

fn parse_hunk_header(header: &str) -> (usize, usize) {
    let inner = header.strip_prefix("@@ ").unwrap_or(header);
    let range_part = if let Some(pos) = inner.find(" @@") {
        &inner[..pos]
    } else {
        inner
    };
    let parts: Vec<&str> = range_part.split_whitespace().collect();
    let old_start = parts
        .first()
        .and_then(|p| p.strip_prefix('-'))
        .and_then(|p| p.split(',').next())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1);
    let new_start = parts
        .get(1)
        .and_then(|p| p.strip_prefix('+'))
        .and_then(|p| p.split(',').next())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1);
    (old_start, new_start)
}

/// Optional search highlight state for diff rendering.
pub(crate) struct DiffSearchHighlight<'a> {
    pub query_lower: &'a str,
    /// The current match to scroll to: (file_idx, hunk_idx, line_idx).
    pub current: Option<(usize, usize, usize)>,
}

pub(crate) fn render_inline_diff(
    ui: &mut egui::Ui,
    diff: &ParsedDiff,
    collapsed_files: &mut HashSet<usize>,
    search: Option<&DiffSearchHighlight<'_>>,
    colors: &SemanticColors,
) {
    let green_bg = colors.addition_bg();
    let red_bg = colors.deletion_bg();
    let green_text = colors.success;
    let red_text = colors.danger;
    let context_text = colors.secondary_text;
    let gutter_color = colors.tertiary_text;

    for (file_idx, file) in diff.files.iter().enumerate() {
        let is_collapsed = collapsed_files.contains(&file_idx);
        let arrow = if is_collapsed { "\u{25B6}" } else { "\u{25BC}" };
        let additions: usize = file
            .hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == DiffLineKind::Addition)
            .count();
        let deletions: usize = file
            .hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == DiffLineKind::Deletion)
            .count();
        let stats = format!(" +{} -{}", additions, deletions);

        if ui
            .add(
                egui::Label::new(
                    egui::RichText::new(format!("{} {}{}", arrow, file.new_path, stats))
                        .strong()
                        .color(colors.diff_header()),
                )
                .sense(egui::Sense::click()),
            )
            .clicked()
        {
            if is_collapsed {
                collapsed_files.remove(&file_idx);
            } else {
                collapsed_files.insert(file_idx);
            }
        }

        if !is_collapsed {
            ui.add_space(SPACE_XS);

            for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
                for (line_idx, line) in hunk.lines.iter().enumerate() {
                    let old_num = line
                        .old_lineno
                        .map(|n| format!("{:>4}", n))
                        .unwrap_or_else(|| "    ".to_string());
                    let new_num = line
                        .new_lineno
                        .map(|n| format!("{:>4}", n))
                        .unwrap_or_else(|| "    ".to_string());
                    let prefix = match line.kind {
                        DiffLineKind::Addition => "+",
                        DiffLineKind::Deletion => "-",
                        DiffLineKind::Context => " ",
                    };
                    let (text_color, bg_color) = match line.kind {
                        DiffLineKind::Addition => (green_text, Some(green_bg)),
                        DiffLineKind::Deletion => (red_text, Some(red_bg)),
                        DiffLineKind::Context => (context_text, None),
                    };

                    let is_search_match = search
                        .as_ref()
                        .map(|s| {
                            !s.query_lower.is_empty()
                                && line.content.to_lowercase().contains(s.query_lower)
                        })
                        .unwrap_or(false);
                    let is_current_match = search
                        .as_ref()
                        .and_then(|s| s.current)
                        .map(|c| c == (file_idx, hunk_idx, line_idx))
                        .unwrap_or(false);

                    let response = ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{} {} {}", old_num, new_num, prefix))
                                .monospace()
                                .color(gutter_color),
                        );
                        if is_search_match {
                            render_highlighted_text(
                                ui,
                                &line.content,
                                search.unwrap().query_lower,
                                text_color,
                                colors,
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(&line.content)
                                    .monospace()
                                    .color(text_color),
                            );
                        }
                    });

                    if is_current_match {
                        response.response.scroll_to_me(Some(egui::Align::Center));
                    }

                    let effective_bg = if is_current_match {
                        Some(colors.search_current_bg())
                    } else if is_search_match {
                        Some(colors.search_match_bg())
                    } else {
                        bg_color
                    };
                    if let Some(bg) = effective_bg {
                        ui.painter().rect_filled(response.response.rect, 0.0, bg);
                    }
                }
                ui.add_space(SPACE_SM);
            }
        }

        ui.separator();
    }
}

/// Render text with search query highlighted.
fn render_highlighted_text(
    ui: &mut egui::Ui,
    text: &str,
    query_lower: &str,
    base_color: egui::Color32,
    colors: &SemanticColors,
) {
    let highlight_bg = colors.search_highlight_bg();
    let highlight_fg = colors.accent_text();
    let text_lower = text.to_lowercase();
    let mut pos = 0;

    // Lay out segments in a single horizontal flow
    while pos < text.len() {
        if let Some(match_start) = text_lower[pos..].find(query_lower) {
            let abs_start = pos + match_start;
            let abs_end = abs_start + query_lower.len();
            // Text before match
            if abs_start > pos {
                ui.label(
                    egui::RichText::new(&text[pos..abs_start])
                        .monospace()
                        .color(base_color),
                );
            }
            // Matched text
            let resp = ui.label(
                egui::RichText::new(&text[abs_start..abs_end])
                    .monospace()
                    .color(highlight_fg)
                    .background_color(highlight_bg),
            );
            // Paint highlight background behind the label
            ui.painter().rect_filled(resp.rect, 2.0, highlight_bg);
            // Re-paint text on top so it's visible above the rect
            let galley = ui.painter().layout_no_wrap(
                text[abs_start..abs_end].to_string(),
                egui::FontId::monospace(ui.text_style_height(&egui::TextStyle::Monospace)),
                highlight_fg,
            );
            ui.painter().galley(resp.rect.min, galley, highlight_fg);
            pos = abs_end;
        } else {
            // Remainder
            ui.label(
                egui::RichText::new(&text[pos..])
                    .monospace()
                    .color(base_color),
            );
            break;
        }
    }
}

pub(crate) fn render_side_by_side_diff(
    ui: &mut egui::Ui,
    diff: &ParsedDiff,
    collapsed_files: &mut HashSet<usize>,
    search: Option<&DiffSearchHighlight<'_>>,
    colors: &SemanticColors,
) {
    let green_bg = colors.addition_bg();
    let red_bg = colors.deletion_bg();
    let green_text = colors.success;
    let red_text = colors.danger;
    let context_text = colors.secondary_text;
    let gutter_color = colors.tertiary_text;
    let sep_color = colors.separator;

    for (file_idx, file) in diff.files.iter().enumerate() {
        let is_collapsed = collapsed_files.contains(&file_idx);
        let arrow = if is_collapsed { "\u{25B6}" } else { "\u{25BC}" };
        let additions: usize = file
            .hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == DiffLineKind::Addition)
            .count();
        let deletions: usize = file
            .hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| l.kind == DiffLineKind::Deletion)
            .count();
        let stats = format!(" +{} -{}", additions, deletions);

        if ui
            .add(
                egui::Label::new(
                    egui::RichText::new(format!("{} {}{}", arrow, file.new_path, stats))
                        .strong()
                        .color(colors.diff_header()),
                )
                .sense(egui::Sense::click()),
            )
            .clicked()
        {
            if is_collapsed {
                collapsed_files.remove(&file_idx);
            } else {
                collapsed_files.insert(file_idx);
            }
        }

        if is_collapsed {
            ui.separator();
            continue;
        }

        ui.add_space(SPACE_XS);

        for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
            let pairs = build_side_by_side_pairs(&hunk.lines);

            egui::Grid::new(format!(
                "sbs_{}_{}_{}",
                file.new_path, hunk_idx, hunk.new_start
            ))
            .num_columns(5)
            .spacing([SPACE_XS, 0.0])
            .min_col_width(0.0)
            .show(ui, |ui| {
                let current_match = search.as_ref().and_then(|s| s.current);
                let query_lower = search.as_ref().map(|s| s.query_lower).unwrap_or("");

                for (left, right) in &pairs {
                    // Check if either side is the current search match
                    let left_is_current = left
                        .as_ref()
                        .map(|(idx, _)| current_match == Some((file_idx, hunk_idx, *idx)))
                        .unwrap_or(false);
                    let right_is_current = right
                        .as_ref()
                        .map(|(idx, _)| current_match == Some((file_idx, hunk_idx, *idx)))
                        .unwrap_or(false);
                    let is_current = left_is_current || right_is_current;

                    // Old line number
                    if let Some((_, ref line)) = left {
                        ui.label(
                            egui::RichText::new(format!("{:>4}", line.old_lineno.unwrap_or(0)))
                                .monospace()
                                .color(gutter_color),
                        );
                    } else {
                        ui.label(egui::RichText::new("    ").monospace().color(gutter_color));
                    }

                    // Old content
                    if let Some((_, ref line)) = left {
                        let (color, bg) = match line.kind {
                            DiffLineKind::Deletion => (red_text, Some(red_bg)),
                            _ => (context_text, None),
                        };
                        let is_match = !query_lower.is_empty()
                            && line.content.to_lowercase().contains(query_lower);
                        let resp =
                            ui.label(egui::RichText::new(&line.content).monospace().color(color));
                        let effective_bg = if left_is_current {
                            Some(colors.search_current_bg())
                        } else if is_match {
                            Some(colors.search_match_bg())
                        } else {
                            bg
                        };
                        if let Some(bg) = effective_bg {
                            ui.painter().rect_filled(resp.rect, 0.0, bg);
                        }
                    } else {
                        ui.label(egui::RichText::new(" ").monospace());
                    }

                    // Separator
                    let sep_resp =
                        ui.label(egui::RichText::new("\u{2502}").monospace().color(sep_color));
                    if is_current {
                        sep_resp.scroll_to_me(Some(egui::Align::Center));
                    }

                    // New line number
                    if let Some((_, ref line)) = right {
                        ui.label(
                            egui::RichText::new(format!("{:>4}", line.new_lineno.unwrap_or(0)))
                                .monospace()
                                .color(gutter_color),
                        );
                    } else {
                        ui.label(egui::RichText::new("    ").monospace().color(gutter_color));
                    }

                    // New content
                    if let Some((_, ref line)) = right {
                        let (color, bg) = match line.kind {
                            DiffLineKind::Addition => (green_text, Some(green_bg)),
                            _ => (context_text, None),
                        };
                        let is_match = !query_lower.is_empty()
                            && line.content.to_lowercase().contains(query_lower);
                        let resp =
                            ui.label(egui::RichText::new(&line.content).monospace().color(color));
                        let effective_bg = if right_is_current {
                            Some(colors.search_current_bg())
                        } else if is_match {
                            Some(colors.search_match_bg())
                        } else {
                            bg
                        };
                        if let Some(bg) = effective_bg {
                            ui.painter().rect_filled(resp.rect, 0.0, bg);
                        }
                    } else {
                        ui.label(egui::RichText::new(" ").monospace());
                    }

                    ui.end_row();
                }
            });

            ui.add_space(SPACE_SM);
        }

        ui.separator();
    }
}

/// A side-by-side pair with optional original line indices.
type SbsPair = (Option<(usize, DiffLine)>, Option<(usize, DiffLine)>);

/// Build paired (old, new) lines for side-by-side rendering.
/// Each entry carries the original index into the hunk's lines vec.
fn build_side_by_side_pairs(lines: &[DiffLine]) -> Vec<SbsPair> {
    let mut pairs: Vec<SbsPair> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        match lines[i].kind {
            DiffLineKind::Context => {
                pairs.push((Some((i, lines[i].clone())), Some((i, lines[i].clone()))));
                i += 1;
            }
            DiffLineKind::Deletion => {
                let mut dels = Vec::new();
                while i < lines.len() && lines[i].kind == DiffLineKind::Deletion {
                    dels.push((i, lines[i].clone()));
                    i += 1;
                }
                let mut adds = Vec::new();
                while i < lines.len() && lines[i].kind == DiffLineKind::Addition {
                    adds.push((i, lines[i].clone()));
                    i += 1;
                }
                let max_len = dels.len().max(adds.len());
                for j in 0..max_len {
                    pairs.push((dels.get(j).cloned(), adds.get(j).cloned()));
                }
            }
            DiffLineKind::Addition => {
                pairs.push((None, Some((i, lines[i].clone()))));
                i += 1;
            }
        }
    }

    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_diff() {
        let parsed = parse_unified_diff("");
        assert!(parsed.files.is_empty());
    }

    #[test]
    fn parse_single_file_diff() {
        let diff = "\
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
 fn main() {
-    println!(\"old\");
+    println!(\"new\");
+    println!(\"extra\");
 }
";
        let parsed = parse_unified_diff(diff);
        assert_eq!(parsed.files.len(), 1);
        assert_eq!(parsed.files[0].new_path, "src/main.rs");
        assert_eq!(parsed.files[0].hunks.len(), 1);
        let hunk = &parsed.files[0].hunks[0];
        assert_eq!(hunk.lines.len(), 5);
        assert_eq!(
            hunk.lines
                .iter()
                .filter(|l| l.kind == DiffLineKind::Addition)
                .count(),
            2
        );
        assert_eq!(
            hunk.lines
                .iter()
                .filter(|l| l.kind == DiffLineKind::Deletion)
                .count(),
            1
        );
    }

    #[test]
    fn parse_multi_file_diff() {
        let diff = "\
--- a/a.rs
+++ b/a.rs
@@ -1,1 +1,1 @@
-old_a
+new_a
--- a/b.rs
+++ b/b.rs
@@ -1,1 +1,1 @@
-old_b
+new_b
";
        let parsed = parse_unified_diff(diff);
        assert_eq!(parsed.files.len(), 2);
        assert_eq!(parsed.files[0].new_path, "a.rs");
        assert_eq!(parsed.files[1].new_path, "b.rs");
    }

    #[test]
    fn parse_multi_hunk_diff() {
        let diff = "\
--- a/f.rs
+++ b/f.rs
@@ -1,3 +1,3 @@
 line1
-old
+new
@@ -10,2 +10,2 @@
-ten_old
+ten_new
 eleven
";
        let parsed = parse_unified_diff(diff);
        assert_eq!(parsed.files.len(), 1);
        assert_eq!(parsed.files[0].hunks.len(), 2);
    }

    #[test]
    fn line_numbers_assigned_correctly() {
        let diff = "\
--- a/f.rs
+++ b/f.rs
@@ -5,3 +5,4 @@
 context
-removed
+added1
+added2
 context2
";
        let parsed = parse_unified_diff(diff);
        let lines = &parsed.files[0].hunks[0].lines;
        // Context at old:5, new:5
        assert_eq!(lines[0].old_lineno, Some(5));
        assert_eq!(lines[0].new_lineno, Some(5));
        // Deletion at old:6
        assert_eq!(lines[1].old_lineno, Some(6));
        assert_eq!(lines[1].new_lineno, None);
        // Addition at new:6
        assert_eq!(lines[2].old_lineno, None);
        assert_eq!(lines[2].new_lineno, Some(6));
        // Addition at new:7
        assert_eq!(lines[3].old_lineno, None);
        assert_eq!(lines[3].new_lineno, Some(7));
    }

    #[test]
    fn build_side_by_side_pairs_context_lines() {
        let lines = vec![DiffLine {
            kind: DiffLineKind::Context,
            old_lineno: Some(1),
            new_lineno: Some(1),
            content: "same".into(),
        }];
        let pairs = build_side_by_side_pairs(&lines);
        assert_eq!(pairs.len(), 1);
        assert!(pairs[0].0.is_some());
        assert!(pairs[0].1.is_some());
    }

    #[test]
    fn build_side_by_side_pairs_deletion_addition() {
        let lines = vec![
            DiffLine {
                kind: DiffLineKind::Deletion,
                old_lineno: Some(1),
                new_lineno: None,
                content: "old".into(),
            },
            DiffLine {
                kind: DiffLineKind::Addition,
                old_lineno: None,
                new_lineno: Some(1),
                content: "new".into(),
            },
        ];
        let pairs = build_side_by_side_pairs(&lines);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0.as_ref().unwrap().1.content, "old");
        assert_eq!(pairs[0].1.as_ref().unwrap().1.content, "new");
    }

    #[test]
    fn build_side_by_side_pairs_unbalanced() {
        let lines = vec![
            DiffLine {
                kind: DiffLineKind::Deletion,
                old_lineno: Some(1),
                new_lineno: None,
                content: "a".into(),
            },
            DiffLine {
                kind: DiffLineKind::Deletion,
                old_lineno: Some(2),
                new_lineno: None,
                content: "b".into(),
            },
            DiffLine {
                kind: DiffLineKind::Addition,
                old_lineno: None,
                new_lineno: Some(1),
                content: "c".into(),
            },
        ];
        let pairs = build_side_by_side_pairs(&lines);
        assert_eq!(pairs.len(), 2);
        assert!(pairs[0].0.is_some());
        assert!(pairs[0].1.is_some());
        assert!(pairs[1].0.is_some());
        assert!(pairs[1].1.is_none());
    }
}
