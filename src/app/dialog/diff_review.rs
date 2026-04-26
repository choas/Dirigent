use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_LG};
use crate::agents::AgentTrigger;
use crate::db::CueStatus;
use crate::diff_view::{self, DiffSearchHighlight, DiffViewMode};
use crate::git;
use crate::settings::SemanticColors;

/// Read-only search state shared across diff rendering functions.
struct SearchState<'a> {
    active: bool,
    matches: &'a [(usize, usize, usize)],
    current: Option<usize>,
}

/// Context for the diff review header bar.
struct DiffHeaderContext<'a> {
    read_only: bool,
    prompt_expanded: bool,
    cue_text: &'a str,
    commit_hash: Option<&'a str>,
}

/// Actions collected during UI rendering, applied after the closure ends.
struct DiffReviewActions {
    close: bool,
    accept: bool,
    reject: bool,
    reply_send: Option<String>,
    toggle_mode: Option<DiffViewMode>,
    toggle_prompt: bool,
    search_changed: bool,
    search_next: bool,
    search_prev: bool,
    search_close: bool,
}

impl DiffReviewActions {
    fn new() -> Self {
        Self {
            close: false,
            accept: false,
            reject: false,
            reply_send: None,
            toggle_mode: None,
            toggle_prompt: false,
            search_changed: false,
            search_next: false,
            search_prev: false,
            search_close: false,
        }
    }
}

impl DirigentApp {
    // Diff review rendered in the central panel (replaces code viewer)
    pub(in crate::app) fn render_diff_review_central(&mut self, ui: &mut egui::Ui) {
        let mut actions = DiffReviewActions::new();
        let fs = self.settings.font_size;
        let sem = self.semantic;

        let Some(review) = self.diff_review.as_mut() else {
            return;
        };
        let cue_id = review.cue_id;
        let diff_text = review.diff.clone();
        let cue_text = review.cue_text.clone();
        let commit_hash = review.commit_hash.clone();
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

        egui::CentralPanel::default().show_inside(ui, |ui| {
            Self::render_diff_header_bar(
                ui,
                fs,
                &sem,
                &DiffHeaderContext {
                    read_only,
                    prompt_expanded,
                    cue_text: &cue_text,
                    commit_hash: commit_hash.as_deref(),
                },
                &mut actions,
            );
            Self::render_diff_prompt_section(
                ui,
                &sem,
                prompt_expanded,
                &cue_text,
                commit_hash.as_deref(),
            );
            ui.separator();

            Self::render_diff_view_mode_toolbar(ui, fs, &sem, view_mode, read_only, &mut actions);
            Self::render_diff_reply_field(ui, fs, read_only, reply_text, &mut actions);
            let search = SearchState {
                active: search_active,
                matches: search_matches,
                current: search_current,
            };
            Self::render_diff_search_bar(ui, fs, &sem, search_query, &search, &mut actions);
            ui.separator();

            Self::render_diff_content(
                ui,
                &sem,
                &parsed,
                view_mode,
                search_query,
                &search,
                collapsed_files,
            );
        });

        self.apply_diff_review_state_updates(&actions);
        self.apply_diff_search_actions(&actions);
        self.apply_diff_review_actions(actions, cue_id, &diff_text, &cue_text);
    }

    fn render_diff_header_bar(
        ui: &mut egui::Ui,
        fs: f32,
        sem: &SemanticColors,
        ctx: &DiffHeaderContext<'_>,
        actions: &mut DiffReviewActions,
    ) {
        let prefix = if ctx.read_only { "Message" } else { "Cue" };
        let arrow = if ctx.prompt_expanded {
            "\u{25BC}"
        } else {
            "\u{25B6}"
        };
        let hover = if ctx.prompt_expanded {
            "Hide prompt"
        } else {
            "Show prompt"
        };
        ui.horizontal(|ui| {
            if ui.button(icon("\u{2190} Back", fs)).clicked() {
                actions.close = true;
            }
            ui.separator();
            ui.strong("Diff Review");
            ui.separator();
            if let Some(hash) = ctx.commit_hash {
                let short = &hash[..7.min(hash.len())];
                if ui
                    .button(icon(short, fs))
                    .on_hover_text("Copy commit ID")
                    .clicked()
                {
                    ui.ctx().copy_text(hash.to_string());
                }
                ui.separator();
            }
            if ui
                .button(icon(&format!("{} {}", arrow, prefix), fs))
                .on_hover_text(hover)
                .clicked()
            {
                actions.toggle_prompt = true;
            }
            Self::render_collapsed_cue_text(ui, sem, ctx.prompt_expanded, ctx.cue_text);
        });
    }

    fn render_collapsed_cue_text(
        ui: &mut egui::Ui,
        sem: &SemanticColors,
        prompt_expanded: bool,
        cue_text: &str,
    ) {
        if prompt_expanded {
            return;
        }
        let truncated = if cue_text.len() > 80 {
            format!("{}...", crate::app::truncate_str(cue_text, 77))
        } else {
            cue_text.to_string()
        };
        ui.label(egui::RichText::new(truncated).color(sem.secondary_text));
    }

    fn render_diff_prompt_section(
        ui: &mut egui::Ui,
        sem: &SemanticColors,
        prompt_expanded: bool,
        cue_text: &str,
        commit_hash: Option<&str>,
    ) {
        if prompt_expanded {
            ui.group(|ui| {
                if let Some(hash) = commit_hash {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Commit ID:")
                                .color(sem.secondary_text)
                                .strong(),
                        );
                        if ui.button(hash).on_hover_text("Copy commit ID").clicked() {
                            ui.ctx().copy_text(hash.to_string());
                        }
                    });
                    ui.add_space(4.0);
                }
                egui::ScrollArea::vertical()
                    .id_salt("prompt_scroll")
                    .max_height(150.0)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(cue_text).color(sem.secondary_text));
                    });
            });
        }
    }

    fn render_diff_view_mode_toolbar(
        ui: &mut egui::Ui,
        fs: f32,
        sem: &SemanticColors,
        view_mode: DiffViewMode,
        read_only: bool,
        actions: &mut DiffReviewActions,
    ) {
        ui.horizontal(|ui| {
            if ui
                .selectable_label(view_mode == DiffViewMode::Inline, "Inline")
                .clicked()
            {
                actions.toggle_mode = Some(DiffViewMode::Inline);
            }
            if ui
                .selectable_label(view_mode == DiffViewMode::SideBySide, "Side-by-Side")
                .clicked()
            {
                actions.toggle_mode = Some(DiffViewMode::SideBySide);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !read_only {
                    if ui
                        .button(icon("\u{21BA} Revert", fs).color(sem.danger))
                        .on_hover_text("Revert changes back to previous state")
                        .clicked()
                    {
                        actions.reject = true;
                    }
                    if ui
                        .button(icon("\u{2713} Commit", fs).color(sem.success))
                        .on_hover_text("Commit the applied changes")
                        .clicked()
                    {
                        actions.accept = true;
                    }
                }
            });
        });
    }

    fn render_diff_reply_field(
        ui: &mut egui::Ui,
        fs: f32,
        read_only: bool,
        reply_text: &mut String,
        actions: &mut DiffReviewActions,
    ) {
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
                        && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command));
                if send && !reply_text.trim().is_empty() {
                    actions.reply_send = Some(reply_text.clone());
                }
            });
        }
    }

    fn render_diff_search_bar(
        ui: &mut egui::Ui,
        fs: f32,
        sem: &SemanticColors,
        search_query: &mut String,
        search: &SearchState<'_>,
        actions: &mut DiffReviewActions,
    ) {
        if !search.active {
            return;
        }
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
                actions.search_changed = true;
            }

            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if ui.input(|i| i.modifiers.shift) {
                    actions.search_prev = true;
                } else {
                    actions.search_next = true;
                }
                response.request_focus();
            }

            Self::render_search_match_count(ui, sem, search_query, search);
            Self::render_search_nav_buttons(ui, fs, actions);

            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                actions.search_close = true;
            }
        });
    }

    fn render_search_match_count(
        ui: &mut egui::Ui,
        sem: &SemanticColors,
        search_query: &str,
        search: &SearchState<'_>,
    ) {
        let match_count = search.matches.len();
        if !search_query.is_empty() {
            let label = if match_count == 0 {
                "No matches".to_string()
            } else {
                let current = search.current.map(|i| i + 1).unwrap_or(0);
                format!("{}/{}", current, match_count)
            };
            ui.label(
                egui::RichText::new(label)
                    .monospace()
                    .small()
                    .color(if match_count == 0 {
                        sem.danger
                    } else {
                        sem.secondary_text
                    }),
            );
        }
    }

    fn render_search_nav_buttons(ui: &mut egui::Ui, fs: f32, actions: &mut DiffReviewActions) {
        if ui
            .small_button(icon("\u{2191}", fs))
            .on_hover_text("Previous (Shift+Enter)")
            .clicked()
        {
            actions.search_prev = true;
        }
        if ui
            .small_button(icon("\u{2193}", fs))
            .on_hover_text("Next (Enter)")
            .clicked()
        {
            actions.search_next = true;
        }
        if ui
            .small_button(icon("\u{2715}", fs))
            .on_hover_text("Close (Esc)")
            .clicked()
        {
            actions.search_close = true;
        }
    }

    fn render_diff_content(
        ui: &mut egui::Ui,
        sem: &SemanticColors,
        parsed: &crate::diff_view::ParsedDiff,
        view_mode: DiffViewMode,
        search_query: &str,
        search: &SearchState<'_>,
        collapsed_files: &mut std::collections::HashSet<usize>,
    ) {
        let query_lower_owned = search_query.to_lowercase();
        let search_highlight = if search.active && !search_query.is_empty() {
            let current = search
                .current
                .and_then(|idx| search.matches.get(idx).copied());
            Some(DiffSearchHighlight {
                query_lower: &query_lower_owned,
                current,
            })
        } else {
            None
        };

        egui::ScrollArea::both()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if parsed.files.is_empty() {
                    ui.add_space(SPACE_LG);
                    ui.label(
                        egui::RichText::new("No file changes in this commit.")
                            .italics()
                            .color(sem.secondary_text),
                    );
                } else {
                    match view_mode {
                        DiffViewMode::Inline => {
                            diff_view::render_inline_diff(
                                ui,
                                parsed,
                                collapsed_files,
                                search_highlight.as_ref(),
                                sem,
                            );
                        }
                        DiffViewMode::SideBySide => {
                            diff_view::render_side_by_side_diff(
                                ui,
                                parsed,
                                collapsed_files,
                                search_highlight.as_ref(),
                                sem,
                            );
                        }
                    }
                }
            });
    }

    fn apply_diff_review_state_updates(&mut self, actions: &DiffReviewActions) {
        if let Some(mode) = actions.toggle_mode {
            if let Some(ref mut review) = self.diff_review {
                review.view_mode = mode;
            }
        }
        if actions.toggle_prompt {
            if let Some(ref mut review) = self.diff_review {
                review.prompt_expanded = !review.prompt_expanded;
            }
        }
    }

    fn apply_diff_search_actions(&mut self, actions: &DiffReviewActions) {
        if actions.search_changed {
            if let Some(ref mut review) = self.diff_review {
                crate::app::search::update_diff_search_matches(review);
                if let Some(idx) = review.search_current {
                    if let Some(&(file_idx, _, _)) = review.search_matches.get(idx) {
                        review.collapsed_files.remove(&file_idx);
                    }
                }
            }
        }
        if actions.search_next {
            self.diff_search_navigate_next();
        }
        if actions.search_prev {
            self.diff_search_navigate_prev();
        }
        if actions.search_close {
            if let Some(ref mut review) = self.diff_review {
                review.search_active = false;
                review.search_query.clear();
                review.search_matches.clear();
                review.search_current = None;
            }
        }
    }

    fn diff_search_navigate_next(&mut self) {
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

    fn diff_search_navigate_prev(&mut self) {
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

    fn apply_diff_review_actions(
        &mut self,
        actions: DiffReviewActions,
        cue_id: i64,
        diff_text: &str,
        cue_text: &str,
    ) {
        if actions.accept {
            self.handle_diff_accept(cue_id, diff_text, cue_text);
        } else if actions.reject {
            self.handle_diff_reject(cue_id, diff_text);
        } else if let Some(reply) = actions.reply_send {
            self.trigger_claude_reply(cue_id, &reply, &[]);
        } else if actions.close {
            self.diff_review = None;
        }
    }

    fn handle_diff_accept(&mut self, cue_id: i64, diff_text: &str, cue_text: &str) {
        let commit_msg = git::generate_commit_message(cue_text);
        match git::commit_diff(&self.project_root, diff_text, &commit_msg) {
            Ok(hash) => {
                let short = &hash[..7.min(hash.len())];
                self.set_status_message(format!("Committed: {}", short));
                let _ = self.db.update_cue_status(cue_id, CueStatus::Done);
                let cue_prompt = self
                    .cues
                    .iter()
                    .find(|c| c.id == cue_id)
                    .map(|c| c.text.clone())
                    .unwrap_or_default();
                self.trigger_agents_for(&AgentTrigger::AfterCommit, Some(cue_id), &cue_prompt);
                self.reload_cues();
                self.reload_git_info();
                self.reload_commit_history();
                self.diff_review = None;
            }
            Err(e) => {
                self.set_status_message(format!("Commit failed: {}", e));
            }
        }
    }

    fn handle_diff_reject(&mut self, cue_id: i64, diff_text: &str) {
        let file_paths = git::parse_diff_file_paths_for_repo(&self.project_root, diff_text);
        match git::revert_files(&self.project_root, &file_paths) {
            Ok(()) => {
                let _ = self.db.update_cue_status(cue_id, CueStatus::Inbox);
                let result = self.reload_open_tabs_and_notify_lsp();
                let mut problems: Vec<String> = self
                    .viewer
                    .tabs
                    .iter()
                    .filter(|tab| !tab.file_path.is_file())
                    .filter_map(|tab| {
                        tab.file_path
                            .file_name()
                            .map(|n| format!("{} (deleted)", n.to_string_lossy()))
                    })
                    .collect();
                for (path, err) in &result.failed {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    problems.push(format!("{} ({})", name, err));
                }
                if !problems.is_empty() {
                    self.set_status_message(format!(
                        "Reverted, but failed to reload: {}",
                        problems.join(", ")
                    ));
                }
                self.reload_cues();
                self.reload_git_info();
                self.diff_review = None;
            }
            Err(e) => {
                self.set_status_message(format!("Revert failed: {}", e));
            }
        }
    }
}
