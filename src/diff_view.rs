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

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

pub(crate) fn parse_unified_diff(diff_text: &str) -> ParsedDiff {
    let mut files = Vec::new();
    let lines: Vec<&str> = diff_text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let is_file_header =
            lines[i].starts_with("--- ") && i + 1 < lines.len() && lines[i + 1].starts_with("+++ ");
        if !is_file_header {
            i += 1;
            continue;
        }

        let new_path = lines[i + 1]
            .strip_prefix("+++ b/")
            .or_else(|| lines[i + 1].strip_prefix("+++ "))
            .unwrap_or("")
            .to_string();
        i += 2;

        let (hunks, next_i) = parse_file_hunks(&lines, i);
        i = next_i;
        files.push(FileDiff { new_path, hunks });
    }

    ParsedDiff { files }
}

/// Parse all hunks for a single file, starting at position `i`.
/// Returns the hunks and the updated line index.
fn parse_file_hunks(lines: &[&str], mut i: usize) -> (Vec<DiffHunk>, usize) {
    let mut hunks = Vec::new();

    while i < lines.len() && !lines[i].starts_with("--- ") {
        if !lines[i].starts_with("@@ ") {
            i += 1;
            continue;
        }

        let (old_start, new_start) = parse_hunk_header(lines[i]);
        i += 1;

        let (hunk_lines, next_i) = parse_hunk_lines(lines, i, old_start, new_start);
        i = next_i;

        hunks.push(DiffHunk {
            new_start,
            lines: hunk_lines,
        });
    }

    (hunks, i)
}

/// Parse the individual lines within a single hunk.
/// Returns the diff lines and the updated line index.
fn parse_hunk_lines(
    lines: &[&str],
    mut i: usize,
    mut old_line: usize,
    mut new_line: usize,
) -> (Vec<DiffLine>, usize) {
    let mut hunk_lines = Vec::new();

    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("@@ ") || line.starts_with("--- ") {
            break;
        }

        if let Some(diff_line) = classify_hunk_line(line, &mut old_line, &mut new_line) {
            hunk_lines.push(diff_line);
        } else {
            break;
        }
        i += 1;
    }

    (hunk_lines, i)
}

/// Classify a single line within a hunk, updating line counters.
/// Returns `None` if the line doesn't match any expected prefix.
fn classify_hunk_line(line: &str, old_line: &mut usize, new_line: &mut usize) -> Option<DiffLine> {
    if let Some(content) = line.strip_prefix('+') {
        let dl = DiffLine {
            kind: DiffLineKind::Addition,
            old_lineno: None,
            new_lineno: Some(*new_line),
            content: content.to_string(),
        };
        *new_line += 1;
        Some(dl)
    } else if let Some(content) = line.strip_prefix('-') {
        let dl = DiffLine {
            kind: DiffLineKind::Deletion,
            old_lineno: Some(*old_line),
            new_lineno: None,
            content: content.to_string(),
        };
        *old_line += 1;
        Some(dl)
    } else if line.starts_with(' ') || line.is_empty() {
        let content = if line.is_empty() { "" } else { &line[1..] };
        let dl = DiffLine {
            kind: DiffLineKind::Context,
            old_lineno: Some(*old_line),
            new_lineno: Some(*new_line),
            content: content.to_string(),
        };
        *old_line += 1;
        *new_line += 1;
        Some(dl)
    } else {
        None
    }
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

// ---------------------------------------------------------------------------
// Shared rendering helpers
// ---------------------------------------------------------------------------

/// Optional search highlight state for diff rendering.
pub(crate) struct DiffSearchHighlight<'a> {
    pub query_lower: &'a str,
    /// The current match to scroll to: (file_idx, hunk_idx, line_idx).
    pub current: Option<(usize, usize, usize)>,
}

/// Count additions and deletions in a file.
fn count_file_changes(file: &FileDiff) -> (usize, usize) {
    let all_lines = file.hunks.iter().flat_map(|h| &h.lines);
    let additions = all_lines
        .clone()
        .filter(|l| l.kind == DiffLineKind::Addition)
        .count();
    let deletions = all_lines
        .filter(|l| l.kind == DiffLineKind::Deletion)
        .count();
    (additions, deletions)
}

/// Render the collapsible file header. Returns true if clicked.
fn render_file_header(
    ui: &mut egui::Ui,
    file: &FileDiff,
    is_collapsed: bool,
    colors: &SemanticColors,
) -> bool {
    let arrow = if is_collapsed { "\u{25B6}" } else { "\u{25BC}" };
    let (additions, deletions) = count_file_changes(file);
    let stats = format!(" +{} -{}", additions, deletions);

    ui.add(
        egui::Label::new(
            egui::RichText::new(format!("{} {}{}", arrow, file.new_path, stats))
                .strong()
                .color(colors.diff_header()),
        )
        .sense(egui::Sense::click()),
    )
    .clicked()
}

/// Toggle file collapsed state.
fn toggle_collapsed(collapsed_files: &mut HashSet<usize>, file_idx: usize) {
    if collapsed_files.contains(&file_idx) {
        collapsed_files.remove(&file_idx);
    } else {
        collapsed_files.insert(file_idx);
    }
}

/// Compute effective background for a diff line considering search state.
fn effective_background(
    is_current: bool,
    is_match: bool,
    default_bg: Option<egui::Color32>,
    colors: &SemanticColors,
) -> Option<egui::Color32> {
    if is_current {
        Some(colors.search_current_bg())
    } else if is_match {
        Some(colors.search_match_bg())
    } else {
        default_bg
    }
}

/// Check if a line matches the search query.
fn is_line_search_match(line: &DiffLine, search: Option<&DiffSearchHighlight<'_>>) -> bool {
    search
        .map(|s| !s.query_lower.is_empty() && line.content.to_lowercase().contains(s.query_lower))
        .unwrap_or(false)
}

/// Check if a line is the current search match.
fn is_line_current_match(
    search: Option<&DiffSearchHighlight<'_>>,
    file_idx: usize,
    hunk_idx: usize,
    line_idx: usize,
) -> bool {
    search
        .and_then(|s| s.current)
        .map(|c| c == (file_idx, hunk_idx, line_idx))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Inline diff rendering
// ---------------------------------------------------------------------------

struct InlineLineColors {
    green_bg: egui::Color32,
    red_bg: egui::Color32,
    green_text: egui::Color32,
    red_text: egui::Color32,
    context_text: egui::Color32,
    gutter_color: egui::Color32,
}

/// Determine prefix, text color, and background color for a diff line kind.
fn line_style(
    kind: DiffLineKind,
    lc: &InlineLineColors,
) -> (&'static str, egui::Color32, Option<egui::Color32>) {
    match kind {
        DiffLineKind::Addition => ("+", lc.green_text, Some(lc.green_bg)),
        DiffLineKind::Deletion => ("-", lc.red_text, Some(lc.red_bg)),
        DiffLineKind::Context => (" ", lc.context_text, None),
    }
}

pub(crate) fn render_inline_diff(
    ui: &mut egui::Ui,
    diff: &ParsedDiff,
    collapsed_files: &mut HashSet<usize>,
    search: Option<&DiffSearchHighlight<'_>>,
    colors: &SemanticColors,
) {
    for (file_idx, file) in diff.files.iter().enumerate() {
        let is_collapsed = collapsed_files.contains(&file_idx);
        if render_file_header(ui, file, is_collapsed, colors) {
            toggle_collapsed(collapsed_files, file_idx);
        }

        if !collapsed_files.contains(&file_idx) {
            ui.add_space(SPACE_XS);
            render_inline_file_hunks(ui, file, file_idx, search, colors);
        }

        ui.separator();
    }
}

/// Render all hunks for a single file in inline mode.
fn render_inline_file_hunks(
    ui: &mut egui::Ui,
    file: &FileDiff,
    file_idx: usize,
    search: Option<&DiffSearchHighlight<'_>>,
    colors: &SemanticColors,
) {
    let lc = InlineLineColors {
        green_bg: colors.addition_bg(),
        red_bg: colors.deletion_bg(),
        green_text: colors.success,
        red_text: colors.danger,
        context_text: colors.secondary_text,
        gutter_color: colors.tertiary_text,
    };

    for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
        for (line_idx, line) in hunk.lines.iter().enumerate() {
            render_inline_line(ui, line, file_idx, hunk_idx, line_idx, search, colors, &lc);
        }
        ui.add_space(SPACE_SM);
    }
}

/// Render a single inline diff line.
fn render_inline_line(
    ui: &mut egui::Ui,
    line: &DiffLine,
    file_idx: usize,
    hunk_idx: usize,
    line_idx: usize,
    search: Option<&DiffSearchHighlight<'_>>,
    colors: &SemanticColors,
    lc: &InlineLineColors,
) {
    let old_num = line
        .old_lineno
        .map(|n| format!("{:>4}", n))
        .unwrap_or_else(|| "    ".to_string());
    let new_num = line
        .new_lineno
        .map(|n| format!("{:>4}", n))
        .unwrap_or_else(|| "    ".to_string());
    let (prefix, text_color, bg_color) = line_style(line.kind, lc);

    let is_search = is_line_search_match(line, search);
    let is_current = is_line_current_match(search, file_idx, hunk_idx, line_idx);

    let response = ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{} {} {}", old_num, new_num, prefix))
                .monospace()
                .color(lc.gutter_color),
        );
        if is_search {
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

    if is_current {
        response.response.scroll_to_me(Some(egui::Align::Center));
    }

    if let Some(bg) = effective_background(is_current, is_search, bg_color, colors) {
        ui.painter().rect_filled(response.response.rect, 0, bg);
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
            ui.painter().rect_filled(resp.rect, 2, highlight_bg);
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

// ---------------------------------------------------------------------------
// Side-by-side diff rendering
// ---------------------------------------------------------------------------

struct SbsColors {
    green_bg: egui::Color32,
    red_bg: egui::Color32,
    green_text: egui::Color32,
    red_text: egui::Color32,
    context_text: egui::Color32,
    gutter_color: egui::Color32,
    sep_color: egui::Color32,
}

pub(crate) fn render_side_by_side_diff(
    ui: &mut egui::Ui,
    diff: &ParsedDiff,
    collapsed_files: &mut HashSet<usize>,
    search: Option<&DiffSearchHighlight<'_>>,
    colors: &SemanticColors,
) {
    let sc = SbsColors {
        green_bg: colors.addition_bg(),
        red_bg: colors.deletion_bg(),
        green_text: colors.success,
        red_text: colors.danger,
        context_text: colors.secondary_text,
        gutter_color: colors.tertiary_text,
        sep_color: colors.separator,
    };

    for (file_idx, file) in diff.files.iter().enumerate() {
        let is_collapsed = collapsed_files.contains(&file_idx);
        if render_file_header(ui, file, is_collapsed, colors) {
            toggle_collapsed(collapsed_files, file_idx);
        }

        if collapsed_files.contains(&file_idx) {
            ui.separator();
            continue;
        }

        ui.add_space(SPACE_XS);
        render_sbs_file_hunks(ui, file, file_idx, search, colors, &sc);
        ui.separator();
    }
}

/// Render all hunks for a single file in side-by-side mode.
fn render_sbs_file_hunks(
    ui: &mut egui::Ui,
    file: &FileDiff,
    file_idx: usize,
    search: Option<&DiffSearchHighlight<'_>>,
    colors: &SemanticColors,
    sc: &SbsColors,
) {
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
                render_sbs_row(
                    ui,
                    left,
                    right,
                    file_idx,
                    hunk_idx,
                    current_match,
                    query_lower,
                    colors,
                    sc,
                );
                ui.end_row();
            }
        });

        ui.add_space(SPACE_SM);
    }
}

/// Check if one side of a pair is the current search match.
fn is_side_current(
    side: &Option<(usize, DiffLine)>,
    current_match: Option<(usize, usize, usize)>,
    file_idx: usize,
    hunk_idx: usize,
) -> bool {
    side.as_ref()
        .map(|(idx, _)| current_match == Some((file_idx, hunk_idx, *idx)))
        .unwrap_or(false)
}

/// Render a line number gutter cell.
fn render_sbs_line_number(ui: &mut egui::Ui, lineno: Option<usize>, gutter_color: egui::Color32) {
    let text = lineno
        .map(|n| format!("{:>4}", n))
        .unwrap_or_else(|| "    ".to_string());
    ui.label(egui::RichText::new(text).monospace().color(gutter_color));
}

/// Render a content cell on one side of the side-by-side view.
fn render_sbs_content_cell(
    ui: &mut egui::Ui,
    side: &Option<(usize, DiffLine)>,
    side_is_current: bool,
    query_lower: &str,
    highlight_kind: DiffLineKind,
    highlight_text: egui::Color32,
    highlight_bg: egui::Color32,
    context_text: egui::Color32,
    colors: &SemanticColors,
) {
    let Some((_, ref line)) = side else {
        ui.label(egui::RichText::new(" ").monospace());
        return;
    };

    let (color, bg) = if line.kind == highlight_kind {
        (highlight_text, Some(highlight_bg))
    } else {
        (context_text, None)
    };
    let is_match = !query_lower.is_empty() && line.content.to_lowercase().contains(query_lower);
    let resp = ui.label(egui::RichText::new(&line.content).monospace().color(color));

    if let Some(bg) = effective_background(side_is_current, is_match, bg, colors) {
        ui.painter().rect_filled(resp.rect, 0, bg);
    }
}

/// Render one complete row (left number, left content, separator, right number, right content).
fn render_sbs_row(
    ui: &mut egui::Ui,
    left: &Option<(usize, DiffLine)>,
    right: &Option<(usize, DiffLine)>,
    file_idx: usize,
    hunk_idx: usize,
    current_match: Option<(usize, usize, usize)>,
    query_lower: &str,
    colors: &SemanticColors,
    sc: &SbsColors,
) {
    let left_is_current = is_side_current(left, current_match, file_idx, hunk_idx);
    let right_is_current = is_side_current(right, current_match, file_idx, hunk_idx);
    let row_is_current = left_is_current || right_is_current;

    // Old line number
    let old_lineno = left.as_ref().and_then(|(_, l)| l.old_lineno);
    render_sbs_line_number(ui, old_lineno, sc.gutter_color);

    // Old content
    render_sbs_content_cell(
        ui,
        left,
        left_is_current,
        query_lower,
        DiffLineKind::Deletion,
        sc.red_text,
        sc.red_bg,
        sc.context_text,
        colors,
    );

    // Separator
    let sep_resp = ui.label(
        egui::RichText::new("\u{2502}")
            .monospace()
            .color(sc.sep_color),
    );
    if row_is_current {
        sep_resp.scroll_to_me(Some(egui::Align::Center));
    }

    // New line number
    let new_lineno = right.as_ref().and_then(|(_, l)| l.new_lineno);
    render_sbs_line_number(ui, new_lineno, sc.gutter_color);

    // New content
    render_sbs_content_cell(
        ui,
        right,
        right_is_current,
        query_lower,
        DiffLineKind::Addition,
        sc.green_text,
        sc.green_bg,
        sc.context_text,
        colors,
    );
}

// ---------------------------------------------------------------------------
// Side-by-side pair building
// ---------------------------------------------------------------------------

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
