use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use eframe::egui;

use super::super::{DiffReview, DirigentApp, FONT_SCALE_SUBHEADING};
use crate::diff_view::{self, DiffViewMode};
use crate::file_tree::FileEntry;
use crate::git;
use crate::settings::SemanticColors;

/// Bundled context for recursive file-tree rendering, reducing parameter count.
pub(super) struct FileTreeCtx<'a> {
    pub expanded: &'a mut HashSet<PathBuf>,
    pub current_file: &'a Option<PathBuf>,
    pub action: &'a mut Option<FileTreeAction>,
    pub project_root: &'a Path,
    pub dirty_files: &'a HashMap<String, char>,
    pub semantic: &'a SemanticColors,
    pub depth: usize,
    pub font_size: f32,
    pub status_msg: &'a mut Option<String>,
}

/// Actions triggered from the file tree context menu.
pub(super) enum FileTreeAction {
    Open(PathBuf),
    AddToGitignore(PathBuf),
    Delete(PathBuf, bool),
    RenameStart(PathBuf),
}

impl DirigentApp {
    pub(in super::super) fn render_file_tree_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("file_tree")
            .default_size(220.0)
            .min_size(150.0)
            .max_size(400.0)
            .show_inside(ui, |ui| {
                ui.label(
                    egui::RichText::new("Files")
                        .size(self.settings.font_size * FONT_SCALE_SUBHEADING)
                        .strong(),
                );
                ui.separator();

                let file_tree_height = Self::compute_file_tree_height(
                    ui.available_height(),
                    self.viewer.active().is_some_and(|t| !t.symbols.is_empty()),
                    self.git.show_log,
                );
                let (tree_action, tree_status_msg) = egui::ScrollArea::vertical()
                    .id_salt("file_tree_scroll")
                    .max_height(file_tree_height)
                    .show(ui, |ui| {
                        let mut action = None;
                        let mut status_msg = None;
                        if let Some(ref tree) = self.file_tree {
                            let current_file = self.viewer.current_file().cloned();
                            let mut ctx = FileTreeCtx {
                                expanded: &mut self.expanded_dirs,
                                current_file: &current_file,
                                action: &mut action,
                                project_root: &self.project_root,
                                dirty_files: &self.git.dirty_files,
                                semantic: &self.semantic,
                                depth: 0,
                                font_size: self.settings.font_size,
                                status_msg: &mut status_msg,
                            };
                            for entry in &tree.entries {
                                Self::render_file_entry(ui, entry, &mut ctx);
                            }
                        }
                        (action, status_msg)
                    })
                    .inner;
                if let Some(msg) = tree_status_msg {
                    self.set_status_message(msg);
                }
                self.handle_file_tree_action(tree_action);

                ui.separator();

                self.render_symbol_outline(ui);
                self.render_git_log_section(ui);
            });
    }

    /// Compute the height available for the file tree scroll area.
    fn compute_file_tree_height(available: f32, has_outline: bool, git_log_open: bool) -> f32 {
        let reserved = match (has_outline, git_log_open) {
            (true, true) => 174.0 + available * 0.3,
            (true, false) => 174.0,
            (false, true) => available * 0.4,
            (false, false) => 24.0,
        };
        (available - reserved).max(80.0)
    }

    /// Process actions returned from the file tree (open, gitignore, delete, rename).
    fn handle_file_tree_action(&mut self, action: Option<FileTreeAction>) {
        match action {
            Some(FileTreeAction::Open(path)) => {
                self.push_nav_history();
                self.load_file(path);
            }
            Some(FileTreeAction::AddToGitignore(path)) => {
                self.handle_add_to_gitignore(&path);
            }
            Some(FileTreeAction::Delete(path, is_dir)) => {
                self.pending_file_delete = Some((path, is_dir));
            }
            Some(FileTreeAction::RenameStart(path)) => {
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                self.rename_target = Some(path);
                self.rename_buffer = name;
                self.rename_focus_requested = false;
            }
            None => {}
        }
    }

    /// Append a path to .gitignore.
    fn handle_add_to_gitignore(&mut self, path: &Path) {
        let rel = path
            .strip_prefix(&self.project_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let gitignore = self.project_root.join(".gitignore");
        let entry_line = if path.is_dir() {
            format!("{}/", rel)
        } else {
            rel.clone()
        };
        let current = match std::fs::read_to_string(&gitignore) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                self.set_status_message(format!("Failed to read .gitignore: {}", e));
                return;
            }
        };
        let separator = if current.ends_with('\n') || current.is_empty() {
            ""
        } else {
            "\n"
        };
        if let Err(e) = std::fs::write(
            &gitignore,
            format!("{}{}{}\n", current, separator, entry_line),
        ) {
            self.set_status_message(format!("Failed to update .gitignore: {}", e));
        } else {
            self.set_status_message(format!("Added '{}' to .gitignore", entry_line));
            self.reload_file_tree();
        }
    }

    /// Render the symbol outline collapsible section.
    fn render_symbol_outline(&mut self, ui: &mut egui::Ui) {
        let Some(symbols) = self
            .viewer
            .active()
            .map(|t| &t.symbols)
            .filter(|s| !s.is_empty())
        else {
            return;
        };

        let outline_header = egui::CollapsingHeader::new(
            egui::RichText::new(format!("Outline ({})", symbols.len()))
                .size(self.settings.font_size * FONT_SCALE_SUBHEADING)
                .strong(),
        )
        .default_open(self.viewer.show_outline);
        let accent = self.semantic.accent;
        let outline_resp = outline_header.show(ui, |ui| {
            let mut scroll_to: Option<usize> = None;
            egui::ScrollArea::vertical()
                .id_salt("outline_scroll")
                .max_height(200.0)
                .show(ui, |ui| {
                    for sym in symbols {
                        let indent = sym.depth as f32 * 12.0;
                        ui.horizontal(|ui| {
                            ui.add_space(indent);
                            ui.label(
                                egui::RichText::new(sym.kind.icon())
                                    .monospace()
                                    .small()
                                    .color(accent),
                            );
                            let kind_label = sym.kind.label();
                            let mut label = sym.name.clone();
                            if !kind_label.is_empty() {
                                label = format!("{} {}", kind_label, sym.name);
                            }
                            if ui
                                .add(
                                    egui::Label::new(egui::RichText::new(&label).small())
                                        .truncate()
                                        .sense(egui::Sense::click()),
                                )
                                .clicked()
                            {
                                scroll_to = Some(sym.line);
                            }
                        });
                    }
                });
            scroll_to
        });
        self.viewer.show_outline = outline_resp.fully_open();
        if let Some(line) = outline_resp.body_returned.flatten() {
            self.viewer.scroll_to_line = Some(line);
        }

        ui.separator();
    }

    /// Render the git log collapsible section.
    fn render_git_log_section(&mut self, ui: &mut egui::Ui) {
        let ahead_label = if self.git.ahead_of_remote > 0 {
            format!(" [+{}]", self.git.ahead_of_remote)
        } else {
            String::new()
        };
        let header_text = format!(
            "Git Log ({}/{}){}",
            self.git.commit_history.len(),
            self.git.commit_history_total,
            ahead_label
        );
        let header_resp = egui::CollapsingHeader::new(
            egui::RichText::new(header_text)
                .size(self.settings.font_size * FONT_SCALE_SUBHEADING)
                .strong(),
        )
        .default_open(self.git.show_log)
        .show(ui, |ui| self.render_git_log_entries(ui));
        self.git.show_log = header_resp.fully_open();
        if let Some(Some((full_hash, message, body))) = header_resp.body_returned {
            self.open_commit_diff_review(&full_hash, &message, body);
        }
    }

    /// Render individual commit entries inside the git log scroll area.
    fn render_git_log_entries(&mut self, ui: &mut egui::Ui) -> Option<(String, String, String)> {
        let mut clicked_commit: Option<(String, String, String)> = None;
        let mut load_more = false;

        // Precompute graph column width: each lane = 12px, cap at 6 lanes.
        let lane_width = 12.0_f32;
        let visible_lanes = self.git.graph_max_lanes.clamp(1, 6);
        // Cast bool→u8 first: Rust does not allow direct bool→f32 casts.
        let extra_for_ellipsis = (self.git.graph_max_lanes > 6) as u8 as f32;
        let graph_col_width = (visible_lanes as f32 + extra_for_ellipsis + 0.5) * lane_width;

        // Compute branch lineage highlight from previous frame's hovered row.
        let lineage = self
            .git
            .hovered_graph_row
            .and_then(|row| compute_branch_lineage(&self.git.graph_rows, row));

        egui::ScrollArea::vertical()
            .id_salt("git_log_scroll")
            .show(ui, |ui| {
                let avail_width = ui.available_width();
                let char_width = self.settings.font_size * 0.52;
                let hash_prefix_len = 8;
                let text_avail = avail_width - graph_col_width - 4.0;
                let max_msg_chars = ((text_avail / char_width) as usize)
                    .saturating_sub(hash_prefix_len)
                    .max(10);
                let ahead = self.git.ahead_of_remote;
                let mut new_hovered: Option<usize> = None;
                for (idx, commit) in self.git.commit_history.iter().enumerate() {
                    let is_unpushed = idx < ahead;
                    let graph_row = self.git.graph_rows.get(idx);
                    let highlight_lane = lineage.as_ref().filter(|l| l.rows[idx]).map(|l| l.lane);
                    let (clicked, hovered) = render_commit_row(
                        ui,
                        &CommitRowParams {
                            commit,
                            is_unpushed,
                            max_msg_chars,
                            graph_row,
                            graph_col_width,
                            lane_width,
                            semantic: &self.semantic,
                            highlight_lane,
                            any_highlight_active: lineage.is_some(),
                        },
                    );
                    if clicked {
                        clicked_commit = Some((
                            commit.full_hash.clone(),
                            commit.message.clone(),
                            commit.body.clone(),
                        ));
                    }
                    if hovered {
                        new_hovered = Some(idx);
                    }
                }
                self.git.hovered_graph_row = new_hovered;
                if self.git.commit_history.len() == self.git.commit_history_limit {
                    ui.add_space(4.0);
                    if ui
                        .button(
                            egui::RichText::new("Load More\u{2026}")
                                .small()
                                .color(ui.visuals().hyperlink_color),
                        )
                        .clicked()
                    {
                        load_more = true;
                    }
                }
            });
        if load_more {
            self.git.commit_history_limit += 10;
            self.reload_commit_history();
        }
        clicked_commit
    }

    /// Open a diff review for the given commit.
    fn open_commit_diff_review(&mut self, full_hash: &str, message: &str, body: String) {
        let short_hash = &full_hash[..7.min(full_hash.len())];
        let diff_text = git::get_commit_diff(&self.project_root, full_hash).unwrap_or_default();
        let parsed = diff_view::parse_unified_diff(&diff_text);
        let cue_text = if body.len() > message.len() {
            body
        } else {
            format!("{} {}", short_hash, message)
        };
        self.dismiss_central_overlays();
        self.diff_review = Some(DiffReview {
            cue_id: 0,
            diff: diff_text,
            cue_text,
            commit_hash: Some(full_hash.to_string()),
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

    fn render_file_entry(ui: &mut egui::Ui, entry: &FileEntry, ctx: &mut FileTreeCtx<'_>) {
        if entry.is_dir {
            Self::render_dir_entry(ui, entry, ctx);
        } else {
            Self::render_file_leaf_entry(ui, entry, ctx);
        }
    }

    /// Render a directory entry row (disclosure triangle, name, context menu, children).
    fn render_dir_entry(ui: &mut egui::Ui, entry: &FileEntry, ctx: &mut FileTreeCtx<'_>) {
        let indent = ctx.depth as f32 * 16.0;
        let is_expanded = ctx.expanded.contains(&entry.path);
        let dir_has_dirty = Self::dir_has_dirty_files(entry, ctx.project_root, ctx.dirty_files);

        let (row_rect, response) = allocate_tree_row(ui);
        paint_hover_highlight(ui, &response, row_rect);

        // Disclosure triangle
        let triangle = if is_expanded { "\u{25BC}" } else { "\u{25B6}" };
        let text_pos = row_rect.left_center() + egui::vec2(indent, 0.0);
        ui.painter().text(
            egui::pos2(text_pos.x + 6.0, text_pos.y),
            egui::Align2::LEFT_CENTER,
            triangle,
            egui::FontId::proportional(10.0),
            ui.visuals().weak_text_color(),
        );

        // Directory name
        let name_color = entry_name_color(ui, entry.is_ignored, dir_has_dirty, ctx.semantic);
        ui.painter().text(
            egui::pos2(text_pos.x + 20.0, text_pos.y),
            egui::Align2::LEFT_CENTER,
            &entry.name,
            egui::FontId::proportional(ctx.font_size),
            name_color,
        );

        if response.clicked() {
            if is_expanded {
                ctx.expanded.remove(&entry.path);
            } else {
                ctx.expanded.insert(entry.path.clone());
            }
        }

        render_dir_context_menu(
            &response,
            entry,
            ctx.project_root,
            ctx.semantic,
            ctx.action,
            ctx.status_msg,
        );

        if is_expanded {
            let child_depth = ctx.depth + 1;
            let prev_depth = ctx.depth;
            ctx.depth = child_depth;
            for child in &entry.children {
                Self::render_file_entry(ui, child, ctx);
            }
            ctx.depth = prev_depth;
        }
    }

    /// Render a file (leaf) entry row (name, git badge, context menu).
    fn render_file_leaf_entry(ui: &mut egui::Ui, entry: &FileEntry, ctx: &mut FileTreeCtx<'_>) {
        let indent = ctx.depth as f32 * 16.0;
        let is_selected = ctx.current_file.as_ref() == Some(&entry.path);
        let rel = entry
            .path
            .strip_prefix(ctx.project_root)
            .unwrap_or(&entry.path)
            .to_string_lossy()
            .replace('\\', "/");
        let status_letter = ctx.dirty_files.get(&rel).copied();

        let (row_rect, response) = allocate_tree_row(ui);

        if is_selected {
            ui.painter()
                .rect_filled(row_rect, 0, ctx.semantic.selection_bg());
        }
        if !is_selected {
            paint_hover_highlight(ui, &response, row_rect);
        }

        // File name
        let name_color =
            entry_name_color(ui, entry.is_ignored, status_letter.is_some(), ctx.semantic);
        let text_pos = row_rect.left_center() + egui::vec2(indent + 20.0, 0.0);
        ui.painter().text(
            text_pos,
            egui::Align2::LEFT_CENTER,
            &entry.name,
            egui::FontId::proportional(ctx.font_size),
            name_color,
        );

        paint_git_status_badge(ui, row_rect, status_letter, ctx.semantic);

        if response.clicked() {
            *ctx.action = Some(FileTreeAction::Open(entry.path.clone()));
        }

        render_file_context_menu(
            &response,
            entry,
            &rel,
            ctx.project_root,
            ctx.semantic,
            ctx.action,
            ctx.status_msg,
        );
    }

    /// Check if a directory contains any dirty files (recursively).
    fn dir_has_dirty_files(
        entry: &FileEntry,
        project_root: &Path,
        dirty_files: &HashMap<String, char>,
    ) -> bool {
        if !entry.is_dir {
            let rel = entry
                .path
                .strip_prefix(project_root)
                .unwrap_or(&entry.path)
                .to_string_lossy()
                .replace('\\', "/");
            return dirty_files.contains_key(&rel);
        }
        entry
            .children
            .iter()
            .any(|child| Self::dir_has_dirty_files(child, project_root, dirty_files))
    }
}

// ---------------------------------------------------------------------------
// Git graph rendering helpers
// ---------------------------------------------------------------------------

/// Result of computing which rows belong to a branch lineage.
struct BranchLineage {
    /// Per-row flag: true if this row is part of the highlighted lineage.
    rows: Vec<bool>,
    /// The lane column that defines this lineage.
    lane: usize,
}

/// Trace the contiguous branch path through `lane` starting from `hovered_row`.
/// Returns `None` if the hovered row is out of range or has no graph data.
fn compute_branch_lineage(
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

/// Parameters for rendering a single commit row.
struct CommitRowParams<'a> {
    commit: &'a crate::git::CommitInfo,
    is_unpushed: bool,
    max_msg_chars: usize,
    graph_row: Option<&'a crate::git::graph::GraphRow>,
    graph_col_width: f32,
    lane_width: f32,
    semantic: &'a SemanticColors,
    highlight_lane: Option<usize>,
    any_highlight_active: bool,
}

fn render_commit_row(ui: &mut egui::Ui, params: &CommitRowParams<'_>) -> (bool, bool) {
    let is_dark = params.semantic.is_dark();
    let lane_colors = params.semantic.lane_colors();
    let row_height = ui.text_style_height(&egui::TextStyle::Small) + 4.0;
    let full_width = ui.available_width();

    // Allocate full row for click detection.
    let (row_rect, response) =
        ui.allocate_exact_size(egui::vec2(full_width, row_height), egui::Sense::click());

    // Branch lineage highlight (tinted row background).
    if params.highlight_lane.is_some() {
        let tint = if is_dark {
            egui::Color32::from_white_alpha(8)
        } else {
            egui::Color32::from_black_alpha(6)
        };
        ui.painter().rect_filled(row_rect, 0, tint);
    }

    // Hover highlight.
    if response.hovered() {
        let hover = if is_dark {
            egui::Color32::from_white_alpha(15)
        } else {
            egui::Color32::from_black_alpha(12)
        };
        ui.painter().rect_filled(row_rect, 0, hover);
    }

    // Paint graph column.
    if let Some(graph) = params.graph_row {
        let gctx = GraphPaintCtx {
            graph_left: row_rect.left(),
            lane_width: params.lane_width,
            base_line_width: 1.5,
            max_visible_lanes: 6,
            lane_colors: &lane_colors,
            highlight_lane: params.highlight_lane,
            any_highlight_active: params.any_highlight_active,
        };
        paint_graph_column(ui, row_rect, graph, row_height, &gctx);
    }

    // Paint commit text to the right of the graph column.
    let text_x = row_rect.left() + params.graph_col_width + 2.0;
    let text_y = row_rect.center().y;

    let msg = if params.commit.message.len() > params.max_msg_chars + 3 {
        format!(
            "{}...",
            super::super::truncate_str(&params.commit.message, params.max_msg_chars)
        )
    } else {
        params.commit.message.clone()
    };
    let dot = if params.is_unpushed { "\u{25CF} " } else { "" };
    let label = format!("{}{} {}", dot, params.commit.short_hash, msg);

    let text_color = if params.is_unpushed {
        ui.visuals().warn_fg_color
    } else {
        ui.visuals().text_color()
    };

    ui.painter().text(
        egui::pos2(text_x, text_y),
        egui::Align2::LEFT_CENTER,
        &label,
        egui::FontId::monospace(ui.text_style_height(&egui::TextStyle::Small) * 0.85),
        text_color,
    );

    // Branch/tag labels as colored badges.
    paint_commit_badges(
        ui,
        params.commit,
        text_x,
        text_y,
        &label,
        text_color,
        params.semantic,
    );

    // Hover tooltip.
    let hover_text = format_commit_hover(params.commit, params.is_unpushed);
    let hovered = response.hovered();
    let response = response.on_hover_text(hover_text);

    (response.clicked(), hovered)
}

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
}

/// Shared context for graph painting functions.
struct GraphPaintCtx<'a> {
    graph_left: f32,
    lane_width: f32,
    base_line_width: f32,
    max_visible_lanes: usize,
    lane_colors: &'a [egui::Color32; 6],
    highlight_lane: Option<usize>,
    any_highlight_active: bool,
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
    match segment {
        LaneSegment::Straight => {
            painter.line_segment([egui::pos2(x, top), egui::pos2(x, bot)], style.stroke);
        }
        LaneSegment::Commit => {
            painter.line_segment(
                [egui::pos2(x, top), egui::pos2(x, mid_y - 3.0)],
                style.stroke,
            );
            painter.line_segment(
                [egui::pos2(x, mid_y + 3.0), egui::pos2(x, bot)],
                style.stroke,
            );
            let dot_radius = if style.is_highlighted { 4.0 } else { 3.0 };
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
fn paint_graph_column(
    ui: &egui::Ui,
    row_rect: egui::Rect,
    graph: &crate::git::graph::GraphRow,
    row_height: f32,
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
            painter.circle_filled(egui::pos2(ellipsis_x, mid_y), 3.0, dot_color);
        } else {
            let color = ui.visuals().text_color().gamma_multiply(0.4);
            painter.text(
                egui::pos2(ellipsis_x, mid_y),
                egui::Align2::CENTER_CENTER,
                "\u{00B7}\u{00B7}\u{00B7}",
                egui::FontId::monospace(row_height * 0.7),
                color,
            );
        }
    }

    paint_graph_connections(painter, graph, mid_y, ctx);
}

/// Paint branch/tag ref badges next to the commit message.
fn paint_commit_badges(
    ui: &egui::Ui,
    commit: &crate::git::CommitInfo,
    text_x: f32,
    text_y: f32,
    label: &str,
    text_color: egui::Color32,
    semantic: &SemanticColors,
) {
    if commit.branch_labels.is_empty() && commit.tag_labels.is_empty() {
        return;
    }
    let lane_colors = semantic.lane_colors();
    let label_galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::monospace(ui.text_style_height(&egui::TextStyle::Small) * 0.85),
        text_color,
    );
    let mut badge_x = text_x + label_galley.size().x + 6.0;
    let badge_font = egui::FontId::proportional(9.0);
    for (i, branch_name) in commit.branch_labels.iter().enumerate() {
        let color = lane_colors[i % lane_colors.len()];
        badge_x = paint_ref_badge(ui, badge_x, text_y, branch_name, color, &badge_font);
    }
    for tag_name in &commit.tag_labels {
        let color = semantic.warning; // tags use warning/yellow
        badge_x = paint_ref_badge(ui, badge_x, text_y, tag_name, color, &badge_font);
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

/// Format the hover tooltip for a commit entry.
fn format_commit_hover(commit: &crate::git::CommitInfo, is_unpushed: bool) -> String {
    let mut lines = Vec::new();
    if is_unpushed {
        lines.push("\u{2B06} Not pushed".to_string());
    }
    lines.push(format!("{} - {}", commit.short_hash, commit.author));
    lines.push(commit.message.clone());
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

// ---------------------------------------------------------------------------
// Free helper functions for file tree rendering (extracted to reduce complexity)
// ---------------------------------------------------------------------------

/// Allocate a full-width clickable row for a file tree entry.
pub(super) fn allocate_tree_row(ui: &mut egui::Ui) -> (egui::Rect, egui::Response) {
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
    let available_width = ui.available_width();
    ui.allocate_exact_size(
        egui::vec2(available_width, row_height),
        egui::Sense::click(),
    )
}

/// Paint a hover highlight behind a tree row if hovered.
pub(super) fn paint_hover_highlight(
    ui: &egui::Ui,
    response: &egui::Response,
    row_rect: egui::Rect,
) {
    if response.hovered() {
        let hover = if ui.visuals().dark_mode {
            egui::Color32::from_white_alpha(15)
        } else {
            egui::Color32::from_black_alpha(12)
        };
        ui.painter().rect_filled(row_rect, 0, hover);
    }
}

/// Determine the display color for a file or directory name.
pub(super) fn entry_name_color(
    ui: &egui::Ui,
    is_ignored: bool,
    is_dirty: bool,
    semantic: &SemanticColors,
) -> egui::Color32 {
    if is_ignored {
        ui.visuals().weak_text_color()
    } else if is_dirty {
        semantic.warning
    } else {
        ui.visuals().text_color()
    }
}

/// Paint a git status badge character right-aligned in the row.
pub(super) fn paint_git_status_badge(
    ui: &egui::Ui,
    row_rect: egui::Rect,
    status_letter: Option<char>,
    semantic: &SemanticColors,
) {
    if let Some(letter) = status_letter {
        let badge_color = match letter {
            'D' => semantic.danger,
            'A' | '?' => semantic.success,
            _ => semantic.warning,
        };
        let badge_text = format!("{}", letter);
        let badge_pos = egui::pos2(row_rect.right() - 14.0, row_rect.center().y);
        ui.painter().text(
            badge_pos,
            egui::Align2::CENTER_CENTER,
            &badge_text,
            egui::FontId::monospace(10.0),
            badge_color,
        );
    }
}

/// Render the context menu for a directory entry.
fn render_dir_context_menu(
    response: &egui::Response,
    entry: &FileEntry,
    project_root: &Path,
    semantic: &SemanticColors,
    action: &mut Option<FileTreeAction>,
    status_msg: &mut Option<String>,
) {
    let entry_path = entry.path.clone();
    let rel_path = entry_path
        .strip_prefix(project_root)
        .unwrap_or(&entry_path)
        .to_string_lossy()
        .to_string();
    let is_ignored = entry.is_ignored;

    response.context_menu(|ui| {
        render_copy_path_items(ui, &entry_path, &rel_path, status_msg);
        ui.separator();
        render_reveal_open_terminal_items(ui, &entry_path, &entry_path, status_msg);
        ui.separator();
        if !is_ignored && ui.button("Add to .gitignore").clicked() {
            *action = Some(FileTreeAction::AddToGitignore(entry_path.clone()));
            ui.close();
        }
        if ui.button("Rename\u{2026}").clicked() {
            *action = Some(FileTreeAction::RenameStart(entry_path.clone()));
            ui.close();
        }
        if ui
            .button(egui::RichText::new("Delete Directory\u{2026}").color(semantic.danger))
            .clicked()
        {
            *action = Some(FileTreeAction::Delete(entry_path.clone(), true));
            ui.close();
        }
    });
}

/// Render the context menu for a file entry.
fn render_file_context_menu(
    response: &egui::Response,
    entry: &FileEntry,
    rel: &str,
    _project_root: &Path,
    semantic: &SemanticColors,
    action: &mut Option<FileTreeAction>,
    status_msg: &mut Option<String>,
) {
    let entry_path = entry.path.clone();
    let rel_clone = rel.to_string();
    let parent_dir = entry_path.parent().unwrap_or(&entry_path).to_path_buf();
    let is_ignored = entry.is_ignored;

    response.context_menu(|ui| {
        render_copy_path_items(ui, &entry_path, &rel_clone, status_msg);
        ui.separator();
        render_reveal_open_terminal_items(ui, &entry_path, &parent_dir, status_msg);
        ui.separator();
        if !is_ignored && ui.button("Add to .gitignore").clicked() {
            *action = Some(FileTreeAction::AddToGitignore(entry_path.clone()));
            ui.close();
        }
        if ui.button("Rename\u{2026}").clicked() {
            *action = Some(FileTreeAction::RenameStart(entry_path.clone()));
            ui.close();
        }
        if ui
            .button(egui::RichText::new("Delete File\u{2026}").color(semantic.danger))
            .clicked()
        {
            *action = Some(FileTreeAction::Delete(entry_path.clone(), false));
            ui.close();
        }
    });
}

/// Render "Copy Path", "Copy Relative Path", "Copy Name", and (for files) "Copy Contents".
fn render_copy_path_items(
    ui: &mut egui::Ui,
    abs_path: &Path,
    rel_path: &str,
    status_msg: &mut Option<String>,
) {
    if ui.button("Copy Name").clicked() {
        let name = abs_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        ui.ctx().copy_text(name);
        ui.close();
    }
    if ui.button("Copy Path").clicked() {
        ui.ctx().copy_text(abs_path.to_string_lossy().to_string());
        ui.close();
    }
    if ui.button("Copy Relative Path").clicked() {
        ui.ctx().copy_text(rel_path.to_string());
        ui.close();
    }
    if abs_path.is_file() {
        const MAX_COPY_SIZE: u64 = 10 * 1024 * 1024; // 10 MB
        let meta = std::fs::metadata(abs_path).ok();
        let file_size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let too_large = file_size > MAX_COPY_SIZE;
        let likely_binary = is_likely_binary(abs_path);

        let enabled = !too_large && !likely_binary;
        let label = if too_large {
            "Copy Contents (file too large)"
        } else if likely_binary {
            "Copy Contents (binary file)"
        } else {
            "Copy Contents"
        };

        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            match std::fs::read_to_string(abs_path) {
                Ok(contents) => ui.ctx().copy_text(contents),
                Err(err) => {
                    *status_msg = Some(format!("Failed to read file: {err}"));
                }
            }
            ui.close();
        }
    }
}

fn is_likely_binary(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    use std::io::Read;
    let mut buf = [0u8; 8192];
    let n = match file.take(8192).read(&mut buf) {
        Ok(n) => n,
        Err(_) => return false,
    };
    buf[..n].contains(&0)
}

/// Render "Reveal in File Manager" and "Open in Terminal" context menu items.
fn render_reveal_open_terminal_items(
    ui: &mut egui::Ui,
    reveal_path: &Path,
    terminal_path: &Path,
    status_msg: &mut Option<String>,
) {
    let reveal_label = if cfg!(target_os = "macos") {
        "Reveal in Finder"
    } else if cfg!(target_os = "windows") {
        "Reveal in Explorer"
    } else {
        "Reveal in File Manager"
    };

    if ui.button(reveal_label).clicked() {
        match spawn_reveal(reveal_path) {
            Ok(_) => ui.close(),
            Err(e) => {
                *status_msg = Some(format!("Failed to reveal: {e}"));
            }
        }
    }
    if ui.button("Open in Terminal").clicked() {
        match spawn_terminal(terminal_path) {
            Ok(_) => ui.close(),
            Err(e) => {
                *status_msg = Some(format!("Failed to open terminal: {e}"));
            }
        }
    }
}

/// Open the system file manager to reveal the given path.
fn spawn_reveal(path: &Path) -> std::io::Result<std::process::Child> {
    if cfg!(target_os = "macos") {
        if path.is_file() {
            std::process::Command::new("open")
                .arg("-R")
                .arg(path)
                .spawn()
        } else {
            std::process::Command::new("open").arg(path).spawn()
        }
    } else if cfg!(target_os = "windows") {
        if path.is_file() {
            std::process::Command::new("explorer")
                .arg(format!("/select,\"{}\"", path.display()))
                .spawn()
        } else {
            std::process::Command::new("explorer").arg(path).spawn()
        }
    } else {
        // Linux / other: xdg-open on the parent directory for files
        let target = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        std::process::Command::new("xdg-open").arg(target).spawn()
    }
}

/// Open a terminal emulator at the given directory.
fn spawn_terminal(dir: &Path) -> std::io::Result<std::process::Child> {
    if cfg!(target_os = "macos") {
        std::process::Command::new("open")
            .args(["-a", "Terminal"])
            .arg(dir)
            .spawn()
    } else if cfg!(target_os = "windows") {
        // Try Windows Terminal first, fall back to cmd.exe
        std::process::Command::new("wt")
            .arg("-d")
            .arg(dir)
            .spawn()
            .or_else(|_| {
                std::process::Command::new("cmd.exe")
                    .args(["/C", "start", "cmd.exe"])
                    .current_dir(dir)
                    .spawn()
            })
    } else {
        // Linux: try common terminals in order of preference
        std::process::Command::new("gnome-terminal")
            .arg(format!("--working-directory={}", dir.display()))
            .spawn()
            .or_else(|_| {
                std::process::Command::new("konsole")
                    .arg(format!("--workdir={}", dir.display()))
                    .spawn()
            })
            .or_else(|_| {
                std::process::Command::new("x-terminal-emulator")
                    .current_dir(dir)
                    .spawn()
            })
            .or_else(|_| std::process::Command::new("xdg-open").arg(dir).spawn())
    }
}
