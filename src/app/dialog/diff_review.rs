use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_LG};
use crate::agents::AgentTrigger;
use crate::db::CueStatus;
use crate::diff_view::{self, DiffSearchHighlight, DiffViewMode};
use crate::git;

impl DirigentApp {
    // Diff review rendered in the central panel (replaces code viewer)
    pub(in crate::app) fn render_diff_review_central(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut accept = false;
        let mut reject = false;
        let mut reply_send: Option<String> = None;
        let mut toggle_mode = None;
        let fs = self.settings.font_size;
        let sem = self.semantic;

        let review = self.diff_review.as_mut().unwrap();
        let cue_id = review.cue_id;
        let diff_text = review.diff.clone();
        let cue_text = review.cue_text.clone();
        let parsed = review.parsed.clone();
        let view_mode = review.view_mode;
        let read_only = review.read_only;
        let prompt_expanded = review.prompt_expanded;
        let search_active = review.search_active;
        let reply_text = &mut review.reply_text;
        let collapsed_files = &mut review.collapsed_files;
        let search_query = &mut review.search_query;
        let search_matches = &review.search_matches;
        let search_current = review.search_current;

        let mut toggle_prompt = false;
        let mut search_changed = false;
        let mut search_next = false;
        let mut search_prev = false;
        let mut search_close = false;

        egui::CentralPanel::default().show(ctx, |ui| {
            // Header bar
            let prefix = if read_only { "Commit" } else { "Cue" };
            ui.horizontal(|ui| {
                if ui.button(icon("\u{2190} Back", fs)).clicked() {
                    close = true;
                }
                ui.separator();
                ui.strong("Diff Review");
                ui.separator();
                let arrow = if prompt_expanded {
                    "\u{25BC}"
                } else {
                    "\u{25B6}"
                };
                if ui
                    .button(icon(&format!("{} {}", arrow, prefix), fs))
                    .on_hover_text(if prompt_expanded {
                        "Hide prompt"
                    } else {
                        "Show prompt"
                    })
                    .clicked()
                {
                    toggle_prompt = true;
                }
                if !prompt_expanded {
                    let truncated = if cue_text.len() > 80 {
                        format!("{}...", crate::app::truncate_str(&cue_text, 77))
                    } else {
                        cue_text.clone()
                    };
                    ui.label(egui::RichText::new(truncated).color(self.semantic.secondary_text));
                }
            });
            if prompt_expanded {
                ui.group(|ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("prompt_scroll")
                        .max_height(150.0)
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(&cue_text).color(self.semantic.secondary_text),
                            );
                        });
                });
            }
            ui.separator();

            // View mode toggle + action buttons
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(view_mode == DiffViewMode::Inline, "Inline")
                    .clicked()
                {
                    toggle_mode = Some(DiffViewMode::Inline);
                }
                if ui
                    .selectable_label(view_mode == DiffViewMode::SideBySide, "Side-by-Side")
                    .clicked()
                {
                    toggle_mode = Some(DiffViewMode::SideBySide);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !read_only {
                        if ui
                            .button(icon("\u{21BA} Revert", fs).color(self.semantic.danger))
                            .on_hover_text("Revert changes back to previous state")
                            .clicked()
                        {
                            reject = true;
                        }
                        if ui
                            .button(icon("\u{2713} Commit", fs).color(self.semantic.success))
                            .on_hover_text("Commit the applied changes")
                            .clicked()
                        {
                            accept = true;
                        }
                    }
                });
            });
            // Reply field (only for actionable reviews)
            if !read_only {
                ui.horizontal(|ui| {
                    let te = ui.add(
                        egui::TextEdit::singleline(reply_text)
                            .desired_width(ui.available_width() - 80.0)
                            .hint_text("Reply with feedback..."),
                    );
                    let send = ui
                        .button(icon("\u{21A9} Reply", fs))
                        .on_hover_text("Send feedback to Claude for another iteration")
                        .clicked()
                        || (te.has_focus()
                            && ui
                                .input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command));
                    if send && !reply_text.trim().is_empty() {
                        reply_send = Some(reply_text.clone());
                    }
                });
            }

            // Search bar
            if search_active {
                ui.horizontal(|ui| {
                    ui.label("Find:");
                    let response = ui.add(
                        egui::TextEdit::singleline(search_query)
                            .desired_width(250.0)
                            .hint_text("Search in diff...")
                            .font(egui::TextStyle::Monospace),
                    );
                    response.request_focus();

                    if response.changed() {
                        search_changed = true;
                    }

                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        if ui.input(|i| i.modifiers.shift) {
                            search_prev = true;
                        } else {
                            search_next = true;
                        }
                        response.request_focus();
                    }

                    let match_count = search_matches.len();
                    if !search_query.is_empty() {
                        let label = if match_count == 0 {
                            "No matches".to_string()
                        } else {
                            let current = search_current.map(|i| i + 1).unwrap_or(0);
                            format!("{}/{}", current, match_count)
                        };
                        ui.label(egui::RichText::new(label).monospace().small().color(
                            if match_count == 0 {
                                self.semantic.danger
                            } else {
                                self.semantic.secondary_text
                            },
                        ));
                    }

                    if ui
                        .small_button(icon("\u{2191}", fs))
                        .on_hover_text("Previous (Shift+Enter)")
                        .clicked()
                    {
                        search_prev = true;
                    }
                    if ui
                        .small_button(icon("\u{2193}", fs))
                        .on_hover_text("Next (Enter)")
                        .clicked()
                    {
                        search_next = true;
                    }
                    if ui
                        .small_button(icon("\u{2715}", fs))
                        .on_hover_text("Close (Esc)")
                        .clicked()
                    {
                        search_close = true;
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        search_close = true;
                    }
                });
            }
            ui.separator();

            // Build search highlight for diff rendering
            let query_lower_owned = search_query.to_lowercase();
            let search_highlight = if search_active && !search_query.is_empty() {
                let current = search_current.map(|idx| search_matches[idx]);
                Some(DiffSearchHighlight {
                    query_lower: &query_lower_owned,
                    current,
                })
            } else {
                None
            };

            // Diff content fills the rest
            egui::ScrollArea::both()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    if parsed.files.is_empty() {
                        ui.add_space(SPACE_LG);
                        ui.label(
                            egui::RichText::new("No file changes in this commit.")
                                .italics()
                                .color(self.semantic.secondary_text),
                        );
                    } else {
                        match view_mode {
                            DiffViewMode::Inline => {
                                diff_view::render_inline_diff(
                                    ui,
                                    &parsed,
                                    collapsed_files,
                                    search_highlight.as_ref(),
                                    &sem,
                                );
                            }
                            DiffViewMode::SideBySide => {
                                diff_view::render_side_by_side_diff(
                                    ui,
                                    &parsed,
                                    collapsed_files,
                                    search_highlight.as_ref(),
                                    &sem,
                                );
                            }
                        }
                    }
                });
        });

        if let Some(mode) = toggle_mode {
            if let Some(ref mut review) = self.diff_review {
                review.view_mode = mode;
            }
        }
        if toggle_prompt {
            if let Some(ref mut review) = self.diff_review {
                review.prompt_expanded = !review.prompt_expanded;
            }
        }

        // Handle diff search state changes
        if search_changed {
            if let Some(ref mut review) = self.diff_review {
                crate::app::search::update_diff_search_matches(review);
                // Expand collapsed files that contain the current match
                if let Some(idx) = review.search_current {
                    let (file_idx, _, _) = review.search_matches[idx];
                    review.collapsed_files.remove(&file_idx);
                }
            }
        }
        if search_next {
            if let Some(ref mut review) = self.diff_review {
                if !review.search_matches.is_empty() {
                    let next = match review.search_current {
                        Some(i) => (i + 1) % review.search_matches.len(),
                        None => 0,
                    };
                    review.search_current = Some(next);
                    let (file_idx, _, _) = review.search_matches[next];
                    review.collapsed_files.remove(&file_idx);
                }
            }
        }
        if search_prev {
            if let Some(ref mut review) = self.diff_review {
                if !review.search_matches.is_empty() {
                    let prev = match review.search_current {
                        Some(0) => review.search_matches.len() - 1,
                        Some(i) => i - 1,
                        None => 0,
                    };
                    review.search_current = Some(prev);
                    let (file_idx, _, _) = review.search_matches[prev];
                    review.collapsed_files.remove(&file_idx);
                }
            }
        }
        if search_close {
            if let Some(ref mut review) = self.diff_review {
                review.search_active = false;
                review.search_query.clear();
                review.search_matches.clear();
                review.search_current = None;
            }
        }

        if accept {
            let commit_msg = git::generate_commit_message(&cue_text);
            match git::commit_diff(&self.project_root, &diff_text, &commit_msg) {
                Ok(hash) => {
                    let short = &hash[..7.min(hash.len())];
                    self.set_status_message(format!("Committed: {}", short));
                    let _ = self.db.update_cue_status(cue_id, CueStatus::Done);
                    // Trigger post-commit agents (format, lint, etc.)
                    let cue_prompt = self
                        .cues
                        .iter()
                        .find(|c| c.id == cue_id)
                        .map(|c| c.text.clone())
                        .unwrap_or_default();
                    self.trigger_agents_for(&AgentTrigger::AfterCommit, Some(cue_id), &cue_prompt);
                }
                Err(e) => {
                    self.set_status_message(format!("Commit failed: {}", e));
                }
            }
            self.reload_cues();
            self.reload_git_info();
            self.reload_commit_history();
            self.diff_review = None;
        } else if reject {
            let file_paths = git::parse_diff_file_paths_for_repo(&self.project_root, &diff_text);
            if let Err(e) = git::revert_files(&self.project_root, &file_paths) {
                self.set_status_message(format!("Revert failed: {}", e));
            }
            let _ = self.db.update_cue_status(cue_id, CueStatus::Inbox);
            if let Some(ref path) = self.viewer.current_file {
                let p = path.clone();
                self.load_file(p);
            }
            self.reload_cues();
            self.reload_git_info();
            self.diff_review = None;
        } else if let Some(reply) = reply_send {
            self.trigger_claude_reply(cue_id, &reply, &[]);
        } else if close {
            self.diff_review = None;
        }
    }
}
