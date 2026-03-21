use std::time::Instant;

use eframe::egui;

use super::super::{icon, CueAction, DirigentApp, SPACE_XS};
use crate::db::{Cue, CueStatus};

impl DirigentApp {
    pub(in crate::app) fn render_cue_card(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        status: CueStatus,
    ) {
        let fs = self.settings.font_size;
        let avail_w = ui.available_width();
        let frame_resp = self.semantic.card_frame().show(ui, |ui| {
            ui.set_min_width(avail_w - 22.0);
            // Cue text - inline editable for Inbox
            let is_editing = self.editing_cue.as_ref().map(|e| e.id) == Some(cue.id);
            if is_editing {
                let editing = self.editing_cue.as_mut().unwrap();
                let response = ui.text_edit_multiline(&mut editing.text);
                ui.horizontal(|ui| {
                    if ui.small_button(icon("\u{2713} Save", fs)).clicked() {
                        actions.push((
                            cue.id,
                            CueAction::SaveEdit(self.editing_cue.as_ref().unwrap().text.clone()),
                        ));
                    }
                    if ui.small_button(icon("\u{2715} Cancel", fs)).clicked() {
                        actions.push((cue.id, CueAction::CancelEdit));
                    }
                });
                // Request focus only once when editing starts
                let editing = self.editing_cue.as_mut().unwrap();
                if !editing.focus_requested {
                    response.request_focus();
                    editing.focus_requested = true;
                }
            } else {
                // Collapse long cue text (>50 words or >10 lines) unless expanded
                let line_count = cue.text.lines().count();
                let word_count = cue.text.split_whitespace().count();
                let is_long = line_count > 10 || word_count > 50;
                let is_expanded = self.cue_text_expanded.contains(&cue.id);

                let display_text = if is_long && !is_expanded {
                    // Truncate to first 5 lines or ~50 words
                    let truncated: String = cue.text.lines().take(5).collect::<Vec<_>>().join("\n");
                    // Further trim by word count if needed
                    let words: Vec<&str> = truncated.split_whitespace().collect();
                    if words.len() > 50 {
                        format!("{}…", words[..50].join(" "))
                    } else {
                        format!("{}…", truncated)
                    }
                } else {
                    cue.text.clone()
                };

                let label_response = ui.add(egui::Label::new(&display_text).wrap());
                // Double-click label to edit (Inbox/Backlog)
                if matches!(status, CueStatus::Inbox | CueStatus::Backlog)
                    && !is_editing
                    && label_response.double_clicked()
                {
                    actions.push((cue.id, CueAction::StartEdit(cue.text.clone())));
                }
                // Single-click to show diff (Review/Done/Archived)
                if matches!(
                    status,
                    CueStatus::Review | CueStatus::Done | CueStatus::Archived
                ) && label_response.clicked()
                {
                    actions.push((cue.id, CueAction::ShowDiff(cue.id)));
                }

                // Show expand/collapse toggle for long cues
                if is_long {
                    let toggle_label = if is_expanded {
                        "\u{25B4} Show less"
                    } else {
                        "\u{25BE} Show more"
                    };
                    if ui
                        .add(
                            egui::Label::new(
                                egui::RichText::new(toggle_label)
                                    .small()
                                    .color(self.semantic.accent),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .clicked()
                    {
                        if is_expanded {
                            self.cue_text_expanded.remove(&cue.id);
                        } else {
                            self.cue_text_expanded.insert(cue.id);
                        }
                    }
                }
            }

            // Source label badge, tag badge, and image count
            let has_badge =
                cue.source_label.is_some() || cue.tag.is_some() || !cue.attached_images.is_empty();
            if has_badge {
                ui.horizontal(|ui| {
                    if let Some(ref label) = cue.source_label {
                        let badge_color = source_label_color(label);
                        let badge = egui::RichText::new(label)
                            .small()
                            .background_color(badge_color)
                            .color(self.semantic.badge_text);
                        ui.label(badge);
                    }
                    if let Some(ref tag) = cue.tag {
                        let badge_color = tag_badge_color(tag);
                        let badge = egui::RichText::new(format!("\u{1F3F7} {}", tag))
                            .small()
                            .background_color(badge_color)
                            .color(self.semantic.badge_text);
                        ui.label(badge);
                    }
                    if !cue.attached_images.is_empty() {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} image{}",
                                cue.attached_images.len(),
                                if cue.attached_images.len() == 1 {
                                    ""
                                } else {
                                    "s"
                                }
                            ))
                            .small()
                            .color(self.semantic.accent),
                        );
                    }
                });
            }

            // File:line link or "Global" label
            if cue.file_path.is_empty() {
                ui.label(
                    egui::RichText::new("Global")
                        .small()
                        .color(self.semantic.global_label()),
                );
            } else {
                let location = if let Some(end) = cue.line_number_end {
                    format!("{}:{}-{}", cue.file_path, cue.line_number, end)
                } else {
                    format!("{}:{}", cue.file_path, cue.line_number)
                };
                if ui
                    .small_button(&location)
                    .on_hover_text("Navigate to this location")
                    .clicked()
                {
                    actions.push((
                        cue.id,
                        CueAction::Navigate(
                            cue.file_path.clone(),
                            cue.line_number,
                            cue.line_number_end,
                        ),
                    ));
                }
            }

            // Action buttons — separated from content zone
            ui.add_space(SPACE_XS);
            ui.horizontal_wrapped(|ui| {
                match cue.status {
                    CueStatus::Inbox => {
                        let is_queued = self.run_queue.contains(&cue.id);
                        let is_scheduled = self.scheduled_runs.contains_key(&cue.id);

                        if is_queued || is_scheduled {
                            // Show queued/scheduled state with cancel button
                            let label = if is_queued {
                                "\u{23F3} Queued".to_string()
                            } else if let Some(&when) = self.scheduled_runs.get(&cue.id) {
                                let remaining = when.saturating_duration_since(Instant::now());
                                let secs = remaining.as_secs();
                                if secs < 60 {
                                    format!("\u{23F2} {}s", secs)
                                } else if secs < 3600 {
                                    format!("\u{23F2} {}:{:02}", secs / 60, secs % 60)
                                } else {
                                    format!("\u{23F2} {}h{}m", secs / 3600, (secs % 3600) / 60)
                                }
                            } else {
                                "\u{23F3} Pending".to_string()
                            };
                            ui.label(
                                egui::RichText::new(&label)
                                    .small()
                                    .color(self.semantic.accent),
                            );
                            if is_scheduled {
                                ui.ctx()
                                    .request_repaint_after(std::time::Duration::from_secs(1));
                            }
                            if ui
                                .small_button(icon("\u{2715} Cancel", fs))
                                .on_hover_text("Cancel queued/scheduled run")
                                .clicked()
                            {
                                actions.push((cue.id, CueAction::CancelQueue));
                            }
                        } else {
                            // Normal Inbox buttons
                            if !is_editing {
                                if ui.small_button("Edit").on_hover_text("Edit cue").clicked() {
                                    actions.push((cue.id, CueAction::StartEdit(cue.text.clone())));
                                }
                            }
                            // Split "Run" button: main button runs now, dropdown arrow for options
                            if ui
                                .small_button(icon("\u{25B6} Run", fs))
                                .on_hover_text("Send to Claude now")
                                .clicked()
                            {
                                actions.push((cue.id, CueAction::MoveTo(CueStatus::Ready)));
                            }
                            let dropdown_btn =
                                ui.small_button("\u{25BE}").on_hover_text("Run options");
                            egui::Popup::menu(&dropdown_btn)
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
                                .show(|ui| {
                                    ui.set_min_width(160.0);
                                    if ui.button(icon("\u{25B6} Run now", fs)).clicked() {
                                        actions.push((cue.id, CueAction::MoveTo(CueStatus::Ready)));
                                    }
                                    if ui
                                        .button(icon("\u{23ED} Run next", fs))
                                        .on_hover_text("Run after all current runs finish")
                                        .clicked()
                                    {
                                        actions.push((cue.id, CueAction::QueueNext));
                                    }
                                    if ui
                                        .button(icon("\u{23F2} Schedule...", fs))
                                        .on_hover_text("Schedule run after a delay (e.g. 5m, 2h)")
                                        .clicked()
                                    {
                                        self.schedule_inputs
                                            .entry(cue.id)
                                            .or_insert_with(|| "5m".to_string());
                                    }
                                });
                            if ui
                                .small_button(icon("\u{2713} Done", fs))
                                .on_hover_text("Mark done (no Claude)")
                                .clicked()
                            {
                                actions.push((cue.id, CueAction::MoveTo(CueStatus::Done)));
                            }
                            if ui
                                .small_button(icon("\u{2193} Backlog", fs))
                                .on_hover_text("Move to Backlog")
                                .clicked()
                            {
                                actions.push((cue.id, CueAction::MoveTo(CueStatus::Backlog)));
                            }
                        }
                    }
                    CueStatus::Ready => {
                        let elapsed = self.format_elapsed(cue.id);
                        let label = if elapsed.is_empty() {
                            "\u{25CF} Running...".to_string()
                        } else {
                            format!("\u{25CF} Running... {}", elapsed)
                        };
                        if ui
                            .small_button(icon(&label, fs).color(self.semantic.accent))
                            .on_hover_text("View Claude's progress")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::ShowRunningLog(cue.id)));
                        }
                        ui.ctx()
                            .request_repaint_after(super::super::ELAPSED_REPAINT);
                        if ui
                            .small_button(icon("\u{2715} Cancel", fs))
                            .on_hover_text("Cancel and move back to Inbox")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::MoveTo(CueStatus::Inbox)));
                        }
                    }
                    CueStatus::Review => {
                        if ui
                            .small_button(icon("\u{25B6} Diff", fs))
                            .on_hover_text("View the diff")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::ShowDiff(cue.id)));
                        }
                        if ui
                            .small_button(icon("Log", fs))
                            .on_hover_text("View Claude's output log")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::ShowRunningLog(cue.id)));
                        }
                        if !self.settings.agents.is_empty() {
                            if ui
                                .small_button(icon("Agents", fs))
                                .on_hover_text("View agent run logs (format, lint, build, test)")
                                .clicked()
                            {
                                actions.push((cue.id, CueAction::ShowAgentRuns(cue.id)));
                            }
                        }
                        if ui
                            .small_button(icon("\u{21A9} Reply", fs))
                            .on_hover_text("Send feedback to Claude for another iteration")
                            .clicked()
                        {
                            // Toggle reply input visibility
                            if self.reply_inputs.contains_key(&cue.id) {
                                self.reply_inputs.remove(&cue.id);
                            } else {
                                self.reply_inputs.insert(cue.id, String::new());
                            }
                        }
                        if ui
                            .small_button(icon("\u{2713} Commit", fs))
                            .on_hover_text("Commit the applied changes")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::CommitReview(cue.id)));
                        }
                        if ui
                            .small_button(icon("\u{21BA} Revert", fs))
                            .on_hover_text("Revert changes and move back to Inbox")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::RevertReview(cue.id)));
                        }
                    }
                    CueStatus::Done => {
                        ui.label(icon("\u{2713}", fs).color(self.semantic.success));
                        // Show highlighted Push button when there are unpushed commits
                        // and Claude's log for this cue mentions pushing
                        let log_mentions_push = self
                            .claude
                            .running_logs
                            .get(&cue.id)
                            .map(|(log, _)| {
                                let lower = log.to_lowercase();
                                lower.contains("push")
                            })
                            .unwrap_or(false);
                        if self.git.ahead_of_remote > 0 && !self.git.pushing && log_mentions_push {
                            let push_btn = egui::Button::new(
                                icon("\u{2191} Push", fs).color(self.semantic.badge_text),
                            )
                            .fill(self.semantic.accent);
                            if ui
                                .add(push_btn)
                                .on_hover_text(format!(
                                    "Push to remote ({} commit{} ahead)",
                                    self.git.ahead_of_remote,
                                    if self.git.ahead_of_remote == 1 {
                                        ""
                                    } else {
                                        "s"
                                    }
                                ))
                                .clicked()
                            {
                                actions.push((cue.id, CueAction::Push));
                            }
                        }
                        if ui
                            .small_button(icon("Log", fs))
                            .on_hover_text("View Claude's output log")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::ShowRunningLog(cue.id)));
                        }
                        if !self.settings.agents.is_empty() {
                            if ui
                                .small_button(icon("Agents", fs))
                                .on_hover_text("View agent run logs (format, lint, build, test)")
                                .clicked()
                            {
                                actions.push((cue.id, CueAction::ShowAgentRuns(cue.id)));
                            }
                        }
                        if ui
                            .small_button(icon("\u{21A9} Reply", fs))
                            .on_hover_text("Send follow-up feedback to Claude")
                            .clicked()
                        {
                            // Toggle reply input visibility
                            if self.reply_inputs.contains_key(&cue.id) {
                                self.reply_inputs.remove(&cue.id);
                            } else {
                                self.reply_inputs.insert(cue.id, String::new());
                            }
                        }
                        // "Notify PR" button for PR-sourced cues
                        let is_pr_sourced = cue
                            .source_ref
                            .as_ref()
                            .map(|s| s.starts_with("pr"))
                            .unwrap_or(false);
                        let already_notified = is_pr_sourced
                            && self
                                .db
                                .get_last_activity_matching(cue.id, "Notified PR")
                                .ok()
                                .flatten()
                                .is_some();
                        if is_pr_sourced && !already_notified && !self.git.notifying_pr {
                            if ui
                                .small_button(icon("\u{1F514} Notify PR", fs))
                                .on_hover_text(
                                    "Reply to the PR comment that this finding was fixed",
                                )
                                .clicked()
                            {
                                actions.push((cue.id, CueAction::NotifyPR(cue.id)));
                            }
                        } else if is_pr_sourced && already_notified {
                            ui.label(
                                egui::RichText::new("\u{2713} PR notified")
                                    .small()
                                    .color(self.semantic.success),
                            );
                        }
                        if ui
                            .small_button(icon("\u{2193} Archive", fs))
                            .on_hover_text("Move to Archived")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::MoveTo(CueStatus::Archived)));
                        }
                        if ui
                            .small_button(icon("\u{21BA} Reopen", fs))
                            .on_hover_text("Move back to Inbox")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::MoveTo(CueStatus::Inbox)));
                        }
                    }
                    CueStatus::Archived => {
                        if ui
                            .small_button(icon("Log", fs))
                            .on_hover_text("View Claude's output log")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::ShowRunningLog(cue.id)));
                        }
                        if !self.settings.agents.is_empty() {
                            if ui
                                .small_button(icon("Agents", fs))
                                .on_hover_text("View agent run logs (format, lint, build, test)")
                                .clicked()
                            {
                                actions.push((cue.id, CueAction::ShowAgentRuns(cue.id)));
                            }
                        }
                        if ui
                            .small_button(icon("\u{21BA} Unarchive", fs))
                            .on_hover_text("Move back to Done")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::MoveTo(CueStatus::Done)));
                        }
                    }
                    CueStatus::Backlog => {
                        if !is_editing {
                            if ui.small_button("Edit").on_hover_text("Edit cue").clicked() {
                                actions.push((cue.id, CueAction::StartEdit(cue.text.clone())));
                            }
                        }
                        if ui
                            .small_button(icon("\u{2191} Inbox", fs))
                            .on_hover_text("Move to Inbox")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::MoveTo(CueStatus::Inbox)));
                        }
                        if ui
                            .small_button(icon("\u{25B6} Run", fs))
                            .on_hover_text("Send to Claude")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::MoveTo(CueStatus::Ready)));
                        }
                    }
                }

                // Overflow menu
                let more_btn = ui.small_button("\u{2026}").on_hover_text("More actions");
                egui::Popup::menu(&more_btn)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        ui.set_min_width(140.0);
                        let is_expanded = self.logbook_expanded.contains(&cue.id);
                        let activity_label = if is_expanded {
                            "\u{25BE} Activity"
                        } else {
                            "\u{25B8} Activity"
                        };
                        if ui.button(activity_label).clicked() {
                            if is_expanded {
                                self.logbook_expanded.remove(&cue.id);
                            } else {
                                self.logbook_expanded.insert(cue.id);
                            }
                        }
                        // Tag submenu
                        let tag_label = if cue.tag.is_some() {
                            "\u{1F3F7} Edit Tag"
                        } else {
                            "\u{1F3F7} Add Tag"
                        };
                        if ui.button(tag_label).clicked() {
                            let current = cue.tag.clone().unwrap_or_default();
                            self.tag_inputs.insert(cue.id, current);
                        }
                        if cue.tag.is_some() {
                            if ui.button("\u{2715} Remove Tag").clicked() {
                                actions.push((cue.id, CueAction::SetTag(None)));
                            }
                        }
                        // Create PR (only on non-default branches)
                        {
                            let is_default_branch = self
                                .git
                                .info
                                .as_ref()
                                .map(|i| i.branch == "main" || i.branch == "master")
                                .unwrap_or(true);
                            if !is_default_branch && !self.git.creating_pr {
                                if ui.button("Create PR").clicked() {
                                    actions.push((cue.id, CueAction::CreatePR));
                                }
                            }
                        }
                        ui.separator();
                        if ui.button(icon("\u{2715} Delete", fs)).clicked() {
                            actions.push((cue.id, CueAction::Delete));
                        }
                    });
            });

            // Schedule input field (visible when toggled via Run dropdown)
            let mut cancel_schedule = false;
            if let Some(schedule_text) = self.schedule_inputs.get_mut(&cue.id) {
                ui.add_space(SPACE_XS);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("\u{23F2}").color(self.semantic.accent));
                    let response = ui.add(
                        egui::TextEdit::singleline(schedule_text)
                            .desired_width(60.0)
                            .hint_text("5m, 2h"),
                    );
                    let submit = ui
                        .small_button(icon("\u{2713} Go", fs))
                        .on_hover_text("Schedule this run")
                        .clicked()
                        || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    if submit && !schedule_text.trim().is_empty() {
                        actions.push((cue.id, CueAction::ScheduleRun(schedule_text.clone())));
                    }
                    if ui
                        .small_button("\u{2715}")
                        .on_hover_text("Cancel")
                        .clicked()
                    {
                        cancel_schedule = true;
                    }
                });
            }
            if cancel_schedule {
                self.schedule_inputs.remove(&cue.id);
            }

            // Reply input field (visible when toggled for Review/Done cues)
            if let Some(reply_text) = self.reply_inputs.get_mut(&cue.id) {
                ui.add_space(SPACE_XS);
                let response = ui.add(
                    egui::TextEdit::multiline(reply_text)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .hint_text("Describe what needs to change..."),
                );
                // Submit on Cmd+Enter
                let submit = ui
                    .small_button(icon("\u{25B6} Send", fs))
                    .on_hover_text("Send feedback to Claude (also Cmd+Enter)")
                    .clicked()
                    || (response.has_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command));
                if submit && !reply_text.trim().is_empty() {
                    actions.push((cue.id, CueAction::ReplyReview(cue.id, reply_text.clone())));
                }
            }

            // Tag input field (visible when toggled via overflow menu)
            let mut cancel_tag = false;
            if let Some(tag_text) = self.tag_inputs.get_mut(&cue.id) {
                ui.add_space(SPACE_XS);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("\u{1F3F7}").color(self.semantic.accent));
                    let response = ui.add(
                        egui::TextEdit::singleline(tag_text)
                            .desired_width(100.0)
                            .hint_text("Tag name"),
                    );
                    let submit = ui
                        .small_button(icon("\u{2713} Set", fs))
                        .on_hover_text("Set tag")
                        .clicked()
                        || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    if submit {
                        let tag_val = if tag_text.trim().is_empty() {
                            None
                        } else {
                            Some(tag_text.trim().to_string())
                        };
                        actions.push((cue.id, CueAction::SetTag(tag_val)));
                    }
                    if ui
                        .small_button("\u{2715}")
                        .on_hover_text("Cancel")
                        .clicked()
                    {
                        cancel_tag = true;
                    }
                });
            }
            if cancel_tag {
                self.tag_inputs.remove(&cue.id);
            }

            // Activity logbook (shown when expanded via overflow menu)
            if self.logbook_expanded.contains(&cue.id) {
                if let Ok(entries) = self.db.get_activities(cue.id) {
                    if entries.is_empty() {
                        ui.label(
                            egui::RichText::new("No activity yet")
                                .small()
                                .color(self.semantic.muted_text()),
                        );
                    } else {
                        // Fetch agent runs for this cue (for expandable output)
                        let agent_runs = self.db.get_agent_runs_for_cue(cue.id).unwrap_or_default();

                        for entry in &entries {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(&entry.timestamp)
                                        .small()
                                        .color(self.semantic.muted_text()),
                                );
                                // Check if this is an agent event with output
                                let is_agent_event = entry.event.contains("passed")
                                    || entry.event.contains("failed")
                                    || entry.event.contains("error");
                                let agent_kind_for_event = if is_agent_event {
                                    ["Format", "Lint", "Build", "Test"]
                                        .iter()
                                        .find(|k| entry.event.starts_with(*k))
                                        .map(|k| k.to_string())
                                } else {
                                    None
                                };

                                if agent_kind_for_event.is_some() {
                                    let key = (cue.id, entry.timestamp.clone());
                                    let is_expanded = self.agent_output_expanded.contains(&key);
                                    let arrow = if is_expanded { "\u{25BE}" } else { "\u{25B8}" };
                                    if ui
                                        .add(
                                            egui::Label::new(
                                                egui::RichText::new(format!(
                                                    "{} {}",
                                                    arrow, &entry.event
                                                ))
                                                .small(),
                                            )
                                            .sense(egui::Sense::click()),
                                        )
                                        .clicked()
                                    {
                                        if is_expanded {
                                            self.agent_output_expanded.remove(&key);
                                        } else {
                                            self.agent_output_expanded.insert(key);
                                        }
                                    }
                                } else {
                                    ui.label(egui::RichText::new(&entry.event).small());
                                }
                            });

                            // Render agent output block outside the horizontal layout
                            if let Some(ref kind_label) = {
                                let is_agent_event = entry.event.contains("passed")
                                    || entry.event.contains("failed")
                                    || entry.event.contains("error");
                                if is_agent_event {
                                    ["Format", "Lint", "Build", "Test"]
                                        .iter()
                                        .find(|k| entry.event.starts_with(*k))
                                        .map(|k| k.to_string())
                                } else {
                                    None
                                }
                            } {
                                let key = (cue.id, entry.timestamp.clone());
                                if self.agent_output_expanded.contains(&key) {
                                    let kind_str = kind_label.to_lowercase();
                                    if let Some(run) = agent_runs
                                        .iter()
                                        .find(|r| {
                                            r.agent_kind == kind_str
                                                && r.started_at == entry.timestamp
                                        })
                                        .or_else(|| {
                                            agent_runs.iter().find(|r| r.agent_kind == kind_str)
                                        })
                                    {
                                        let output = if run.output.len() > 2000 {
                                            format!(
                                                "{}...\n(truncated, {} bytes total)",
                                                crate::app::truncate_str(&run.output, 2000),
                                                run.output.len()
                                            )
                                        } else if run.output.trim().is_empty() {
                                            "(no output)".to_string()
                                        } else {
                                            run.output.clone()
                                        };
                                        egui::Frame::NONE
                                            .inner_margin(egui::Margin::same(4))
                                            .corner_radius(4)
                                            .fill(self.semantic.selection_bg())
                                            .show(ui, |ui| {
                                                egui::ScrollArea::vertical()
                                                    .max_height(200.0)
                                                    .show(ui, |ui| {
                                                        ui.label(
                                                            egui::RichText::new(&output)
                                                                .monospace()
                                                                .small()
                                                                .color(self.semantic.muted_text()),
                                                        );
                                                    });
                                            });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        // Transition flash overlay when cue moves between columns
        if let Some(&when) = self.cue_move_flash.get(&cue.id) {
            let elapsed = when.elapsed().as_secs_f32();
            if elapsed < 0.6 {
                let alpha = ((0.6 - elapsed) / 0.6 * 50.0) as u8;
                let [r, g, b, _] = self.semantic.accent.to_array();
                ui.painter().rect_filled(
                    frame_resp.response.rect,
                    8,
                    egui::Color32::from_rgba_premultiplied(r, g, b, alpha),
                );
                ui.ctx().request_repaint();
            }
        }

        ui.add_space(SPACE_XS);
    }
}

/// Pick a deterministic badge color for a tag.
fn tag_badge_color(tag: &str) -> egui::Color32 {
    let hash = tag.bytes().fold(5381u32, |acc, b| {
        acc.wrapping_mul(33).wrapping_add(b as u32)
    });
    let colors = [
        egui::Color32::from_rgb(38, 154, 108), // emerald
        egui::Color32::from_rgb(163, 68, 168), // vivid purple
        egui::Color32::from_rgb(206, 120, 36), // tangerine
        egui::Color32::from_rgb(44, 138, 186), // cerulean
        egui::Color32::from_rgb(210, 60, 78),  // coral
        egui::Color32::from_rgb(108, 72, 190), // violet
        egui::Color32::from_rgb(60, 120, 216), // royal blue
        egui::Color32::from_rgb(188, 82, 148), // magenta
    ];
    colors[(hash as usize) % colors.len()]
}

/// Pick a deterministic badge color based on the source label string.
fn source_label_color(label: &str) -> egui::Color32 {
    let hash = label
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    let colors = [
        egui::Color32::from_rgb(60, 120, 216), // royal blue
        egui::Color32::from_rgb(163, 68, 168), // vivid purple
        egui::Color32::from_rgb(206, 120, 36), // tangerine
        egui::Color32::from_rgb(38, 154, 108), // emerald
        egui::Color32::from_rgb(210, 60, 78),  // coral
        egui::Color32::from_rgb(44, 138, 186), // cerulean
        egui::Color32::from_rgb(188, 82, 148), // magenta
        egui::Color32::from_rgb(108, 72, 190), // violet
    ];
    colors[(hash as usize) % colors.len()]
}
