mod cue_card;
mod markdown_import;

use std::collections::{BTreeSet, HashSet};
use std::time::Instant;

use eframe::egui;

use std::collections::HashMap;

use super::{icon, CueAction, DirigentApp, PendingPlay, FONT_SCALE_SUBHEADING, SPACE_XS};
use crate::db::{Cue, CueStatus};

/// (text, file_path, line_number, line_number_end, attached_images)
type ReuseCueData = (String, String, usize, Option<usize>, Vec<String>);
use crate::diff_view::{self, DiffViewMode};
use crate::git;
use crate::settings::{self, CliProvider};

use markdown_import::{parse_markdown_sections, pick_markdown_file};

impl DirigentApp {
    pub(super) fn render_cue_pool(&mut self, ctx: &egui::Context) {
        // Clean up expired transition flashes
        self.cue_move_flash
            .retain(|_, when| when.elapsed().as_secs_f32() < 1.0);

        egui::SidePanel::right("cue_pool")
            .default_width(250.0)
            .min_width(200.0)
            .show(ctx, |ui| {
                let (selected_play_prompt, custom_cue_requested, import_requested) =
                    self.render_cue_pool_header(ui);
                self.handle_playbook_selection(selected_play_prompt);
                self.handle_custom_cue_request(custom_cue_requested);
                self.handle_import_request(import_requested);
                self.render_source_filter(ui);
                let reuse_cue = self.render_prompt_history(ui);
                self.handle_reuse_cue(reuse_cue);

                let panel_rect = ui.max_rect();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut actions: Vec<(i64, CueAction)> = Vec::new();
                    let mut load_more_archived = false;

                    let cues_snapshot = self.cues.clone();
                    let source_filter = self.sources.filter.clone();
                    let filtered_archived_count = match &source_filter {
                        Some(label) => self.db.archived_cue_count_by_source(label).unwrap_or(0),
                        None => self.archived_cue_count,
                    };
                    for &status in CueStatus::all() {
                        let section_cues: Vec<&Cue> = filter_cues_by_status_and_source(
                            &cues_snapshot,
                            status,
                            &source_filter,
                        );
                        self.render_cue_section(
                            ui,
                            status,
                            &section_cues,
                            &mut actions,
                            &mut load_more_archived,
                            filtered_archived_count,
                        );
                    }

                    if load_more_archived {
                        self.archived_cue_limit += 50;
                        self.reload_cues();
                    }

                    self.claude.expand_running = false;

                    for (id, action) in actions {
                        self.process_cue_action(id, action);
                    }
                });

                self.render_lava_lamp(ui, panel_rect);
            });
    }

    fn render_cue_pool_header(&mut self, ui: &mut egui::Ui) -> (Option<String>, bool, bool) {
        let mut result = (None, false, false);
        let heading_text = build_heading_text(&self.cues);
        let font_size = self.settings.font_size;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(heading_text)
                    .size(font_size * FONT_SCALE_SUBHEADING)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                result = render_cue_pool_buttons(ui, &self.settings.playbook);
            });
        });
        result
    }

    fn handle_playbook_selection(&mut self, selected_play_prompt: Option<String>) {
        let Some(prompt) = selected_play_prompt else {
            return;
        };
        let vars = settings::parse_play_variables(&prompt);
        if vars.is_empty() {
            let _ = self.db.insert_cue(&prompt, "", 0, None, &[]);
            self.reload_cues();
            return;
        }
        let mut auto_resolved = HashMap::new();
        let mut selected = Vec::new();
        let mut custom_text = Vec::new();
        for (i, var) in vars.iter().enumerate() {
            if var.name.eq_ignore_ascii_case("LICENSE") {
                let has_license = [
                    "LICENSE",
                    "LICENSE.md",
                    "LICENSE.txt",
                    "LICENCE",
                    "LICENCE.md",
                ]
                .iter()
                .any(|f| self.project_root.join(f).exists());
                if has_license {
                    auto_resolved.insert(i, "already present".to_string());
                }
            }
            selected.push(0);
            custom_text.push(String::new());
        }
        if auto_resolved.len() == vars.len() {
            let resolved: Vec<(String, String)> = vars
                .iter()
                .enumerate()
                .map(|(i, v)| (v.token.clone(), auto_resolved[&i].clone()))
                .collect();
            let final_prompt = settings::substitute_play_variables(&prompt, &resolved);
            let _ = self.db.insert_cue(&final_prompt, "", 0, None, &[]);
            self.reload_cues();
        } else {
            self.pending_play = Some(PendingPlay {
                prompt,
                variables: vars,
                selected,
                custom_text,
                auto_resolved,
            });
        }
    }

    fn handle_custom_cue_request(&mut self, requested: bool) {
        if requested {
            self.global_prompt_input.clear();
        }
    }

    fn handle_import_request(&mut self, import_requested: bool) {
        if !import_requested {
            return;
        }
        let Some(path) = pick_markdown_file(&self.project_root) else {
            return;
        };
        let Ok(content) = std::fs::read_to_string(&path) else {
            return;
        };
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("import")
            .to_string();
        let sections = parse_markdown_sections(&content);
        let (new_count, updated_count) = self.import_sections(&sections, &path, &stem);
        self.reload_cues();
        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("document");
        let msg = format_import_message(new_count, updated_count, filename);
        self.set_status_message(msg);
    }

    fn import_sections(
        &self,
        sections: &[markdown_import::ImportedSection],
        path: &std::path::Path,
        stem: &str,
    ) -> (usize, usize) {
        let mut new_count = 0usize;
        let mut updated_count = 0usize;
        for section in sections {
            let source_ref = format!("{}#{}", path.display(), section.number);
            let text = format!("{}\n\n{}", section.title, section.body);
            match self.db.cue_exists_by_source_ref(&source_ref) {
                Ok(true) => {
                    if self
                        .db
                        .update_cue_text_by_source_ref(&source_ref, &text)
                        .is_ok()
                    {
                        updated_count += 1;
                    }
                }
                Ok(false) => {
                    if self
                        .db
                        .insert_cue_from_source(&text, stem, &source_ref)
                        .is_ok()
                    {
                        new_count += 1;
                    }
                }
                Err(e) => {
                    eprintln!("Failed to check source ref {}: {}", source_ref, e);
                }
            }
        }
        (new_count, updated_count)
    }

    fn render_source_filter(&mut self, ui: &mut egui::Ui) {
        let unique_labels = collect_unique_labels(&self.cues, &self.settings.sources);
        if unique_labels.is_empty() {
            return;
        }
        ui.horizontal(|ui| {
            let current = self.sources.filter.as_deref().unwrap_or("All");
            egui::ComboBox::from_id_salt("source_filter")
                .selected_text(current)
                .width(ui.available_width() - 8.0)
                .show_ui(ui, |ui| {
                    let is_all = self.sources.filter.is_none();
                    if ui.selectable_label(is_all, "All").clicked() {
                        self.sources.filter = None;
                    }
                    for label in &unique_labels {
                        let count = self
                            .cues
                            .iter()
                            .filter(|c| c.source_label.as_deref() == Some(label.as_str()))
                            .count();
                        let display = format!("{} ({})", label, count);
                        let selected = self.sources.filter.as_deref() == Some(label.as_str());
                        if ui.selectable_label(selected, &display).clicked() {
                            self.sources.filter = Some(label.clone());
                        }
                    }
                });
        });
    }

    fn render_prompt_history(&mut self, ui: &mut egui::Ui) -> Option<ReuseCueData> {
        self.render_prompt_history_search_bar(ui);
        self.render_prompt_history_results(ui)
    }

    fn render_prompt_history_search_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            self.render_prompt_history_toggle(ui);
            if self.prompt_history_active {
                self.render_prompt_history_input(ui);
            }
        });
    }

    fn render_prompt_history_toggle(&mut self, ui: &mut egui::Ui) {
        let (search_icon, hover) = if self.prompt_history_active {
            ("\u{2715}", "Close search")
        } else {
            ("\u{1F50D}", "Search past prompts")
        };
        if ui.small_button(search_icon).on_hover_text(hover).clicked() {
            self.prompt_history_active = !self.prompt_history_active;
            if !self.prompt_history_active {
                self.prompt_history_query.clear();
                self.prompt_history_results.clear();
            }
        }
    }

    fn render_prompt_history_input(&mut self, ui: &mut egui::Ui) {
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.prompt_history_query)
                .desired_width(ui.available_width())
                .hint_text("Search past cues...")
                .font(egui::TextStyle::Small),
        );
        if response.changed() && self.prompt_history_query.len() >= 2 {
            self.prompt_history_results = self
                .db
                .search_cue_history(&self.prompt_history_query, 10)
                .unwrap_or_default();
        } else if self.prompt_history_query.len() < 2 {
            self.prompt_history_results.clear();
        }
    }

    fn render_prompt_history_results(&mut self, ui: &mut egui::Ui) -> Option<ReuseCueData> {
        if !self.prompt_history_active || self.prompt_history_results.is_empty() {
            return None;
        }
        let mut reuse_cue: Option<ReuseCueData> = None;
        egui::Frame::NONE
            .inner_margin(egui::Margin::same(4))
            .corner_radius(4)
            .fill(self.semantic.selection_bg())
            .show(ui, |ui| {
                for (_id, text, file_path, line_number, line_number_end, images) in
                    &self.prompt_history_results
                {
                    ui.horizontal(|ui| {
                        let preview: String =
                            text.lines().next().unwrap_or("").chars().take(60).collect();
                        let location = if file_path.is_empty() {
                            "Global".to_string()
                        } else {
                            file_path.clone()
                        };
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&preview).small());
                            ui.label(
                                egui::RichText::new(&location)
                                    .small()
                                    .color(self.semantic.muted_text()),
                            );
                        });
                        if ui
                            .small_button(icon("\u{21A9} Reuse", self.settings.font_size))
                            .on_hover_text("Create a new cue with this text")
                            .clicked()
                        {
                            reuse_cue = Some((
                                text.clone(),
                                file_path.clone(),
                                *line_number,
                                *line_number_end,
                                images.clone(),
                            ));
                        }
                    });
                    ui.add_space(2.0);
                }
            });
        reuse_cue
    }

    fn handle_reuse_cue(&mut self, reuse_cue: Option<ReuseCueData>) {
        let Some((text, file_path, line_number, line_number_end, images)) = reuse_cue else {
            return;
        };
        match self
            .db
            .insert_cue(&text, &file_path, line_number, line_number_end, &images)
        {
            Ok(_) => {
                self.reload_cues();
                self.prompt_history_active = false;
                self.prompt_history_query.clear();
                self.prompt_history_results.clear();
            }
            Err(e) => {
                eprintln!("Failed to insert cue: {e}");
            }
        }
    }

    fn render_cue_section(
        &mut self,
        ui: &mut egui::Ui,
        status: CueStatus,
        section_cues: &[&Cue],
        actions: &mut Vec<(i64, CueAction)>,
        load_more_archived: &mut bool,
        filtered_archived_count: usize,
    ) {
        let header = build_section_header(status, section_cues.len(), filtered_archived_count);
        let header_rt = egui::RichText::new(header)
            .size(self.settings.font_size * FONT_SCALE_SUBHEADING)
            .strong();
        let mut collapsing = egui::CollapsingHeader::new(header_rt)
            .id_salt(status.label())
            .default_open(status == CueStatus::Inbox || status == CueStatus::Review);
        if status == CueStatus::Ready && self.claude.expand_running {
            collapsing = collapsing.open(Some(true));
        }
        collapsing.show(ui, |ui| {
            if section_cues.is_empty() {
                ui.label(
                    egui::RichText::new("(empty)")
                        .italics()
                        .color(self.semantic.tertiary_text),
                );
            }
            self.render_section_bulk_actions(ui, status, section_cues, actions);
            for cue in section_cues {
                self.render_cue_card(ui, cue, actions, status);
            }
            if status == CueStatus::Archived && filtered_archived_count > section_cues.len() {
                let remaining = filtered_archived_count - section_cues.len();
                let label = format!("Load more ({} remaining)", remaining);
                if ui.small_button(label).clicked() {
                    *load_more_archived = true;
                }
            }
        });
    }

    fn render_section_bulk_actions(
        &mut self,
        ui: &mut egui::Ui,
        status: CueStatus,
        section_cues: &[&Cue],
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        if status == CueStatus::Review && section_cues.len() > 1 {
            self.render_review_bulk_actions(ui, actions);
        }
        if status == CueStatus::Done {
            self.render_done_bulk_actions(ui, section_cues, actions);
        }
    }

    fn render_review_bulk_actions(
        &mut self,
        ui: &mut egui::Ui,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        ui.horizontal(|ui| {
            if ui
                .small_button(icon("\u{2713} Commit All", self.settings.font_size))
                .on_hover_text("Commit all uncommitted changes and move all Review cues to Done")
                .clicked()
            {
                actions.push((0, CueAction::CommitAll));
            }
            if ui
                .small_button(icon("\u{1F3F7} Tag All", self.settings.font_size))
                .on_hover_text("Add a tag to all Review cues")
                .clicked()
            {
                self.tag_all_review_input = Some(String::new());
            }
        });
        self.render_tag_all_review_input(ui, actions);
        ui.add_space(SPACE_XS);
    }

    fn render_tag_all_review_input(
        &mut self,
        ui: &mut egui::Ui,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let mut cancel_tag_all = false;
        if let Some(ref mut tag_text) = self.tag_all_review_input {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("\u{1F3F7}").color(self.semantic.accent));
                let response = ui.add(
                    egui::TextEdit::singleline(tag_text)
                        .desired_width(100.0)
                        .hint_text("Tag for all"),
                );
                let submit = ui
                    .small_button(icon("\u{2713} Set", self.settings.font_size))
                    .on_hover_text("Apply tag to all Review cues")
                    .clicked()
                    || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if submit && !tag_text.trim().is_empty() {
                    actions.push((0, CueAction::TagAllReview(tag_text.trim().to_string())));
                }
                if ui
                    .small_button("\u{2715}")
                    .on_hover_text("Cancel")
                    .clicked()
                {
                    cancel_tag_all = true;
                }
            });
        }
        if cancel_tag_all {
            self.tag_all_review_input = None;
        }
    }

    fn render_done_bulk_actions(
        &mut self,
        ui: &mut egui::Ui,
        section_cues: &[&Cue],
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let has_pr_cues = section_cues.iter().any(|c| {
            c.source_ref
                .as_ref()
                .map(|s| s.starts_with("pr"))
                .unwrap_or(false)
        });
        if !has_pr_cues {
            return;
        }
        ui.horizontal(|ui| {
            self.render_push_and_notify_button(ui, actions);
            self.render_refresh_pr_button(ui, actions);
        });
        ui.add_space(SPACE_XS);
    }

    fn render_push_and_notify_button(
        &self,
        ui: &mut egui::Ui,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        if !self.git.notifying_pr && !self.git.pushing {
            let btn = egui::Button::new(
                icon("\u{2191} Push & Notify PR", self.settings.font_size)
                    .color(self.semantic.badge_text),
            )
            .fill(self.semantic.accent);
            if ui
                .add(btn)
                .on_hover_text("Push commits and reply to all PR comments that findings were fixed")
                .clicked()
            {
                actions.push((0, CueAction::PushAndNotifyPR));
            }
        } else if self.git.notifying_pr {
            ui.label(
                egui::RichText::new("Notifying PR...")
                    .small()
                    .color(self.semantic.accent),
            );
        }
    }

    fn render_refresh_pr_button(&self, ui: &mut egui::Ui, actions: &mut Vec<(i64, CueAction)>) {
        if self.git.importing_pr {
            ui.label(
                egui::RichText::new("Refreshing PR\u{2026}")
                    .small()
                    .color(self.semantic.accent),
            );
        } else if ui
            .small_button(icon("\u{21BB} Refresh PR", self.settings.font_size))
            .on_hover_text("Re-import findings from the PR (check for new review comments)")
            .clicked()
        {
            actions.push((0, CueAction::RefreshPR));
        }
    }

    fn process_cue_action(&mut self, id: i64, action: CueAction) {
        match action {
            CueAction::StartEdit(text) => {
                self.editing_cue = Some(super::EditingCue {
                    id,
                    text,
                    focus_requested: false,
                });
            }
            CueAction::CancelEdit => {
                self.editing_cue = None;
            }
            CueAction::SaveEdit(new_text) => {
                let _ = self.db.update_cue_text(id, &new_text);
                let _ = self.db.log_activity(id, "Edited");
                self.editing_cue = None;
            }
            CueAction::MoveTo(new_status) => {
                self.process_move_to(id, new_status);
            }
            CueAction::Delete => {
                self.process_delete(id);
            }
            CueAction::Navigate(file_path, line, line_end) => {
                self.process_navigate(&file_path, line, line_end);
            }
            CueAction::ShowDiff(cue_id) => {
                self.process_show_diff(cue_id);
            }
            CueAction::CommitReview(cue_id) => {
                self.process_commit_review(cue_id);
            }
            CueAction::RevertReview(cue_id) => {
                self.process_revert_review(cue_id);
            }
            CueAction::ReplyReview(cue_id, reply_text) => {
                self.reply_inputs.remove(&cue_id);
                let _ = self.db.log_activity(cue_id, "Reply sent");
                self.trigger_claude_reply(cue_id, &reply_text, &[]);
            }
            CueAction::ShowRunningLog(cue_id) => {
                self.process_show_running_log(cue_id);
            }
            CueAction::ShowAgentRuns(cue_id) => {
                self.dismiss_central_overlays();
                self.show_agent_runs_for_cue = Some(cue_id);
            }
            CueAction::CommitAll => {
                self.process_commit_all();
            }
            CueAction::QueueNext => {
                self.process_queue_next(id);
            }
            CueAction::ScheduleRun(input) => {
                self.process_schedule_run(id, &input);
            }
            CueAction::CancelQueue => {
                self.run_queue.retain(|&cid| cid != id);
                self.scheduled_runs.remove(&id);
                self.schedule_inputs.remove(&id);
                let _ = self.db.log_activity(id, "Queue/schedule cancelled");
            }
            CueAction::SetTag(tag) => {
                let _ = self.db.update_cue_tag(id, tag.as_deref());
                self.tag_inputs.remove(&id);
                if let Some(ref t) = tag {
                    let _ = self.db.log_activity(id, &format!("Tagged: {}", t));
                } else {
                    let _ = self.db.log_activity(id, "Tag removed");
                }
            }
            CueAction::Push => {
                self.start_git_push();
            }
            CueAction::CreatePR => {
                self.open_create_pr_dialog();
            }
            CueAction::NotifyPR(cue_id) => {
                self.start_notify_pr_single(cue_id);
            }
            CueAction::PushAndNotifyPR => {
                self.start_push_and_notify_pr();
            }
            CueAction::RefreshPR => {
                self.process_refresh_pr();
            }
            CueAction::TagAllReview(tag) => {
                self.process_tag_all_review(&tag);
            }
        }
        self.reload_cues();
    }

    fn process_move_to(&mut self, id: i64, new_status: CueStatus) {
        if new_status != CueStatus::Ready {
            self.cancel_cue_task(id);
        }
        self.run_queue.retain(|&cid| cid != id);
        self.scheduled_runs.remove(&id);
        self.schedule_inputs.remove(&id);
        if let Err(e) = self.db.update_cue_status(id, new_status) {
            let _ = self.db.log_activity(
                id,
                &format!("Failed to move to {}: {}", new_status.label(), e),
            );
            return;
        }
        let _ = self
            .db
            .log_activity(id, &format!("Moved to {}", new_status.label()));
        self.cue_move_flash.insert(id, Instant::now());
        if new_status == CueStatus::Ready {
            self.claude.expand_running = true;
            self.reload_cues();
            self.trigger_claude(id);
        }
    }

    fn process_delete(&mut self, id: i64) {
        self.cancel_cue_task(id);
        self.run_queue.retain(|&cid| cid != id);
        self.scheduled_runs.remove(&id);
        self.schedule_inputs.remove(&id);
        let _ = self.db.delete_cue(id);
    }

    fn process_navigate(&mut self, file_path: &str, line: usize, line_end: Option<usize>) {
        self.push_nav_history();
        let full_path = self.project_root.join(file_path);
        if self.viewer.current_file() != Some(&full_path) {
            self.load_file(full_path);
        } else {
            self.dismiss_central_overlays();
        }
        if let Some(tab) = self.viewer.active_mut() {
            tab.selection_start = Some(line);
            tab.selection_end = Some(line_end.unwrap_or(line));
        }
        self.viewer.scroll_to_line = Some(line);
    }

    fn process_show_diff(&mut self, cue_id: i64) {
        let Ok(Some(exec)) = self.db.get_latest_execution(cue_id) else {
            return;
        };
        let Some(diff) = exec.diff else { return };
        let cue = self.cues.iter().find(|c| c.id == cue_id);
        let text = cue.map(|c| c.text.clone()).unwrap_or_default();
        let read_only = cue.map(|c| c.status != CueStatus::Review).unwrap_or(true);
        let parsed = diff_view::parse_unified_diff(&diff);
        self.dismiss_central_overlays();
        self.diff_review = Some(super::DiffReview {
            cue_id,
            diff,
            cue_text: text,
            parsed,
            view_mode: DiffViewMode::Inline,
            read_only,
            collapsed_files: HashSet::new(),
            prompt_expanded: false,
            reply_text: String::new(),
            search_active: false,
            search_query: String::new(),
            search_matches: Vec::new(),
            search_current: None,
        });
    }

    fn process_commit_review(&mut self, cue_id: i64) {
        if let Ok(Some(exec)) = self.db.get_latest_execution(cue_id) {
            if let Some(ref diff) = exec.diff {
                let cue_text = self
                    .cues
                    .iter()
                    .find(|c| c.id == cue_id)
                    .map(|c| c.text.clone())
                    .unwrap_or_default();
                let commit_msg = git::generate_commit_message(&cue_text);
                self.apply_commit_result(
                    cue_id,
                    git::commit_diff(&self.project_root, diff, &commit_msg),
                );
            }
        }
        self.reload_git_info();
        self.reload_commit_history();
    }

    fn apply_commit_result(&mut self, cue_id: i64, result: crate::error::Result<String>) {
        match result {
            Ok(hash) => {
                let short = &hash[..7.min(hash.len())];
                self.set_status_message(format!("Committed: {}", short));
                let _ = self.db.update_cue_status(cue_id, CueStatus::Done);
                let _ = self
                    .db
                    .log_activity(cue_id, &format!("Committed ({})", short));
            }
            Err(e) => {
                let msg = format!("{}", e);
                if msg.contains("nothing to commit") {
                    self.set_status_message("Nothing to commit \u{2014} moved to Done".into());
                    let _ = self.db.update_cue_status(cue_id, CueStatus::Done);
                    let _ = self
                        .db
                        .log_activity(cue_id, "Moved to Done (already committed)");
                } else {
                    self.set_status_message(format!("Commit failed: {}", e));
                }
            }
        }
    }

    fn process_revert_review(&mut self, cue_id: i64) {
        let reverted = match self.db.get_latest_execution(cue_id) {
            Ok(Some(exec)) => {
                if let Some(ref diff) = exec.diff {
                    let file_paths = git::parse_diff_file_paths_for_repo(&self.project_root, diff);
                    match git::revert_files(&self.project_root, &file_paths) {
                        Ok(()) => true,
                        Err(e) => {
                            self.set_status_message(format!("Revert failed: {}", e));
                            false
                        }
                    }
                } else {
                    self.set_status_message("Nothing to revert — no diff in execution".into());
                    false
                }
            }
            Ok(None) => {
                self.set_status_message("Nothing to revert — no execution found".into());
                false
            }
            Err(e) => {
                self.set_status_message(format!("Revert failed: {}", e));
                false
            }
        };
        if reverted {
            let _ = self.db.update_cue_status(cue_id, CueStatus::Inbox);
            let _ = self.db.log_activity(cue_id, "Reverted");
            self.reload_open_tabs();
            self.reload_git_info();
        }
    }

    fn process_show_running_log(&mut self, cue_id: i64) {
        if let Ok(execs) = self.db.get_all_executions(cue_id) {
            if let std::collections::hash_map::Entry::Vacant(e) =
                self.claude.running_logs.entry(cue_id)
            {
                if let Some(last) = execs.last() {
                    if let Some(ref log_text) = last.log {
                        e.insert((log_text.clone(), CliProvider::Claude));
                    }
                }
            }
            self.claude.conversation_history = execs;
        }
        self.dismiss_central_overlays();
        self.claude.show_log = Some(cue_id);
    }

    fn process_commit_all(&mut self) {
        let review_cues: Vec<&Cue> = self
            .cues
            .iter()
            .filter(|c| c.status == CueStatus::Review)
            .collect();
        if review_cues.is_empty() {
            self.set_status_message("No cues in Review".into());
            return;
        }
        let subject = build_commit_all_subject(&review_cues);
        let cue_details: Vec<String> = review_cues
            .iter()
            .map(|c| format!("- {}", c.text.trim()))
            .collect();
        let commit_msg = format!("{}\n\n{}", subject, cue_details.join("\n\n"),);
        let review_ids: Vec<i64> = review_cues.iter().map(|c| c.id).collect();
        match git::commit_all(&self.project_root, &commit_msg) {
            Ok(hash) => {
                let short = &hash[..7.min(hash.len())];
                let plural = if review_ids.len() == 1 { "" } else { "s" };
                self.set_status_message(format!(
                    "Committed all: {} ({} cue{})",
                    short,
                    review_ids.len(),
                    plural,
                ));
                for cue_id in &review_ids {
                    let _ = self.db.update_cue_status(*cue_id, CueStatus::Done);
                    let _ = self
                        .db
                        .log_activity(*cue_id, &format!("Committed ({})", short));
                }
            }
            Err(e) => {
                self.set_status_message(format!("Commit all failed: {}", e));
            }
        }
        self.reload_git_info();
        self.reload_commit_history();
    }

    fn process_queue_next(&mut self, id: i64) {
        if self.run_queue.contains(&id) {
            return;
        }
        self.run_queue.push(id);
        let _ = self.db.log_activity(id, "Queued (run next)");
        let preview = self.cue_preview(id);
        self.set_status_message(format!(
            "\"{}\" queued \u{2014} will run after current runs finish",
            preview
        ));
    }

    fn process_schedule_run(&mut self, id: i64, input: &str) {
        if let Some(duration) = parse_schedule_duration(input) {
            let when = Instant::now() + duration;
            self.scheduled_runs.insert(id, when);
            self.schedule_inputs.remove(&id);
            let _ = self.db.log_activity(id, &format!("Scheduled ({})", input));
            let preview = self.cue_preview(id);
            self.set_status_message(format!("\"{}\" scheduled to run in {}", preview, input));
        } else {
            self.set_status_message(format!(
                "Invalid schedule format: \"{}\" \u{2014} use e.g. 5m, 2h, 30s",
                input
            ));
        }
    }

    fn process_refresh_pr(&mut self) {
        let pr_num = self.cues.iter().find_map(|c| {
            c.source_ref
                .as_ref()
                .and_then(|s| s.strip_prefix("pr"))
                .and_then(|s| s.split(':').next())
                .and_then(|n| n.parse::<u32>().ok())
        });
        if let Some(n) = pr_num {
            self.git.import_pr_number = n.to_string();
            self.start_import_pr_findings();
        } else {
            self.open_import_pr_dialog();
        }
    }

    fn process_tag_all_review(&mut self, tag: &str) {
        let review_ids: Vec<i64> = self
            .cues
            .iter()
            .filter(|c| c.status == CueStatus::Review)
            .map(|c| c.id)
            .collect();
        for cue_id in &review_ids {
            let _ = self.db.update_cue_tag(*cue_id, Some(tag));
            let _ = self.db.log_activity(*cue_id, &format!("Tagged: {}", tag));
        }
        self.tag_all_review_input = None;
        let plural = if review_ids.len() == 1 { "" } else { "s" };
        self.set_status_message(format!(
            "Tagged {} Review cue{} with \"{}\"",
            review_ids.len(),
            plural,
            tag
        ));
    }

    fn render_lava_lamp(&mut self, ui: &mut egui::Ui, panel_rect: egui::Rect) {
        if !self.settings.lava_lamp_enabled {
            return;
        }
        if !self.cues.iter().any(|c| c.status == CueStatus::Ready) {
            return;
        }
        let margin = 8.0;
        let scale = if self.lava_lamp_big { 3.0 } else { 1.0 };
        let (lamp_w, lamp_h) = super::lava_lamp::size(scale);
        let origin = egui::pos2(
            panel_rect.right() - lamp_w - margin,
            panel_rect.bottom() - lamp_h - margin,
        );
        let lamp_rect = egui::Rect::from_min_size(origin, egui::vec2(lamp_w, lamp_h));
        let resp = ui.allocate_rect(lamp_rect, egui::Sense::click());
        if resp.clicked() {
            self.lava_lamp_big = !self.lava_lamp_big;
        }
        super::lava_lamp::paint_at(
            ui.painter(),
            ui.ctx(),
            origin,
            self.semantic.accent,
            self.settings.theme.is_dark(),
            scale,
        );
    }
}

/// Build the heading text showing cue counts.
fn build_heading_text(cues: &[Cue]) -> String {
    let inbox = cues.iter().filter(|c| c.status == CueStatus::Inbox).count();
    let review = cues
        .iter()
        .filter(|c| c.status == CueStatus::Review)
        .count();
    let counts: Vec<String> = [
        if inbox > 0 {
            Some(format!("{} inbox", inbox))
        } else {
            None
        },
        if review > 0 {
            Some(format!("{} review", review))
        } else {
            None
        },
    ]
    .into_iter()
    .flatten()
    .collect();
    if counts.is_empty() {
        "Cues".to_string()
    } else {
        format!("Cues ({})", counts.join(", "))
    }
}

fn render_cue_pool_buttons(
    ui: &mut egui::Ui,
    playbook: &[settings::Play],
) -> (Option<String>, bool, bool) {
    let mut selected_play_prompt = None;
    let mut custom_cue_requested = false;
    let mut import_requested = false;

    let plus_btn = ui.button("+").on_hover_text("Playbook");
    if ui
        .button("\u{2193}")
        .on_hover_text("Import from document")
        .clicked()
    {
        import_requested = true;
    }
    egui::Popup::menu(&plus_btn)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
        .show(|ui| {
            ui.set_min_width(200.0);
            ui.label(egui::RichText::new("Playbook").strong());
            ui.separator();
            for play in playbook {
                if ui.selectable_label(false, &play.name).clicked() {
                    selected_play_prompt = Some(play.prompt.clone());
                }
            }
            if !playbook.is_empty() {
                ui.separator();
            }
            if ui.selectable_label(false, "+ Custom cue...").clicked() {
                custom_cue_requested = true;
            }
        });

    (selected_play_prompt, custom_cue_requested, import_requested)
}

/// Collect unique source labels from cues and settings.
fn collect_unique_labels(cues: &[Cue], sources: &[crate::settings::SourceConfig]) -> Vec<String> {
    let mut labels = BTreeSet::new();
    for c in cues {
        if let Some(ref label) = c.source_label {
            labels.insert(label.clone());
        }
    }
    for s in sources {
        if s.enabled {
            labels.insert(s.label.clone());
        }
    }
    labels.into_iter().collect()
}

/// Filter cues by status and optional source label.
fn filter_cues_by_status_and_source<'a>(
    cues: &'a [Cue],
    status: CueStatus,
    source_filter: &Option<String>,
) -> Vec<&'a Cue> {
    cues.iter()
        .rev()
        .filter(|c| c.status == status)
        .filter(|c| {
            if let Some(ref filter) = source_filter {
                c.source_label.as_deref() == Some(filter.as_str())
            } else {
                true
            }
        })
        .collect()
}

/// Build section header text for a cue status column.
fn build_section_header(status: CueStatus, count: usize, archived_total: usize) -> String {
    if status == CueStatus::Archived && archived_total > count {
        format!("{} ({}/{})", status.label(), count, archived_total)
    } else {
        format!("{} ({})", status.label(), count)
    }
}

/// Format the import message.
fn format_import_message(new_count: usize, updated_count: usize, filename: &str) -> String {
    match (new_count, updated_count) {
        (0, 0) => format!("No changes from \"{}\"", filename),
        (n, 0) => format!("Imported {} new cue(s) from \"{}\"", n, filename),
        (0, u) => {
            format!("Updated {} cue(s) from \"{}\"", u, filename)
        }
        (n, u) => format!(
            "Imported {} new, updated {} cue(s) from \"{}\"",
            n, u, filename
        ),
    }
}

/// Build the commit subject line for a "Commit All" action.
fn build_commit_all_subject(review_cues: &[&Cue]) -> String {
    if review_cues.len() == 1 {
        build_single_cue_subject(review_cues[0])
    } else {
        build_multi_cue_subject(review_cues)
    }
}

fn build_single_cue_subject(cue: &Cue) -> String {
    let first_line = cue
        .text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_else(|| cue.text.lines().next().unwrap_or(&cue.text));
    let trimmed = first_line.trim();
    let trimmed = if trimmed.is_empty() {
        let fallback = cue.text.trim();
        if fallback.is_empty() {
            ""
        } else {
            fallback
        }
    } else {
        trimmed
    };
    if trimmed.is_empty() {
        return "Dirigent".to_string();
    }
    let prefix = "Dirigent: ";
    let allowed = 72 - prefix.len();
    if trimmed.len() > allowed {
        format!(
            "{}{}...",
            prefix,
            crate::app::truncate_str(trimmed, allowed - 3)
        )
    } else {
        format!("{}{}", prefix, trimmed)
    }
}

fn build_multi_cue_subject(review_cues: &[&Cue]) -> String {
    let short_names: Vec<&str> = review_cues
        .iter()
        .filter_map(|c| {
            let first = c
                .text
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or_else(|| c.text.lines().next().unwrap_or(&c.text));
            let trimmed = first.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .collect();
    if short_names.is_empty() {
        return format!("Dirigent: {} cues", review_cues.len());
    }
    let combined = short_names.join(", ");
    if combined.len() <= 62 {
        return format!("Dirigent: {}", combined);
    }
    let truncated = crate::app::truncate_str(&combined, 59);
    if truncated.is_empty() {
        format!("Dirigent: {} cues", review_cues.len())
    } else {
        format!("Dirigent: {}...", truncated)
    }
}

/// Parse a schedule duration string like "5m", "2h", "30s" into a `Duration`.
fn parse_schedule_duration(input: &str) -> Option<std::time::Duration> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    let (num_str, suffix) = if let Some(s) = input.strip_suffix('m') {
        (s, 'm')
    } else if let Some(s) = input.strip_suffix('h') {
        (s, 'h')
    } else if let Some(s) = input.strip_suffix('s') {
        (s, 's')
    } else {
        // Default to minutes if no suffix
        (input, 'm')
    };
    let num: u64 = num_str.trim().parse().ok()?;
    if num == 0 {
        return None;
    }
    match suffix {
        's' => Some(std::time::Duration::from_secs(num)),
        'm' => Some(std::time::Duration::from_secs(num * 60)),
        'h' => Some(std::time::Duration::from_secs(num * 3600)),
        _ => None,
    }
}
