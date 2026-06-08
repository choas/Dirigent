//! Shared git-graph rendering helpers.
//!
//! Both the sidebar git-log (`panels::file_tree`) and the full-screen graph
//! view (`dialog::graph_view`) draw the same commit-graph lanes, dots, ref
//! badges and hover tooltips. These helpers are the single canonical
//! implementation so the two views cannot drift apart. View-specific cosmetics
//! (lane width, line/dot sizes, ellipsis font, whether the hover includes the
//! commit body) are passed in by the caller rather than hard-coded here.

use eframe::egui;

// ---------------------------------------------------------------------------
// Branch lineage
// ---------------------------------------------------------------------------

/// Result of computing which rows belong to a branch lineage.
pub(in crate::app) struct BranchLineage {
    /// Per-row flag: true if this row is part of the highlighted lineage.
    pub(in crate::app) rows: Vec<bool>,
    /// The lane column that defines this lineage.
    pub(in crate::app) lane: usize,
}

/// Trace the contiguous branch path through `lane` starting from `hovered_row`.
/// Returns `None` if the hovered row is out of range or has no graph data.
pub(in crate::app) fn compute_branch_lineage(
    graph_rows: &[crate::git::graph::GraphRow],
    hovered_row: usize,
) -> Option<BranchLineage> {
    let graph = graph_rows.get(hovered_row)?;
    let lane = graph.column;
    let num = graph_rows.len();
    let mut rows = vec![false; num];
    rows[hovered_row] = true;

    // Trace upward: contiguous non-Empty segments in this lane.
    for i in (0..hovered_row).rev() {
        let seg = graph_rows[i]
            .lanes
            .get(lane)
            .copied()
            .unwrap_or(crate::git::graph::LaneSegment::Empty);
        if seg == crate::git::graph::LaneSegment::Empty {
            break;
        }
        rows[i] = true;
    }

    // Trace downward.
    for i in (hovered_row + 1)..num {
        let seg = graph_rows[i]
            .lanes
            .get(lane)
            .copied()
            .unwrap_or(crate::git::graph::LaneSegment::Empty);
        if seg == crate::git::graph::LaneSegment::Empty {
            break;
        }
        rows[i] = true;
    }

    Some(BranchLineage { rows, lane })
}

// ---------------------------------------------------------------------------
// Graph column painting
// ---------------------------------------------------------------------------

/// Geometric layout for painting a single lane segment.
struct LaneLayout {
    x: f32,
    top: f32,
    bot: f32,
    mid_y: f32,
    commit_x: f32,
}

/// Visual style for painting a lane segment.
struct LaneStyle {
    stroke: egui::Stroke,
    is_highlighted: bool,
    color: egui::Color32,
    /// Radius of the (un-highlighted) commit dot; the highlighted dot is one
    /// pixel larger and the vertical gap around the dot matches this radius.
    dot_radius: f32,
}

/// Shared context for graph painting functions.
pub(in crate::app) struct GraphPaintCtx<'a> {
    pub(in crate::app) graph_left: f32,
    pub(in crate::app) lane_width: f32,
    pub(in crate::app) base_line_width: f32,
    /// Radius of an un-highlighted commit dot (view-specific sizing).
    pub(in crate::app) dot_radius: f32,
    /// Font size for the overflow ellipsis glyph.
    pub(in crate::app) ellipsis_font_size: f32,
    pub(in crate::app) max_visible_lanes: usize,
    pub(in crate::app) lane_colors: &'a [egui::Color32; 6],
    pub(in crate::app) highlight_lane: Option<usize>,
    pub(in crate::app) any_highlight_active: bool,
}

/// Resolve color and stroke width based on highlight state.
fn resolve_lane_style(
    base_color: egui::Color32,
    is_highlighted: bool,
    any_highlight_active: bool,
    base_line_width: f32,
) -> (egui::Color32, f32) {
    if !any_highlight_active {
        return (base_color, base_line_width);
    }
    if is_highlighted {
        (base_color, base_line_width * 1.6)
    } else {
        (base_color.linear_multiply(0.25), base_line_width)
    }
}

/// Paint a single lane segment (vertical line, commit dot, fork, or merge).
fn paint_lane_segment(
    painter: &egui::Painter,
    segment: &crate::git::graph::LaneSegment,
    layout: &LaneLayout,
    style: &LaneStyle,
) {
    use crate::git::graph::LaneSegment;
    let LaneLayout {
        x,
        top,
        bot,
        mid_y,
        commit_x,
    } = *layout;
    let gap = style.dot_radius;
    match segment {
        LaneSegment::Straight => {
            painter.line_segment([egui::pos2(x, top), egui::pos2(x, bot)], style.stroke);
        }
        LaneSegment::Commit => {
            painter.line_segment(
                [egui::pos2(x, top), egui::pos2(x, mid_y - gap)],
                style.stroke,
            );
            painter.line_segment(
                [egui::pos2(x, mid_y + gap), egui::pos2(x, bot)],
                style.stroke,
            );
            let dot_radius = if style.is_highlighted {
                style.dot_radius + 1.0
            } else {
                style.dot_radius
            };
            painter.circle_filled(egui::pos2(x, mid_y), dot_radius, style.color);
        }
        LaneSegment::ForkRight => {
            painter.line_segment(
                [egui::pos2(commit_x, mid_y), egui::pos2(x, mid_y)],
                style.stroke,
            );
            painter.line_segment([egui::pos2(x, mid_y), egui::pos2(x, bot)], style.stroke);
        }
        LaneSegment::MergeLeft => {
            painter.line_segment([egui::pos2(x, top), egui::pos2(x, mid_y)], style.stroke);
            painter.line_segment(
                [egui::pos2(x, mid_y), egui::pos2(commit_x, mid_y)],
                style.stroke,
            );
        }
        LaneSegment::Empty => {}
    }
}

/// Draw connection diagonals for merge/fork lines.
fn paint_graph_connections(
    painter: &egui::Painter,
    graph: &crate::git::graph::GraphRow,
    mid_y: f32,
    ctx: &GraphPaintCtx,
) {
    use crate::git::graph::LaneSegment;

    for &(from_lane, to_lane) in &graph.connections {
        let vis_from = from_lane.min(ctx.max_visible_lanes);
        let vis_to = to_lane.min(ctx.max_visible_lanes);
        if vis_from == vis_to {
            continue;
        }
        // Only draw explicit connection if not already drawn by ForkRight/MergeLeft
        // on *either* the source or target lane.
        let is_merge_or_fork = |lane: usize| -> bool {
            lane < ctx.max_visible_lanes
                && matches!(
                    graph.lanes.get(lane),
                    Some(&LaneSegment::ForkRight) | Some(&LaneSegment::MergeLeft)
                )
        };
        if is_merge_or_fork(from_lane) || is_merge_or_fork(to_lane) {
            continue;
        }
        let from_x = ctx.graph_left + (vis_from as f32 + 0.5) * ctx.lane_width;
        let to_x = ctx.graph_left + (vis_to as f32 + 0.5) * ctx.lane_width;
        let color_idx = to_lane.min(ctx.lane_colors.len() - 1);
        let base_color = ctx.lane_colors[color_idx % ctx.lane_colors.len()];
        let conn_hl = ctx.highlight_lane == Some(from_lane) || ctx.highlight_lane == Some(to_lane);
        let (color, conn_width) = resolve_lane_style(
            base_color,
            conn_hl,
            ctx.any_highlight_active,
            ctx.base_line_width,
        );
        painter.line_segment(
            [egui::pos2(from_x, mid_y), egui::pos2(to_x, mid_y)],
            egui::Stroke::new(conn_width, color),
        );
    }
}

/// Paint the graph column (lane lines + commit dot) for one row.
pub(in crate::app) fn paint_graph_column(
    ui: &egui::Ui,
    row_rect: egui::Rect,
    graph: &crate::git::graph::GraphRow,
    ctx: &GraphPaintCtx<'_>,
) {
    use crate::git::graph::LaneSegment;

    let painter = ui.painter();
    let top = row_rect.top();
    let bot = row_rect.bottom();
    let mid_y = row_rect.center().y;
    let clamped_column = graph.column.min(ctx.max_visible_lanes);
    let commit_x = ctx.graph_left + (clamped_column as f32 + 0.5) * ctx.lane_width;

    for (lane_idx, segment) in graph.lanes.iter().take(ctx.max_visible_lanes).enumerate() {
        let x = ctx.graph_left + (lane_idx as f32 + 0.5) * ctx.lane_width;
        let base_color = ctx.lane_colors[lane_idx % ctx.lane_colors.len()];
        let is_hl = ctx.highlight_lane == Some(lane_idx);
        let (color, line_width) = resolve_lane_style(
            base_color,
            is_hl,
            ctx.any_highlight_active,
            ctx.base_line_width,
        );

        let layout = LaneLayout {
            x,
            top,
            bot,
            mid_y,
            commit_x,
        };
        let style = LaneStyle {
            stroke: egui::Stroke::new(line_width, color),
            is_highlighted: is_hl,
            color,
            dot_radius: ctx.dot_radius,
        };
        paint_lane_segment(painter, segment, &layout, &style);
    }

    // Ellipsis / overflow column: show "···" when this row has active lanes beyond
    // the visible cap.  If the commit itself is in the overflow region, paint a
    // commit dot there instead of just an ellipsis so the row is never markerless.
    let has_overflow = graph
        .lanes
        .iter()
        .skip(ctx.max_visible_lanes)
        .any(|s| *s != LaneSegment::Empty);
    if has_overflow {
        let ellipsis_x = ctx.graph_left + (ctx.max_visible_lanes as f32 + 0.5) * ctx.lane_width;
        if graph.column >= ctx.max_visible_lanes {
            // Commit is hidden beyond the visible lanes — draw dot here.
            let dot_color = ctx.lane_colors[graph.column % ctx.lane_colors.len()];
            painter.circle_filled(egui::pos2(ellipsis_x, mid_y), ctx.dot_radius, dot_color);
        } else {
            let color = ui.visuals().text_color().gamma_multiply(0.4);
            painter.text(
                egui::pos2(ellipsis_x, mid_y),
                egui::Align2::CENTER_CENTER,
                "\u{00B7}\u{00B7}\u{00B7}",
                egui::FontId::monospace(ctx.ellipsis_font_size),
                color,
            );
        }
    }

    paint_graph_connections(painter, graph, mid_y, ctx);
}

// ---------------------------------------------------------------------------
// Ref badges
// ---------------------------------------------------------------------------

/// Paint branch/tag ref badges next to the commit message.
#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn paint_commit_badges(
    ui: &egui::Ui,
    commit: &crate::git::CommitInfo,
    text_x: f32,
    text_y: f32,
    label: &str,
    label_font: &egui::FontId,
    text_color: egui::Color32,
    lane_colors: &[egui::Color32; 6],
    warning: egui::Color32,
) {
    if commit.branch_labels.is_empty() && commit.tag_labels.is_empty() {
        return;
    }
    let label_galley =
        ui.painter()
            .layout_no_wrap(label.to_string(), label_font.clone(), text_color);
    let mut badge_x = text_x + label_galley.size().x + 6.0;
    let badge_font = egui::FontId::proportional(9.0);
    for (i, branch_name) in commit.branch_labels.iter().enumerate() {
        let color = lane_colors[i % lane_colors.len()];
        badge_x = paint_ref_badge(ui, badge_x, text_y, branch_name, color, &badge_font);
    }
    for tag_name in &commit.tag_labels {
        // Tags use the warning/yellow color.
        badge_x = paint_ref_badge(ui, badge_x, text_y, tag_name, warning, &badge_font);
    }
    let _ = badge_x; // suppress unused
}

/// Paint a small rounded badge for a branch or tag ref. Returns x position after badge.
fn paint_ref_badge(
    ui: &egui::Ui,
    x: f32,
    center_y: f32,
    name: &str,
    color: egui::Color32,
    font: &egui::FontId,
) -> f32 {
    let galley = ui
        .painter()
        .layout_no_wrap(name.to_string(), font.clone(), color);
    let text_size = galley.size();
    let pad_x = 4.0;
    let pad_y = 1.0;
    let badge_rect = egui::Rect::from_min_size(
        egui::pos2(x, center_y - text_size.y / 2.0 - pad_y),
        egui::vec2(text_size.x + pad_x * 2.0, text_size.y + pad_y * 2.0),
    );
    let bg = color.linear_multiply(0.15);
    ui.painter().rect_filled(badge_rect, 3.0, bg);
    ui.painter().galley(
        egui::pos2(x + pad_x, center_y - text_size.y / 2.0),
        galley,
        color,
    );
    badge_rect.right() + 4.0
}

// ---------------------------------------------------------------------------
// Hover tooltip
// ---------------------------------------------------------------------------

/// Format the hover tooltip for a commit entry. When `include_body` is true the
/// commit body (if any) is shown beneath the subject line.
pub(in crate::app) fn format_commit_hover(
    commit: &crate::git::CommitInfo,
    is_unpushed: bool,
    include_body: bool,
) -> String {
    let mut lines = Vec::new();
    if is_unpushed {
        lines.push("\u{2B06} Not pushed".to_string());
    }
    lines.push(format!("{} - {}", commit.short_hash, commit.author));
    lines.push(commit.message.clone());
    if include_body && !commit.body.is_empty() {
        lines.push(commit.body.clone());
    }
    lines.push(commit.time_ago.clone());
    if !commit.branch_labels.is_empty() {
        lines.push(format!("Branches: {}", commit.branch_labels.join(", ")));
    }
    if !commit.tag_labels.is_empty() {
        lines.push(format!("Tags: {}", commit.tag_labels.join(", ")));
    }
    if commit.is_merge {
        lines.push("Merge commit".to_string());
    }
    lines.join("\n")
}
