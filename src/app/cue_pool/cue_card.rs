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
        let avail_w = ui.available_width();
        let frame_resp = self.semantic.card_frame().show(ui, |ui| {
            ui.set_min_width(avail_w - 22.0);
            self.render_cue_text(ui, cue, actions, status);
            self.render_badge_row(ui, cue);
            self.render_file_location(ui, cue, actions);
            self.render_run_metrics(ui, cue, status);
            ui.add_space(SPACE_XS);
            ui.horizontal_wrapped(|ui| {
                self.render_status_buttons(ui, cue, actions);
                self.render_overflow_menu(ui, cue, actions);
            });
            self.render_schedule_input(ui, cue, actions);
            self.render_reply_input(ui, cue, actions);
            self.render_tag_input(ui, cue, actions);
            self.render_activity_logbook(ui, cue);
        });

        self.render_transition_flash(ui, cue, &frame_resp);
        ui.add_space(SPACE_XS);
    }

    fn render_cue_text(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        status: CueStatus,
    ) {
        let fs = self.settings.font_size;
        let is_editing = self.editing_cue.as_ref().map(|e| e.id) == Some(cue.id);
        if is_editing {
            self.render_editing_cue(ui, cue, actions, fs);
        } else {
            self.render_display_cue(ui, cue, actions, status);
        }
    }

    fn render_editing_cue(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        fs: f32,
    ) {
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
        let editing = self.editing_cue.as_mut().unwrap();
        if !editing.focus_requested {
            response.request_focus();
            editing.focus_requested = true;
        }
    }

    fn render_display_cue(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        status: CueStatus,
    ) {
        let display_text = compute_display_text(cue, self.cue_text_expanded.contains(&cue.id));
        let label_response = ui.add(egui::Label::new(&display_text).wrap());

        if matches!(status, CueStatus::Inbox | CueStatus::Backlog)
            && label_response.double_clicked()
        {
            actions.push((cue.id, CueAction::StartEdit(cue.text.clone())));
        }
        if matches!(
            status,
            CueStatus::Review | CueStatus::Done | CueStatus::Archived
        ) && label_response.clicked()
        {
            actions.push((cue.id, CueAction::ShowDiff(cue.id)));
        }

        self.render_expand_collapse_toggle(ui, cue);
    }

    fn render_expand_collapse_toggle(&mut self, ui: &mut egui::Ui, cue: &Cue) {
        let line_count = cue.text.lines().count();
        let word_count = cue.text.split_whitespace().count();
        let is_long = line_count > 10 || word_count > 50;
        if !is_long {
            return;
        }
        let is_expanded = self.cue_text_expanded.contains(&cue.id);
        let toggle_label = if is_expanded {
            "\u{25B4} Show less"
        } else {
            "\u{25BE} Show more"
        };
        let clicked = ui
            .add(
                egui::Label::new(
                    egui::RichText::new(toggle_label)
                        .small()
                        .color(self.semantic.accent),
                )
                .sense(egui::Sense::click()),
            )
            .clicked();
        if clicked {
            if is_expanded {
                self.cue_text_expanded.remove(&cue.id);
            } else {
                self.cue_text_expanded.insert(cue.id);
            }
        }
    }

    fn render_badge_row(&self, ui: &mut egui::Ui, cue: &Cue) {
        let has_badge =
            cue.source_label.is_some() || cue.tag.is_some() || !cue.attached_images.is_empty();
        if !has_badge {
            return;
        }
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
                let plural = if cue.attached_images.len() == 1 {
                    ""
                } else {
                    "s"
                };
                ui.label(
                    egui::RichText::new(format!("{} image{}", cue.attached_images.len(), plural))
                        .small()
                        .color(self.semantic.accent),
                );
            }
        });
    }

    fn render_file_location(
        &self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        if cue.file_path.is_empty() {
            ui.label(
                egui::RichText::new("Global")
                    .small()
                    .color(self.semantic.global_label()),
            );
            return;
        }
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
                CueAction::Navigate(cue.file_path.clone(), cue.line_number, cue.line_number_end),
            ));
        }
    }

    fn render_run_metrics(&self, ui: &mut egui::Ui, cue: &Cue, status: CueStatus) {
        if !matches!(
            status,
            CueStatus::Review | CueStatus::Done | CueStatus::Archived
        ) {
            return;
        }
        let Some(metrics) = self.latest_exec_cache.get(&cue.id) else {
            return;
        };
        let mut parts = Vec::new();
        if let Some(turns) = metrics.num_turns {
            parts.push(format!("{} turns", turns));
        }
        if let Some(ms) = metrics.duration_ms {
            parts.push(format!("{:.1}s", ms as f64 / 1000.0));
        }
        if let Some(cost) = metrics.cost_usd {
            parts.push(format!("${:.4}", cost));
        }
        if !parts.is_empty() {
            ui.label(
                egui::RichText::new(parts.join("  "))
                    .small()
                    .color(self.semantic.muted_text()),
            );
        }
    }

    fn render_status_buttons(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        match cue.status {
            CueStatus::Inbox => self.render_inbox_buttons(ui, cue, actions),
            CueStatus::Ready => self.render_ready_buttons(ui, cue, actions),
            CueStatus::Review => self.render_review_buttons(ui, cue, actions),
            CueStatus::Done => self.render_done_buttons(ui, cue, actions),
            CueStatus::Archived => self.render_archived_buttons(ui, cue, actions),
            CueStatus::Backlog => self.render_backlog_buttons(ui, cue, actions),
        }
    }

    fn render_inbox_buttons(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let fs = self.settings.font_size;
        let is_queued = self.run_queue.contains(&cue.id);
        let is_scheduled = self.scheduled_runs.contains_key(&cue.id);

        if is_queued || is_scheduled {
            self.render_queued_state(ui, cue, actions, fs);
        } else {
            self.render_normal_inbox_buttons(ui, cue, actions, fs);
        }
    }

    fn render_queued_state(
        &self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        fs: f32,
    ) {
        let is_queued = self.run_queue.contains(&cue.id);
        let is_scheduled = self.scheduled_runs.contains_key(&cue.id);
        let label = format_queue_label(is_queued, self.scheduled_runs.get(&cue.id).copied());
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
    }

    fn render_normal_inbox_buttons(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        fs: f32,
    ) {
        let is_editing = self.editing_cue.as_ref().map(|e| e.id) == Some(cue.id);
        if !is_editing && ui.small_button("Edit").on_hover_text("Edit cue").clicked() {
            actions.push((cue.id, CueAction::StartEdit(cue.text.clone())));
        }
        if ui
            .small_button(icon("\u{25B6} Run", fs))
            .on_hover_text("Send to Claude now")
            .clicked()
        {
            actions.push((cue.id, CueAction::MoveTo(CueStatus::Ready)));
        }
        self.render_run_dropdown(ui, cue, actions, fs);
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

    fn render_run_dropdown(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        fs: f32,
    ) {
        let dropdown_btn = ui.small_button("\u{25BE}").on_hover_text("Run options");
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
    }

    fn render_ready_buttons(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let fs = self.settings.font_size;
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
            .small_button(icon("\u{21A9} Follow-up", fs))
            .on_hover_text("Queue a follow-up prompt for when this run completes")
            .clicked()
        {
            toggle_reply_input(&mut self.reply_inputs, cue.id);
        }
        let queued_count = self
            .follow_up_queue
            .get(&cue.id)
            .map(|v| v.len())
            .unwrap_or(0);
        if queued_count > 0 {
            let plural = if queued_count == 1 { "" } else { "s" };
            ui.label(
                egui::RichText::new(format!("{} follow-up{} queued", queued_count, plural))
                    .small()
                    .color(self.semantic.accent),
            );
        }
        if ui
            .small_button(icon("\u{2715} Cancel", fs))
            .on_hover_text("Cancel and move back to Inbox")
            .clicked()
        {
            actions.push((cue.id, CueAction::MoveTo(CueStatus::Inbox)));
        }
    }

    fn render_review_buttons(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let fs = self.settings.font_size;
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
        if !self.settings.agents.is_empty()
            && ui
                .small_button(icon("Agents", fs))
                .on_hover_text("View agent run logs (format, lint, build, test)")
                .clicked()
        {
            actions.push((cue.id, CueAction::ShowAgentRuns(cue.id)));
        }
        if ui
            .small_button(icon("\u{21A9} Reply", fs))
            .on_hover_text("Send feedback to Claude for another iteration")
            .clicked()
        {
            toggle_reply_input(&mut self.reply_inputs, cue.id);
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

    fn render_done_buttons(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let fs = self.settings.font_size;
        ui.label(icon("\u{2713}", fs).color(self.semantic.success));
        self.render_push_button(ui, cue, actions, fs);
        self.render_log_and_agents_buttons(ui, cue, actions, fs);
        if ui
            .small_button(icon("\u{21A9} Reply", fs))
            .on_hover_text("Send follow-up feedback to Claude")
            .clicked()
        {
            toggle_reply_input(&mut self.reply_inputs, cue.id);
        }
        self.render_pr_notify_button(ui, cue, actions, fs);
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

    fn render_push_button(
        &self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        fs: f32,
    ) {
        let log_mentions_push = self
            .claude
            .running_logs
            .get(&cue.id)
            .map(|(log, _)| log.to_lowercase().contains("push"))
            .unwrap_or(false);
        if self.git.ahead_of_remote == 0 || self.git.pushing || !log_mentions_push {
            return;
        }
        let push_btn = egui::Button::new(icon("\u{2191} Push", fs).color(self.semantic.badge_text))
            .fill(self.semantic.accent);
        let plural = if self.git.ahead_of_remote == 1 {
            ""
        } else {
            "s"
        };
        if ui
            .add(push_btn)
            .on_hover_text(format!(
                "Push to remote ({} commit{} ahead)",
                self.git.ahead_of_remote, plural,
            ))
            .clicked()
        {
            actions.push((cue.id, CueAction::Push));
        }
    }

    fn render_log_and_agents_buttons(
        &self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        fs: f32,
    ) {
        if ui
            .small_button(icon("Log", fs))
            .on_hover_text("View Claude's output log")
            .clicked()
        {
            actions.push((cue.id, CueAction::ShowRunningLog(cue.id)));
        }
        if !self.settings.agents.is_empty()
            && ui
                .small_button(icon("Agents", fs))
                .on_hover_text("View agent run logs (format, lint, build, test)")
                .clicked()
        {
            actions.push((cue.id, CueAction::ShowAgentRuns(cue.id)));
        }
    }

    fn render_pr_notify_button(
        &self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        fs: f32,
    ) {
        let is_pr_sourced = cue
            .source_ref
            .as_ref()
            .map(|s| s.starts_with("pr"))
            .unwrap_or(false);
        if !is_pr_sourced {
            return;
        }
        let already_notified = self
            .db
            .get_last_activity_matching(cue.id, "Notified PR")
            .ok()
            .flatten()
            .is_some();
        if !already_notified && !self.git.notifying_pr {
            if ui
                .small_button(icon("\u{1F514} Notify PR", fs))
                .on_hover_text("Reply to the PR comment that this finding was fixed")
                .clicked()
            {
                actions.push((cue.id, CueAction::NotifyPR(cue.id)));
            }
        } else if already_notified {
            ui.label(
                egui::RichText::new("\u{2713} PR notified")
                    .small()
                    .color(self.semantic.success),
            );
        }
    }

    fn render_archived_buttons(
        &self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let fs = self.settings.font_size;
        self.render_log_and_agents_buttons(ui, cue, actions, fs);
        if ui
            .small_button(icon("\u{21BA} Unarchive", fs))
            .on_hover_text("Move back to Done")
            .clicked()
        {
            actions.push((cue.id, CueAction::MoveTo(CueStatus::Done)));
        }
    }

    fn render_backlog_buttons(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let fs = self.settings.font_size;
        let is_editing = self.editing_cue.as_ref().map(|e| e.id) == Some(cue.id);
        if !is_editing && ui.small_button("Edit").on_hover_text("Edit cue").clicked() {
            actions.push((cue.id, CueAction::StartEdit(cue.text.clone())));
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

    fn render_overflow_menu(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let fs = self.settings.font_size;
        let more_btn = ui.small_button("\u{2026}").on_hover_text("More actions");
        egui::Popup::menu(&more_btn)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(140.0);
                self.render_overflow_activity_toggle(ui, cue);
                self.render_overflow_tag_items(ui, cue, actions);
                self.render_overflow_create_pr(ui, cue, actions);
                ui.separator();
                if ui.button(icon("\u{2715} Delete", fs)).clicked() {
                    actions.push((cue.id, CueAction::Delete));
                }
            });
    }

    fn render_overflow_activity_toggle(&mut self, ui: &mut egui::Ui, cue: &Cue) {
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
    }

    fn render_overflow_tag_items(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let tag_label = if cue.tag.is_some() {
            "\u{1F3F7} Edit Tag"
        } else {
            "\u{1F3F7} Add Tag"
        };
        if ui.button(tag_label).clicked() {
            let current = cue.tag.clone().unwrap_or_default();
            self.tag_inputs.insert(cue.id, current);
        }
        if cue.tag.is_some() && ui.button("\u{2715} Remove Tag").clicked() {
            actions.push((cue.id, CueAction::SetTag(None)));
        }
    }

    fn render_overflow_create_pr(
        &self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let is_default_branch = self
            .git
            .info
            .as_ref()
            .map(|i| i.branch == "main" || i.branch == "master")
            .unwrap_or(true);
        if !is_default_branch && !self.git.creating_pr && ui.button("Create PR").clicked() {
            actions.push((cue.id, CueAction::CreatePR));
        }
    }

    fn render_schedule_input(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let fs = self.settings.font_size;
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
    }

    fn render_reply_input(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let fs = self.settings.font_size;
        let is_running = cue.status == CueStatus::Ready;
        if let Some(reply_text) = self.reply_inputs.get_mut(&cue.id) {
            ui.add_space(SPACE_XS);
            let hint = if is_running {
                "Follow-up prompt (sent when this run completes)..."
            } else {
                "Describe what needs to change..."
            };
            let response = ui.add(
                egui::TextEdit::multiline(reply_text)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text(hint),
            );
            let (btn_label, btn_hover) = if is_running {
                (
                    "\u{23F3} Queue",
                    "Queue follow-up for when this run completes (also Cmd+Enter)",
                )
            } else {
                ("\u{25B6} Send", "Send feedback to Claude (also Cmd+Enter)")
            };
            let submit = ui
                .small_button(icon(btn_label, fs))
                .on_hover_text(btn_hover)
                .clicked()
                || (response.has_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command));
            if submit && !reply_text.trim().is_empty() {
                if is_running {
                    actions.push((cue.id, CueAction::QueueFollowUp(cue.id, reply_text.clone())));
                } else {
                    actions.push((cue.id, CueAction::ReplyReview(cue.id, reply_text.clone())));
                }
            }
        }
    }

    fn render_tag_input(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
    ) {
        let fs = self.settings.font_size;
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
    }

    fn render_activity_logbook(&mut self, ui: &mut egui::Ui, cue: &Cue) {
        if !self.logbook_expanded.contains(&cue.id) {
            return;
        }
        let entries = match self.db.get_activities(cue.id) {
            Ok(e) => e,
            Err(_) => return,
        };
        if entries.is_empty() {
            ui.label(
                egui::RichText::new("No activity yet")
                    .small()
                    .color(self.semantic.muted_text()),
            );
            return;
        }
        let agent_runs = self.db.get_agent_runs_for_cue(cue.id).unwrap_or_default();
        for entry in &entries {
            let agent_kind = detect_agent_kind(&entry.event);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&entry.timestamp)
                        .small()
                        .color(self.semantic.muted_text()),
                );
                if agent_kind.is_some() {
                    self.render_agent_event_toggle(ui, cue, entry);
                } else {
                    ui.label(egui::RichText::new(&entry.event).small());
                }
            });

            if let Some(ref kind_label) = agent_kind {
                self.render_agent_output_block(ui, cue, entry, kind_label, &agent_runs);
            }
        }
    }

    fn render_agent_event_toggle(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        entry: &crate::db::ActivityEntry,
    ) {
        let key = (cue.id, entry.timestamp.clone());
        let is_expanded = self.agent_output_expanded.contains(&key);
        let arrow = if is_expanded { "\u{25BE}" } else { "\u{25B8}" };
        let clicked = ui
            .add(
                egui::Label::new(
                    egui::RichText::new(format!("{} {}", arrow, &entry.event)).small(),
                )
                .sense(egui::Sense::click()),
            )
            .clicked();
        if clicked {
            if is_expanded {
                self.agent_output_expanded.remove(&key);
            } else {
                self.agent_output_expanded.insert(key);
            }
        }
    }

    fn render_agent_output_block(
        &self,
        ui: &mut egui::Ui,
        cue: &Cue,
        entry: &crate::db::ActivityEntry,
        kind_label: &str,
        agent_runs: &[crate::db::AgentRunEntry],
    ) {
        let key = (cue.id, entry.timestamp.clone());
        if !self.agent_output_expanded.contains(&key) {
            return;
        }
        let kind_str = kind_label.to_lowercase();
        let run = agent_runs
            .iter()
            .find(|r| r.agent_kind == kind_str && r.started_at == entry.timestamp)
            .or_else(|| agent_runs.iter().find(|r| r.agent_kind == kind_str));
        let Some(run) = run else { return };
        let output = format_agent_output(&run.output);
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

    fn render_transition_flash(
        &self,
        ui: &mut egui::Ui,
        cue: &Cue,
        frame_resp: &egui::InnerResponse<()>,
    ) {
        let Some(&when) = self.cue_move_flash.get(&cue.id) else {
            return;
        };
        let elapsed = when.elapsed().as_secs_f32();
        if elapsed >= 0.6 {
            return;
        }
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

/// Compute display text for a cue, truncating if it's long and not expanded.
fn compute_display_text(cue: &Cue, is_expanded: bool) -> String {
    let line_count = cue.text.lines().count();
    let word_count = cue.text.split_whitespace().count();
    let is_long = line_count > 10 || word_count > 50;

    if is_long && !is_expanded {
        let truncated: String = cue.text.lines().take(5).collect::<Vec<_>>().join("\n");
        let words: Vec<&str> = truncated.split_whitespace().collect();
        if words.len() > 50 {
            format!("{}\u{2026}", words[..50].join(" "))
        } else {
            format!("{}\u{2026}", truncated)
        }
    } else {
        cue.text.clone()
    }
}

/// Format queue/schedule label for display.
fn format_queue_label(is_queued: bool, scheduled_when: Option<Instant>) -> String {
    if is_queued {
        return "\u{23F3} Queued".to_string();
    }
    if let Some(when) = scheduled_when {
        let remaining = when.saturating_duration_since(Instant::now());
        let secs = remaining.as_secs();
        if secs < 60 {
            return format!("\u{23F2} {}s", secs);
        }
        if secs < 3600 {
            return format!("\u{23F2} {}:{:02}", secs / 60, secs % 60);
        }
        return format!("\u{23F2} {}h{}m", secs / 3600, (secs % 3600) / 60);
    }
    "\u{23F3} Pending".to_string()
}

/// Toggle reply input visibility for a cue.
fn toggle_reply_input(reply_inputs: &mut std::collections::HashMap<i64, String>, cue_id: i64) {
    if let std::collections::hash_map::Entry::Vacant(e) = reply_inputs.entry(cue_id) {
        e.insert(String::new());
    } else {
        reply_inputs.remove(&cue_id);
    }
}

/// Detect if an activity event is an agent event and return its kind label.
fn detect_agent_kind(event: &str) -> Option<String> {
    let is_agent_event =
        event.contains("passed") || event.contains("failed") || event.contains("error");
    if !is_agent_event {
        return None;
    }
    ["Format", "Lint", "Build", "Test"]
        .iter()
        .find(|k| event.starts_with(*k))
        .map(|k| k.to_string())
}

/// Format agent output, truncating if necessary.
fn format_agent_output(output: &str) -> String {
    if output.len() > 2000 {
        format!(
            "{}...\n(truncated, {} bytes total)",
            crate::app::truncate_str(output, 2000),
            output.len()
        )
    } else if output.trim().is_empty() {
        "(no output)".to_string()
    } else {
        output.to_string()
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
