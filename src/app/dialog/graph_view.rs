use eframe::egui;

use super::super::{DirigentApp, SPACE_SM};
use crate::settings::VcsBackend;

impl DirigentApp {
    pub(in crate::app) fn render_graph_view_central(&mut self, ui: &mut egui::Ui) {
        let fs = self.settings.font_size;
        let log_name = match self.settings.vcs_backend {
            VcsBackend::Jj => "jj Log",
            VcsBackend::Git => "Git Log",
        };

        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Header bar
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
                });
            });
            ui.separator();

            // Precompute graph layout
            let lane_width = 16.0_f32;
            let visible_lanes = self.git.graph_view_max_lanes.clamp(1, 10);
            let extra_for_ellipsis = (self.git.graph_view_max_lanes > 10) as u8 as f32;
            let graph_col_width = (visible_lanes as f32 + extra_for_ellipsis + 0.5) * lane_width;

            let lineage = self
                .git
                .graph_view_hovered_row
                .and_then(|row| compute_graph_view_lineage(&self.git.graph_view_rows, row));

            let row_height = fs * 1.4;
            let num_commits = self.git.graph_view_commits.len();
            let ahead = self.git.ahead_of_remote;
            let lane_colors = self.semantic.lane_colors();
            let is_dark = self.semantic.is_dark();
            let accent = self.semantic.accent;
            let warning = self.semantic.warning;

            let mut new_hovered: Option<usize> = None;
            let mut clicked_commit: Option<(String, String, String, String)> = None;
            let mut load_more = false;

            egui::ScrollArea::vertical()
                .id_salt("graph_view_scroll")
                .auto_shrink([false, false])
                .show_rows(ui, row_height, num_commits + 1, |ui, row_range| {
                    let avail_width = ui.available_width();

                    for idx in row_range {
                        if idx >= num_commits {
                            // "Load More" button at the end
                            ui.add_space(SPACE_SM);
                            if ui
                                .button(
                                    egui::RichText::new("Load More\u{2026}")
                                        .color(ui.visuals().hyperlink_color),
                                )
                                .clicked()
                            {
                                load_more = true;
                            }
                            continue;
                        }

                        let commit = &self.git.graph_view_commits[idx];
                        let graph_row = self.git.graph_view_rows.get(idx);
                        let is_unpushed = idx < ahead;

                        let highlight_lane = lineage
                            .as_ref()
                            .filter(|l| l.rows.get(idx).copied().unwrap_or(false))
                            .map(|l| l.lane);
                        let any_highlight_active = lineage.is_some();

                        // Allocate row
                        let (row_rect, response) = ui.allocate_exact_size(
                            egui::vec2(avail_width, row_height),
                            egui::Sense::click(),
                        );

                        // Row background highlights
                        if highlight_lane.is_some() {
                            let tint = if is_dark {
                                egui::Color32::from_white_alpha(8)
                            } else {
                                egui::Color32::from_black_alpha(6)
                            };
                            ui.painter().rect_filled(row_rect, 0, tint);
                        }
                        if response.hovered() {
                            let hover = if is_dark {
                                egui::Color32::from_white_alpha(15)
                            } else {
                                egui::Color32::from_black_alpha(12)
                            };
                            ui.painter().rect_filled(row_rect, 0, hover);
                        }

                        // Paint graph lanes
                        if let Some(graph) = graph_row {
                            paint_graph_view_column(
                                ui,
                                row_rect,
                                graph,
                                row_height,
                                lane_width,
                                10,
                                &lane_colors,
                                highlight_lane,
                                any_highlight_active,
                            );
                        }

                        // Commit text
                        let text_x = row_rect.left() + graph_col_width + 4.0;
                        let text_y = row_rect.center().y;

                        let text_avail = avail_width - graph_col_width - 8.0;
                        let char_width = fs * 0.52;
                        let max_msg_chars = ((text_avail / char_width) as usize)
                            .saturating_sub(30)
                            .max(20);

                        // Short hash
                        let hash_color = ui.visuals().weak_text_color();
                        let hash_font = egui::FontId::monospace(fs * 0.8);
                        ui.painter().text(
                            egui::pos2(text_x, text_y),
                            egui::Align2::LEFT_CENTER,
                            &commit.short_hash,
                            hash_font.clone(),
                            hash_color,
                        );

                        // Message
                        let msg_x = text_x + char_width * 9.0;
                        let msg = if commit.message.len() > max_msg_chars + 3 {
                            format!("{}...", &commit.message[..max_msg_chars])
                        } else {
                            commit.message.clone()
                        };

                        let dot = if is_unpushed { "\u{25CF} " } else { "" };
                        let text_color = if commit.is_working_copy {
                            accent
                        } else if is_unpushed {
                            ui.visuals().warn_fg_color
                        } else {
                            ui.visuals().text_color()
                        };
                        let msg_font = egui::FontId::monospace(fs * 0.8);
                        ui.painter().text(
                            egui::pos2(msg_x, text_y),
                            egui::Align2::LEFT_CENTER,
                            format!("{}{}", dot, msg),
                            msg_font.clone(),
                            text_color,
                        );

                        // Author + time
                        let meta_x = msg_x + char_width * (max_msg_chars as f32 + 6.0);
                        if meta_x < row_rect.right() - 40.0 {
                            let meta = format!("{} \u{00B7} {}", commit.author, commit.time_ago);
                            ui.painter().text(
                                egui::pos2(meta_x, text_y),
                                egui::Align2::LEFT_CENTER,
                                meta,
                                egui::FontId::proportional(fs * 0.7),
                                ui.visuals().weak_text_color(),
                            );
                        }

                        // Branch/tag badges
                        paint_graph_view_badges(
                            ui,
                            commit,
                            msg_x,
                            text_y,
                            &format!("{}{}", dot, msg),
                            &msg_font,
                            text_color,
                            &lane_colors,
                            warning,
                        );

                        // Hover tooltip
                        let hover_text = format_graph_view_hover(commit, is_unpushed);
                        if response.hovered() {
                            new_hovered = Some(idx);
                        }
                        if response.on_hover_text(hover_text).clicked() {
                            clicked_commit = Some((
                                commit.full_hash.clone(),
                                commit.message.clone(),
                                commit.body.clone(),
                                commit.author.clone(),
                            ));
                        }
                    }
                });

            self.git.graph_view_hovered_row = new_hovered;

            if load_more {
                self.git.graph_view_limit += 100;
                self.reload_graph_view_history();
            }

            if let Some((full_hash, message, body, author)) = clicked_commit {
                self.git.show_graph_view = false;
                self.open_commit_diff_review(&full_hash, &message, body, &author);
            }
        });
    }
}

struct GraphViewLineage {
    rows: Vec<bool>,
    lane: usize,
}

fn compute_graph_view_lineage(
    graph_rows: &[crate::git::graph::GraphRow],
    hovered_row: usize,
) -> Option<GraphViewLineage> {
    let graph = graph_rows.get(hovered_row)?;
    let lane = graph.column;
    let num = graph_rows.len();
    let mut rows = vec![false; num];
    rows[hovered_row] = true;

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

    Some(GraphViewLineage { rows, lane })
}

fn paint_graph_view_column(
    ui: &egui::Ui,
    row_rect: egui::Rect,
    graph: &crate::git::graph::GraphRow,
    _row_height: f32,
    lane_width: f32,
    max_visible_lanes: usize,
    lane_colors: &[egui::Color32; 6],
    highlight_lane: Option<usize>,
    any_highlight_active: bool,
) {
    use crate::git::graph::LaneSegment;

    let painter = ui.painter();
    let graph_left = row_rect.left();
    let top = row_rect.top();
    let bot = row_rect.bottom();
    let mid_y = row_rect.center().y;
    let base_line_width = 1.8_f32;
    let clamped_column = graph.column.min(max_visible_lanes);
    let commit_x = graph_left + (clamped_column as f32 + 0.5) * lane_width;

    for (lane_idx, segment) in graph.lanes.iter().take(max_visible_lanes).enumerate() {
        let x = graph_left + (lane_idx as f32 + 0.5) * lane_width;
        let base_color = lane_colors[lane_idx % lane_colors.len()];
        let is_hl = highlight_lane == Some(lane_idx);
        let (color, line_width) = if !any_highlight_active {
            (base_color, base_line_width)
        } else if is_hl {
            (base_color, base_line_width * 1.6)
        } else {
            (base_color.linear_multiply(0.25), base_line_width)
        };
        let stroke = egui::Stroke::new(line_width, color);

        match segment {
            LaneSegment::Straight => {
                painter.line_segment([egui::pos2(x, top), egui::pos2(x, bot)], stroke);
            }
            LaneSegment::Commit => {
                painter.line_segment([egui::pos2(x, top), egui::pos2(x, mid_y - 3.5)], stroke);
                painter.line_segment([egui::pos2(x, mid_y + 3.5), egui::pos2(x, bot)], stroke);
                let dot_radius = if is_hl { 4.5 } else { 3.5 };
                painter.circle_filled(egui::pos2(x, mid_y), dot_radius, color);
            }
            LaneSegment::ForkRight => {
                painter.line_segment([egui::pos2(commit_x, mid_y), egui::pos2(x, mid_y)], stroke);
                painter.line_segment([egui::pos2(x, mid_y), egui::pos2(x, bot)], stroke);
            }
            LaneSegment::MergeLeft => {
                painter.line_segment([egui::pos2(x, top), egui::pos2(x, mid_y)], stroke);
                painter.line_segment([egui::pos2(x, mid_y), egui::pos2(commit_x, mid_y)], stroke);
            }
            LaneSegment::Empty => {}
        }
    }

    // Overflow ellipsis
    let has_overflow = graph
        .lanes
        .iter()
        .skip(max_visible_lanes)
        .any(|s| *s != LaneSegment::Empty);
    if has_overflow {
        let ellipsis_x = graph_left + (max_visible_lanes as f32 + 0.5) * lane_width;
        if graph.column >= max_visible_lanes {
            let dot_color = lane_colors[graph.column % lane_colors.len()];
            painter.circle_filled(egui::pos2(ellipsis_x, mid_y), 3.5, dot_color);
        } else {
            let color = ui.visuals().text_color().gamma_multiply(0.4);
            painter.text(
                egui::pos2(ellipsis_x, mid_y),
                egui::Align2::CENTER_CENTER,
                "\u{00B7}\u{00B7}\u{00B7}",
                egui::FontId::monospace(12.0),
                color,
            );
        }
    }

    // Connections
    for &(from_lane, to_lane) in &graph.connections {
        let vis_from = from_lane.min(max_visible_lanes);
        let vis_to = to_lane.min(max_visible_lanes);
        if vis_from == vis_to {
            continue;
        }
        let is_merge_or_fork = |lane: usize| -> bool {
            lane < max_visible_lanes
                && matches!(
                    graph.lanes.get(lane),
                    Some(&LaneSegment::ForkRight) | Some(&LaneSegment::MergeLeft)
                )
        };
        if is_merge_or_fork(from_lane) || is_merge_or_fork(to_lane) {
            continue;
        }
        let from_x = graph_left + (vis_from as f32 + 0.5) * lane_width;
        let to_x = graph_left + (vis_to as f32 + 0.5) * lane_width;
        let color_idx = to_lane.min(lane_colors.len() - 1);
        let base_color = lane_colors[color_idx % lane_colors.len()];
        let conn_hl = highlight_lane == Some(from_lane) || highlight_lane == Some(to_lane);
        let (color, conn_width) = if !any_highlight_active {
            (base_color, base_line_width)
        } else if conn_hl {
            (base_color, base_line_width * 1.6)
        } else {
            (base_color.linear_multiply(0.25), base_line_width)
        };
        painter.line_segment(
            [egui::pos2(from_x, mid_y), egui::pos2(to_x, mid_y)],
            egui::Stroke::new(conn_width, color),
        );
    }
}

fn paint_graph_view_badges(
    ui: &egui::Ui,
    commit: &crate::git::CommitInfo,
    text_x: f32,
    text_y: f32,
    label: &str,
    label_font: &egui::FontId,
    text_color: egui::Color32,
    lane_colors: &[egui::Color32; 6],
    warning_color: egui::Color32,
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
        badge_x = paint_badge(ui, badge_x, text_y, branch_name, color, &badge_font);
    }
    for tag_name in &commit.tag_labels {
        badge_x = paint_badge(ui, badge_x, text_y, tag_name, warning_color, &badge_font);
    }
    let _ = badge_x;
}

fn paint_badge(
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

fn format_graph_view_hover(commit: &crate::git::CommitInfo, is_unpushed: bool) -> String {
    let mut lines = Vec::new();
    if is_unpushed {
        lines.push("\u{2B06} Not pushed".to_string());
    }
    lines.push(format!("{} - {}", commit.short_hash, commit.author));
    lines.push(commit.message.clone());
    if !commit.body.is_empty() {
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
