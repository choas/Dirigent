use eframe::egui;

use super::super::graph_helpers::{self, GraphPaintCtx};
use super::super::DirigentApp;
use crate::settings::VcsBackend;

#[derive(Default)]
struct GraphViewActions {
    hovered_row: Option<usize>,
    clicked_commit: Option<(String, String, String, String)>,
    load_more: bool,
}

struct GraphViewLayout<'a> {
    fs: f32,
    row_height: f32,
    lane_width: f32,
    graph_col_width: f32,
    ahead: usize,
    limit: usize,
    lane_colors: &'a [egui::Color32; 6],
    is_dark: bool,
    accent: egui::Color32,
    warning: egui::Color32,
}

struct GraphViewRowData<'a> {
    idx: usize,
    commit: &'a crate::git::CommitInfo,
    graph_row: Option<&'a crate::git::graph::GraphRow>,
    is_unpushed: bool,
    highlight_lane: Option<usize>,
    any_highlight_active: bool,
}

struct GraphTextPaint {
    msg_x: f32,
    text_y: f32,
    label: String,
    msg_font: egui::FontId,
    text_color: egui::Color32,
}

struct GraphMetaPaint {
    row_rect: egui::Rect,
    msg_x: f32,
    text_y: f32,
    char_width: f32,
    max_msg_chars: usize,
}

impl DirigentApp {
    pub(in crate::app) fn render_graph_view_central(&mut self, ui: &mut egui::Ui) {
        let fs = self.settings.font_size;
        let log_name = match self.settings.vcs_backend {
            VcsBackend::Jj => "jj Log",
            VcsBackend::Git => "Git Log",
        };

        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.render_graph_view_header(ui, fs, log_name);

            // Precompute graph layout
            let lane_width = 16.0_f32;
            let visible_lanes = self.git.graph_view_max_lanes.clamp(1, 10);
            let extra_for_ellipsis = (self.git.graph_view_max_lanes > 10) as u8 as f32;
            let graph_col_width = (visible_lanes as f32 + extra_for_ellipsis + 0.5) * lane_width;

            let lineage = self.git.graph_view_hovered_row.and_then(|row| {
                graph_helpers::compute_branch_lineage(&self.git.graph_view_rows, row)
            });

            let row_height = fs * 1.4;
            let num_commits = self.git.graph_view_commits.len();
            let ahead = self.git.ahead_of_remote;
            let lane_colors = self.semantic.lane_colors();
            let is_dark = self.semantic.is_dark();
            let accent = self.semantic.accent;
            let warning = self.semantic.warning;
            let layout = GraphViewLayout {
                fs,
                row_height,
                lane_width,
                graph_col_width,
                ahead,
                limit: self.git.graph_view_limit,
                lane_colors: &lane_colors,
                is_dark,
                accent,
                warning,
            };

            let mut actions = GraphViewActions::default();

            egui::ScrollArea::vertical()
                .id_salt("graph_view_scroll")
                .auto_shrink([false, false])
                .show_rows(ui, row_height, num_commits, |ui, row_range| {
                    render_graph_view_rows(
                        ui,
                        row_range,
                        &self.git.graph_view_commits,
                        &self.git.graph_view_rows,
                        &lineage,
                        &layout,
                        &mut actions,
                    );
                });

            self.git.graph_view_hovered_row = actions.hovered_row;

            if actions.load_more {
                self.git.graph_view_limit += 100;
                self.reload_graph_view_history();
            }

            if let Some((full_hash, message, body, author)) = actions.clicked_commit {
                self.git.show_graph_view = false;
                self.open_commit_diff_review(&full_hash, &message, body, &author);
            }
        });
    }

    fn render_graph_view_header(&mut self, ui: &mut egui::Ui, fs: f32, log_name: &str) {
        ui.horizontal(|ui| {
            ui.heading(
                egui::RichText::new(format!(
                    "\u{1F4CA} {} \u{2014} {} commits",
                    log_name,
                    self.git.graph_view_commits.len()
                ))
                .size(fs * 1.1),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    self.git.show_graph_view = false;
                }
                // Cross-branch change-set review, mirroring the Git View panel
                // header. Git only; jj has no such range diff here.
                if self.settings.vcs_backend == VcsBackend::Git {
                    let analyzing = self.analyze_over_op.in_flight();
                    let label = if analyzing { "\u{29D7}" } else { "\u{21C4}" };
                    if ui
                        .add_enabled(!analyzing, egui::Button::new(label).small())
                        .on_hover_text(
                            "Analyze changes over… (group a branch's changes for review)",
                        )
                        .clicked()
                    {
                        self.open_analyze_over_picker();
                    }
                }
            });
        });
        ui.separator();
    }
}

fn render_graph_view_rows(
    ui: &mut egui::Ui,
    row_range: std::ops::Range<usize>,
    commits: &[crate::git::CommitInfo],
    rows: &[crate::git::graph::GraphRow],
    lineage: &Option<graph_helpers::BranchLineage>,
    layout: &GraphViewLayout<'_>,
    actions: &mut GraphViewActions,
) {
    if row_range.end + 20 >= commits.len() && commits.len() >= layout.limit {
        actions.load_more = true;
    }

    for idx in row_range.filter(|idx| *idx < commits.len()) {
        let row = graph_view_row_data(idx, commits, rows, lineage, layout);
        render_graph_view_row(ui, row, layout, actions);
    }
}

fn graph_view_row_data<'a>(
    idx: usize,
    commits: &'a [crate::git::CommitInfo],
    rows: &'a [crate::git::graph::GraphRow],
    lineage: &Option<graph_helpers::BranchLineage>,
    layout: &GraphViewLayout<'_>,
) -> GraphViewRowData<'a> {
    let highlight_lane = lineage
        .as_ref()
        .filter(|l| l.rows.get(idx).copied().unwrap_or(false))
        .map(|l| l.lane);
    GraphViewRowData {
        idx,
        commit: &commits[idx],
        graph_row: rows.get(idx),
        is_unpushed: idx < layout.ahead,
        highlight_lane,
        any_highlight_active: lineage.is_some(),
    }
}

fn render_graph_view_row(
    ui: &mut egui::Ui,
    row: GraphViewRowData<'_>,
    layout: &GraphViewLayout<'_>,
    actions: &mut GraphViewActions,
) {
    let avail_width = ui.available_width();
    let (row_rect, response) = ui.allocate_exact_size(
        egui::vec2(avail_width, layout.row_height),
        egui::Sense::click(),
    );

    graph_helpers::paint_graph_row_background(
        ui,
        row_rect,
        response.hovered(),
        layout.is_dark,
        row.highlight_lane,
    );
    paint_graph_view_column(ui, row_rect, &row, layout);
    let text = paint_graph_view_commit_text(ui, row_rect, avail_width, &row, layout);
    graph_helpers::paint_commit_badges(
        ui,
        row.commit,
        text.msg_x,
        text.text_y,
        &text.label,
        &text.msg_font,
        text.text_color,
        layout.lane_colors,
        layout.warning,
    );
    apply_graph_view_row_response(response, &row, actions);
}

fn paint_graph_view_column(
    ui: &egui::Ui,
    row_rect: egui::Rect,
    row: &GraphViewRowData<'_>,
    layout: &GraphViewLayout<'_>,
) {
    let Some(graph) = row.graph_row else {
        return;
    };
    let gctx = GraphPaintCtx {
        graph_left: row_rect.left(),
        lane_width: layout.lane_width,
        base_line_width: 1.8,
        dot_radius: 3.5,
        ellipsis_font_size: 12.0,
        max_visible_lanes: 10,
        lane_colors: layout.lane_colors,
        highlight_lane: row.highlight_lane,
        any_highlight_active: row.any_highlight_active,
    };
    graph_helpers::paint_graph_column(ui, row_rect, graph, &gctx);
}

fn paint_graph_view_commit_text(
    ui: &egui::Ui,
    row_rect: egui::Rect,
    avail_width: f32,
    row: &GraphViewRowData<'_>,
    layout: &GraphViewLayout<'_>,
) -> GraphTextPaint {
    let text_x = row_rect.left() + layout.graph_col_width + 4.0;
    let text_y = row_rect.center().y;
    let char_width = layout.fs * 0.52;
    let max_msg_chars = graph_view_max_msg_chars(avail_width, layout.graph_col_width, char_width);

    paint_graph_view_hash(ui, row.commit, text_x, text_y, layout.fs);

    let msg_x = text_x + char_width * 9.0;
    let msg = truncate_graph_message(&row.commit.message, max_msg_chars);
    let dot = if row.is_unpushed { "\u{25CF} " } else { "" };
    let label = format!("{}{}", dot, msg);
    let text_color = graph_commit_text_color(ui, row, layout);
    let msg_font = egui::FontId::monospace(layout.fs * 0.8);
    ui.painter().text(
        egui::pos2(msg_x, text_y),
        egui::Align2::LEFT_CENTER,
        &label,
        msg_font.clone(),
        text_color,
    );
    let meta = GraphMetaPaint {
        row_rect,
        msg_x,
        text_y,
        char_width,
        max_msg_chars,
    };
    paint_graph_view_meta(ui, row.commit, &meta, layout.fs);

    GraphTextPaint {
        msg_x,
        text_y,
        label,
        msg_font,
        text_color,
    }
}

fn graph_view_max_msg_chars(avail_width: f32, graph_col_width: f32, char_width: f32) -> usize {
    let text_avail = avail_width - graph_col_width - 8.0;
    ((text_avail / char_width) as usize)
        .saturating_sub(30)
        .max(20)
}

fn paint_graph_view_hash(
    ui: &egui::Ui,
    commit: &crate::git::CommitInfo,
    text_x: f32,
    text_y: f32,
    fs: f32,
) {
    ui.painter().text(
        egui::pos2(text_x, text_y),
        egui::Align2::LEFT_CENTER,
        &commit.short_hash,
        egui::FontId::monospace(fs * 0.8),
        ui.visuals().weak_text_color(),
    );
}

fn truncate_graph_message(message: &str, max_msg_chars: usize) -> String {
    if message.len() > max_msg_chars + 3 {
        format!("{}...", crate::app::truncate_str(message, max_msg_chars))
    } else {
        message.to_string()
    }
}

fn graph_commit_text_color(
    ui: &egui::Ui,
    row: &GraphViewRowData<'_>,
    layout: &GraphViewLayout<'_>,
) -> egui::Color32 {
    if row.commit.is_working_copy {
        layout.accent
    } else if row.is_unpushed {
        ui.visuals().warn_fg_color
    } else {
        ui.visuals().text_color()
    }
}

fn paint_graph_view_meta(
    ui: &egui::Ui,
    commit: &crate::git::CommitInfo,
    meta: &GraphMetaPaint,
    fs: f32,
) {
    let meta_x = meta.msg_x + meta.char_width * (meta.max_msg_chars as f32 + 6.0);
    if meta_x >= meta.row_rect.right() - 40.0 {
        return;
    }
    let label = format!("{} \u{00B7} {}", commit.author, commit.time_ago);
    ui.painter().text(
        egui::pos2(meta_x, meta.text_y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(fs * 0.7),
        ui.visuals().weak_text_color(),
    );
}

fn apply_graph_view_row_response(
    response: egui::Response,
    row: &GraphViewRowData<'_>,
    actions: &mut GraphViewActions,
) {
    let hover_text = graph_helpers::format_commit_hover(row.commit, row.is_unpushed, true);
    if response.hovered() {
        actions.hovered_row = Some(row.idx);
    }
    if response.on_hover_text(hover_text).clicked() {
        actions.clicked_commit = Some((
            row.commit.full_hash.clone(),
            row.commit.message.clone(),
            row.commit.body.clone(),
            row.commit.author.clone(),
        ));
    }
}
