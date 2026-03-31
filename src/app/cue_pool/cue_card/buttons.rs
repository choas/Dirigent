use eframe::egui;

use super::super::super::{icon, CueAction, DirigentApp};
use super::utils::{format_queue_label, toggle_reply_input};
use crate::db::{Cue, CueStatus};
use crate::settings::SourceKind;

impl DirigentApp {
    pub(in crate::app) fn render_status_buttons(
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
            .request_repaint_after(super::super::super::ELAPSED_REPAINT);
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
        if cue.plan_path.is_some() {
            self.render_plan_buttons(ui, cue, actions, fs);
        }
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
        if cue.plan_path.is_some() {
            self.render_plan_buttons(ui, cue, actions, fs);
        }
        self.render_notion_done_button(ui, cue, actions, fs);
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

    fn render_plan_buttons(
        &self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        fs: f32,
    ) {
        let run_btn =
            egui::Button::new(icon("\u{25B6} Run Plan", fs).color(self.semantic.badge_text))
                .fill(self.semantic.accent);
        if ui
            .add(run_btn)
            .on_hover_text("Execute the Claude Code plan")
            .clicked()
        {
            actions.push((cue.id, CueAction::RunPlan(cue.id)));
        }
        if ui
            .small_button(icon("View Plan", fs))
            .on_hover_text("Open the plan file in the code viewer")
            .clicked()
        {
            actions.push((cue.id, CueAction::ViewPlan(cue.id)));
        }
    }

    pub(in crate::app) fn render_push_button(
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

    pub(in crate::app) fn render_log_and_agents_buttons(
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

    pub(in crate::app) fn render_pr_notify_button(
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

    fn render_notion_done_button(
        &self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        fs: f32,
    ) {
        // Only show for cues sourced from a Notion integration.
        let is_notion_sourced = self.settings.sources.iter().any(|s| {
            s.kind == SourceKind::Notion
                && (s
                    .id
                    .as_ref()
                    .map_or(false, |sid| cue.source_id.as_deref() == Some(sid.as_str()))
                    || cue
                        .source_label
                        .as_ref()
                        .map_or(false, |label| *label == s.label))
        });
        if !is_notion_sourced {
            return;
        }

        // Check if already marked done in Notion (from cache, no DB hit).
        let already_done = self.notion_done_cache.contains(&cue.id);

        if already_done {
            ui.label(
                egui::RichText::new("\u{2713} Notion done")
                    .small()
                    .color(self.semantic.success),
            );
        } else {
            let btn =
                egui::Button::new(icon("\u{2713} Notion Done", fs).color(self.semantic.badge_text))
                    .fill(self.semantic.accent);
            if ui
                .add(btn)
                .on_hover_text("Mark this task as done in Notion")
                .clicked()
            {
                actions.push((cue.id, CueAction::NotionDone(cue.id)));
            }
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
}
