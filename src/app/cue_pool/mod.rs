mod cue_card;
mod markdown_import;

use std::collections::{BTreeSet, HashSet};
use std::time::Instant;

use eframe::egui;

use std::collections::HashMap;

use super::{icon, CueAction, DirigentApp, PendingPlay, FONT_SCALE_SUBHEADING, SPACE_XS};
use crate::db::{Cue, CueStatus};
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
                // Header: "Cues" heading + "+" playbook button
                let mut selected_play_prompt: Option<String> = None;
                let mut custom_cue_requested = false;
                let mut import_requested = false;
                ui.horizontal(|ui| {
                    let inbox = self
                        .cues
                        .iter()
                        .filter(|c| c.status == CueStatus::Inbox)
                        .count();
                    let review = self
                        .cues
                        .iter()
                        .filter(|c| c.status == CueStatus::Review)
                        .count();
                    let counts: Vec<String> = [
                        if inbox > 0 { Some(format!("{} inbox", inbox)) } else { None },
                        if review > 0 { Some(format!("{} review", review)) } else { None },
                    ]
                    .into_iter()
                    .flatten()
                    .collect();
                    let heading_text = if counts.is_empty() {
                        "Cues".to_string()
                    } else {
                        format!("Cues ({})", counts.join(", "))
                    };
                    ui.label(egui::RichText::new(heading_text).size(self.settings.font_size * FONT_SCALE_SUBHEADING).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let plus_btn = ui.button("+").on_hover_text("Playbook");
                        if ui.button("\u{2193}").on_hover_text("Import from document").clicked() {
                            import_requested = true;
                        }
                        egui::Popup::menu(&plus_btn)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
                            .show(|ui| {
                                ui.set_min_width(200.0);
                                ui.label(egui::RichText::new("Playbook").strong());
                                ui.separator();
                                for play in &self.settings.playbook {
                                    if ui.selectable_label(false, &play.name).clicked() {
                                        selected_play_prompt = Some(play.prompt.clone());
                                    }
                                }
                                if !self.settings.playbook.is_empty() {
                                    ui.separator();
                                }
                                if ui.selectable_label(false, "+ Custom cue...").clicked() {
                                    custom_cue_requested = true;
                                }
                            });
                    });
                });

                // Handle playbook selection
                if let Some(prompt) = selected_play_prompt {
                    let vars = settings::parse_play_variables(&prompt);
                    if vars.is_empty() {
                        // No template variables — create cue directly.
                        let _ = self.db.insert_cue(&prompt, "", 0, None, &[]);
                        self.reload_cues();
                    } else {
                        // Check which variables can be auto-resolved.
                        let mut auto_resolved = HashMap::new();
                        let mut selected = Vec::new();
                        let mut custom_text = Vec::new();
                        for (i, var) in vars.iter().enumerate() {
                            if var.name.eq_ignore_ascii_case("LICENSE") {
                                // Auto-resolve if a LICENSE file already exists.
                                let has_license = ["LICENSE", "LICENSE.md", "LICENSE.txt", "LICENCE", "LICENCE.md"]
                                    .iter()
                                    .any(|f| self.project_root.join(f).exists());
                                if has_license {
                                    auto_resolved.insert(i, "already present".to_string());
                                }
                            }
                            selected.push(0);
                            custom_text.push(String::new());
                        }
                        // If all variables are auto-resolved, create cue directly.
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
                }
                if custom_cue_requested {
                    // Focus the global prompt field by clearing and letting egui pick it up
                    self.global_prompt_input.clear();
                }
                if import_requested {
                    if let Some(path) = pick_markdown_file(&self.project_root) {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let stem = path.file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("import")
                                .to_string();
                            let sections = parse_markdown_sections(&content);
                            let mut new_count = 0usize;
                            let mut updated_count = 0usize;
                            for section in &sections {
                                let source_ref = format!("{}#{}", path.display(), section.number);
                                let text = format!("{}\n\n{}", section.title, section.body);
                                if self.db.cue_exists_by_source_ref(&source_ref).unwrap_or(false) {
                                    if self.db.update_cue_text_by_source_ref(&source_ref, &text).is_ok() {
                                        updated_count += 1;
                                    }
                                } else {
                                    let _ = self.db.insert_cue_from_source(&text, &stem, &source_ref);
                                    new_count += 1;
                                }
                            }
                            self.reload_cues();
                            let filename = path.file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or("document");
                            let msg = match (new_count, updated_count) {
                                (0, 0) => format!("No changes from \"{}\"", filename),
                                (n, 0) => format!("Imported {} new cue(s) from \"{}\"", n, filename),
                                (0, u) => format!("Updated {} cue(s) from \"{}\"", u, filename),
                                (n, u) => format!("Imported {} new, updated {} cue(s) from \"{}\"", n, u, filename),
                            };
                            self.set_status_message(msg);
                        }
                    }
                }

                // Source filter dropdown
                let unique_labels: Vec<String> = {
                    let mut labels = BTreeSet::new();
                    for c in &self.cues {
                        if let Some(ref label) = c.source_label {
                            labels.insert(label.clone());
                        }
                    }
                    for s in &self.settings.sources {
                        if s.enabled {
                            labels.insert(s.label.clone());
                        }
                    }
                    labels.into_iter().collect()
                };

                if !unique_labels.is_empty() {
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
                                        .filter(|c| {
                                            c.source_label.as_deref() == Some(label.as_str())
                                        })
                                        .count();
                                    let display = format!("{} ({})", label, count);
                                    let selected = self.sources.filter.as_deref()
                                        == Some(label.as_str());
                                    if ui.selectable_label(selected, &display).clicked() {
                                        self.sources.filter = Some(label.clone());
                                    }
                                }
                            });
                    });
                }

                ui.separator();

                let panel_rect = ui.max_rect();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut actions: Vec<(i64, CueAction)> = Vec::new();
                    let mut load_more_archived = false;

                    let cues_snapshot = self.cues.clone();
                    let source_filter = self.sources.filter.clone();
                    for &status in CueStatus::all() {
                        let section_cues: Vec<&Cue> = cues_snapshot
                            .iter()
                            .rev()
                            .filter(|c| c.status == status)
                            .filter(|c| {
                                if let Some(ref filter) = source_filter {
                                    c.source_label.as_deref() == Some(filter.as_str())
                                } else {
                                    true
                                }
                            })
                            .collect();

                        let header = if status == CueStatus::Archived && self.archived_cue_count > section_cues.len() {
                            format!("{} ({}/{})", status.label(), section_cues.len(), self.archived_cue_count)
                        } else {
                            format!("{} ({})", status.label(), section_cues.len())
                        };
                        let header_rt = egui::RichText::new(header)
                            .size(self.settings.font_size * FONT_SCALE_SUBHEADING)
                            .strong();
                        let mut collapsing = egui::CollapsingHeader::new(header_rt)
                            .id_salt(status.label())
                            .default_open(
                                status == CueStatus::Inbox || status == CueStatus::Review,
                            );
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
                                // "Commit All" and "Tag All" buttons for the Review column
                                if status == CueStatus::Review && section_cues.len() > 1 {
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
                                    // Tag All input field
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
                                    ui.add_space(SPACE_XS);
                                }
                                // "Push & Notify PR" and "Refresh PR" buttons for the Done column
                                if status == CueStatus::Done {
                                    let has_pr_cues = section_cues.iter().any(|c| {
                                        c.source_ref
                                            .as_ref()
                                            .map(|s| s.starts_with("pr"))
                                            .unwrap_or(false)
                                    });
                                    if has_pr_cues {
                                        ui.horizontal(|ui| {
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
                                            if self.git.importing_pr {
                                                ui.label(
                                                    egui::RichText::new("Refreshing PR…")
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
                                        });
                                        ui.add_space(SPACE_XS);
                                    }
                                }
                                for cue in &section_cues {
                                    self.render_cue_card(ui, cue, &mut actions, status);
                                }
                                // "Load more" button for Archived when there are more cues
                                if status == CueStatus::Archived && self.archived_cue_count > section_cues.len() {
                                    let remaining = self.archived_cue_count - section_cues.len();
                                    let label = format!("Load more ({} remaining)", remaining);
                                    if ui.small_button(label).clicked() {
                                        load_more_archived = true;
                                    }
                                }
                            });
                    }

                    if load_more_archived {
                        self.archived_cue_limit += 50;
                        self.reload_cues();
                    }

                    // Reset the expand flag after rendering
                    self.claude.expand_running = false;

                    // Process actions after iteration
                    for (id, action) in actions {
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
                                // Cancel the running task if moving away from Ready
                                if new_status != CueStatus::Ready {
                                    self.cancel_cue_task(id);
                                }
                                // Remove from queue/schedule if being moved manually
                                self.run_queue.retain(|&cid| cid != id);
                                self.scheduled_runs.remove(&id);
                                self.schedule_inputs.remove(&id);
                                let _ = self.db.update_cue_status(id, new_status);
                                let _ = self.db.log_activity(id, &format!("Moved to {}", new_status.label()));
                                self.cue_move_flash.insert(id, Instant::now());
                                if new_status == CueStatus::Ready {
                                    self.claude.expand_running = true;
                                    self.reload_cues();
                                    self.trigger_claude(id);
                                }
                            }
                            CueAction::Delete => {
                                self.cancel_cue_task(id);
                                self.run_queue.retain(|&cid| cid != id);
                                self.scheduled_runs.remove(&id);
                                self.schedule_inputs.remove(&id);
                                let _ = self.db.delete_cue(id);
                            }
                            CueAction::Navigate(file_path, line, line_end) => {
                                let full_path = self.project_root.join(&file_path);
                                if self.viewer.current_file.as_ref() != Some(&full_path) {
                                    self.load_file(full_path);
                                } else {
                                    self.dismiss_central_overlays();
                                }
                                self.viewer.selection_start = Some(line);
                                self.viewer.selection_end = Some(line_end.unwrap_or(line));
                                self.viewer.scroll_to_line = Some(line);
                            }
                            CueAction::ShowDiff(cue_id) => {
                                if let Ok(Some(exec)) = self.db.get_latest_execution(cue_id) {
                                    if let Some(diff) = exec.diff {
                                        let cue = self
                                            .cues
                                            .iter()
                                            .find(|c| c.id == cue_id);
                                        let text = cue
                                            .map(|c| c.text.clone())
                                            .unwrap_or_default();
                                        let read_only = cue
                                            .map(|c| c.status != CueStatus::Review)
                                            .unwrap_or(true);
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
                                }
                            }
                            CueAction::CommitReview(cue_id) => {
                                if let Ok(Some(exec)) =
                                    self.db.get_latest_execution(cue_id)
                                {
                                    if let Some(ref diff) = exec.diff {
                                        let cue_text = self
                                            .cues
                                            .iter()
                                            .find(|c| c.id == cue_id)
                                            .map(|c| c.text.clone())
                                            .unwrap_or_default();
                                        let commit_msg =
                                            git::generate_commit_message(&cue_text);
                                        match git::commit_diff(
                                            &self.project_root,
                                            diff,
                                            &commit_msg,
                                        ) {
                                            Ok(hash) => {
                                                let short = &hash[..7.min(hash.len())];
                                                self.set_status_message(format!("Committed: {}", short));
                                                let _ = self.db.update_cue_status(
                                                    cue_id,
                                                    CueStatus::Done,
                                                );
                                                let _ = self.db.log_activity(cue_id, &format!("Committed ({})", short));
                                            }
                                            Err(e) => {
                                                let msg = format!("{}", e);
                                                if msg.contains("nothing to commit") {
                                                    // Already committed — move to Done
                                                    self.set_status_message("Nothing to commit — moved to Done".into());
                                                    let _ = self.db.update_cue_status(
                                                        cue_id,
                                                        CueStatus::Done,
                                                    );
                                                    let _ = self.db.log_activity(cue_id, "Moved to Done (already committed)");
                                                } else {
                                                    self.set_status_message(format!("Commit failed: {}", e));
                                                }
                                            }
                                        }
                                    }
                                }
                                self.reload_git_info();
                                self.reload_commit_history();
                            }
                            CueAction::RevertReview(cue_id) => {
                                if let Ok(Some(exec)) =
                                    self.db.get_latest_execution(cue_id)
                                {
                                    if let Some(ref diff) = exec.diff {
                                        let file_paths = git::parse_diff_file_paths_for_repo(&self.project_root,diff);
                                        if let Err(e) = git::revert_files(
                                            &self.project_root,
                                            &file_paths,
                                        ) {
                                            self.set_status_message(format!("Revert failed: {}", e));
                                        }
                                    }
                                }
                                let _ = self.db.update_cue_status(
                                    cue_id,
                                    CueStatus::Inbox,
                                );
                                let _ = self.db.log_activity(cue_id, "Reverted");
                                // Reload file to show reverted content
                                if let Some(ref path) = self.viewer.current_file {
                                    let p = path.clone();
                                    self.load_file(p);
                                }
                                self.reload_git_info();
                            }
                            CueAction::ReplyReview(cue_id, reply_text) => {
                                self.reply_inputs.remove(&cue_id);
                                let _ = self.db.log_activity(cue_id, "Reply sent");
                                self.trigger_claude_reply(cue_id, &reply_text, &[]);
                            }
                            CueAction::ShowRunningLog(cue_id) => {
                                // Load all executions for conversation history
                                if let Ok(execs) = self.db.get_all_executions(cue_id) {
                                    // Load the latest log into running_logs if not already in memory
                                    if !self.claude.running_logs.contains_key(&cue_id) {
                                        if let Some(last) = execs.last() {
                                            if let Some(ref log_text) = last.log {
                                                self.claude.running_logs.insert(
                                                    cue_id,
                                                    (log_text.clone(), CliProvider::Claude),
                                                );
                                            }
                                        }
                                    }
                                    self.claude.conversation_history = execs;
                                }
                                self.dismiss_central_overlays();
                                self.claude.show_log = Some(cue_id);
                            }
                            CueAction::ShowAgentRuns(cue_id) => {
                                self.dismiss_central_overlays();
                                self.show_agent_runs_for_cue = Some(cue_id);
                            }
                            CueAction::CommitAll => {
                                // Gather summaries from all Review cues
                                let review_cues: Vec<&Cue> = self
                                    .cues
                                    .iter()
                                    .filter(|c| c.status == CueStatus::Review)
                                    .collect();
                                if review_cues.is_empty() {
                                    self.set_status_message("No cues in Review".into());
                                } else {
                                    // Build commit message summarizing all Review cues
                                    let summary_lines: Vec<String> = review_cues
                                        .iter()
                                        .map(|c| {
                                            let text: String = c.text.lines().next().unwrap_or(&c.text).to_string();
                                            if text.len() > 80 {
                                                format!("- {}...", crate::app::truncate_str(&text, 77))
                                            } else {
                                                format!("- {}", text)
                                            }
                                        })
                                        .collect();
                                    let commit_msg = format!(
                                        "Dirigent: {} cue{} committed\n\n{}",
                                        review_cues.len(),
                                        if review_cues.len() == 1 { "" } else { "s" },
                                        summary_lines.join("\n"),
                                    );
                                    let review_ids: Vec<i64> =
                                        review_cues.iter().map(|c| c.id).collect();
                                    match git::commit_all(&self.project_root, &commit_msg) {
                                        Ok(hash) => {
                                            let short = &hash[..7.min(hash.len())];
                                            self.set_status_message(format!(
                                                "Committed all: {} ({} cue{})",
                                                short,
                                                review_ids.len(),
                                                if review_ids.len() == 1 { "" } else { "s" },
                                            ));
                                            for cue_id in &review_ids {
                                                let _ = self.db.update_cue_status(
                                                    *cue_id,
                                                    CueStatus::Done,
                                                );
                                                let _ = self.db.log_activity(*cue_id, &format!("Committed ({})", short));
                                            }
                                        }
                                        Err(e) => {
                                            self.set_status_message(format!(
                                                "Commit all failed: {}",
                                                e
                                            ));
                                        }
                                    }
                                    self.reload_git_info();
                                    self.reload_commit_history();
                                }
                            }
                            CueAction::QueueNext => {
                                // Add to run queue (will start when all running cues finish)
                                if !self.run_queue.contains(&id) {
                                    self.run_queue.push(id);
                                    let _ = self.db.log_activity(id, "Queued (run next)");
                                    let preview = self.cue_preview(id);
                                    self.set_status_message(format!(
                                        "\"{}\" queued — will run after current runs finish",
                                        preview
                                    ));
                                }
                            }
                            CueAction::ScheduleRun(input) => {
                                if let Some(duration) = parse_schedule_duration(&input) {
                                    let when = Instant::now() + duration;
                                    self.scheduled_runs.insert(id, when);
                                    self.schedule_inputs.remove(&id);
                                    let _ = self.db.log_activity(id, &format!("Scheduled ({})", input));
                                    let preview = self.cue_preview(id);
                                    self.set_status_message(format!(
                                        "\"{}\" scheduled to run in {}",
                                        preview, input
                                    ));
                                } else {
                                    self.set_status_message(format!(
                                        "Invalid schedule format: \"{}\" — use e.g. 5m, 2h, 30s",
                                        input
                                    ));
                                }
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
                                // Auto-detect PR number from existing PR cues and import directly
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
                            CueAction::TagAllReview(tag) => {
                                let review_ids: Vec<i64> = self
                                    .cues
                                    .iter()
                                    .filter(|c| c.status == CueStatus::Review)
                                    .map(|c| c.id)
                                    .collect();
                                for cue_id in &review_ids {
                                    let _ = self.db.update_cue_tag(*cue_id, Some(&tag));
                                    let _ = self.db.log_activity(*cue_id, &format!("Tagged: {}", tag));
                                }
                                self.tag_all_review_input = None;
                                self.set_status_message(format!(
                                    "Tagged {} Review cue{} with \"{}\"",
                                    review_ids.len(),
                                    if review_ids.len() == 1 { "" } else { "s" },
                                    tag
                                ));
                            }
                        }
                        self.reload_cues();
                    }
                });

                // Pixelated lava lamp — visible while a cue is running
                if self.settings.lava_lamp_enabled && self.cues.iter().any(|c| c.status == CueStatus::Ready) {
                    let margin = 8.0;
                    let scale = if self.lava_lamp_big { 3.0 } else { 1.0 };
                    let (lamp_w, lamp_h) = super::lava_lamp::size(scale);
                    let origin = egui::pos2(
                        panel_rect.right() - lamp_w - margin,
                        panel_rect.bottom() - lamp_h - margin,
                    );
                    let lamp_rect = egui::Rect::from_min_size(
                        origin,
                        egui::vec2(lamp_w, lamp_h),
                    );
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
            });
    }
}

/// Parse a schedule duration string like "5m", "2h", "30s" into a `Duration`.
fn parse_schedule_duration(input: &str) -> Option<std::time::Duration> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    let (num_str, suffix) = if input.ends_with('m') {
        (&input[..input.len() - 1], 'm')
    } else if input.ends_with('h') {
        (&input[..input.len() - 1], 'h')
    } else if input.ends_with('s') {
        (&input[..input.len() - 1], 's')
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
