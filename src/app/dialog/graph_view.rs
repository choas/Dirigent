use eframe::egui;

use super::super::graph_helpers::{self, GraphPaintCtx};
use super::super::DirigentApp;
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

            let mut new_hovered: Option<usize> = None;
            let mut clicked_commit: Option<(String, String, String, String)> = None;
            let mut load_more = false;

            egui::ScrollArea::vertical()
                .id_salt("graph_view_scroll")
                .auto_shrink([false, false])
                .show_rows(ui, row_height, num_commits, |ui, row_range| {
                    let avail_width = ui.available_width();

                    if row_range.end + 20 >= num_commits && num_commits >= self.git.graph_view_limit
                    {
                        load_more = true;
                    }

                    for idx in row_range {
                        if idx >= num_commits {
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
                            let gctx = GraphPaintCtx {
                                graph_left: row_rect.left(),
                                lane_width,
                                base_line_width: 1.8,
                                dot_radius: 3.5,
                                ellipsis_font_size: 12.0,
                                max_visible_lanes: 10,
                                lane_colors: &lane_colors,
                                highlight_lane,
                                any_highlight_active,
                            };
                            graph_helpers::paint_graph_column(ui, row_rect, graph, &gctx);
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
                            format!(
                                "{}...",
                                crate::app::truncate_str(&commit.message, max_msg_chars)
                            )
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
                        graph_helpers::paint_commit_badges(
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
                        let hover_text =
                            graph_helpers::format_commit_hover(commit, is_unpushed, true);
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
