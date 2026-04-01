use std::collections::HashSet;
use std::time::Instant;

use super::super::{CueAction, DirigentApp};
use crate::db::{Cue, CueStatus};
use crate::diff_view::{self, DiffViewMode};
use crate::git;
use crate::settings::{CliProvider, SourceKind};

use super::helpers::{build_commit_all_subject, parse_schedule_duration};

impl DirigentApp {
    pub(in crate::app) fn process_cue_action(&mut self, id: i64, action: CueAction) {
        match action {
            CueAction::StartEdit(text) => {
                self.editing_cue = Some(super::super::EditingCue {
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
            CueAction::QueueFollowUp(cue_id, text) => {
                self.reply_inputs.remove(&cue_id);
                self.follow_up_queue.entry(cue_id).or_default().push(text);
                let count = self
                    .follow_up_queue
                    .get(&cue_id)
                    .map(|v| v.len())
                    .unwrap_or(0);
                let _ = self
                    .db
                    .log_activity(cue_id, &format!("Follow-up queued ({} pending)", count));
            }
            CueAction::ViewPlan(cue_id) => {
                self.process_view_plan(cue_id);
            }
            CueAction::RunPlan(cue_id) => {
                self.process_run_plan(cue_id);
            }
            CueAction::NotionDone(cue_id) => {
                self.process_notion_done(cue_id);
            }
        }
        self.reload_cues();
    }

    fn process_move_to(&mut self, id: i64, new_status: CueStatus) {
        if new_status != CueStatus::Ready {
            self.cancel_cue_task(id);
            self.follow_up_queue.remove(&id);
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
            // Clear any previous plan when starting a new run.
            let _ = self.db.update_cue_plan_path(id, None);
            self.claude.expand_running = true;
            self.reload_cues();
            self.trigger_claude(id);
        }
    }

    fn process_delete(&mut self, id: i64) {
        self.cancel_cue_task(id);
        self.run_queue.retain(|&cid| cid != id);
        self.follow_up_queue.remove(&id);
        self.scheduled_runs.remove(&id);
        self.schedule_inputs.remove(&id);
        let _ = self.db.delete_cue(id);
    }

    fn process_navigate(&mut self, file_path: &str, line: usize, _line_end: Option<usize>) {
        self.push_nav_history();
        let full_path = self.project_root.join(file_path);
        if self.viewer.current_file() != Some(&full_path) {
            self.load_file(full_path);
        } else {
            self.dismiss_central_overlays();
        }
        if let Some(tab) = self.viewer.active_mut() {
            tab.selection_start = None;
            tab.selection_end = None;
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
        self.diff_review = Some(super::super::DiffReview {
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
                    // Intentional: if there is nothing to commit the work is
                    // already on disk (e.g. committed in an earlier step or by
                    // the AI itself).  Moving to Done is correct — do not leave
                    // the cue stuck in Review.
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
            let when = std::time::SystemTime::now() + duration;
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

    fn process_view_plan(&mut self, cue_id: i64) {
        let plan_path = self
            .cues
            .iter()
            .find(|c| c.id == cue_id)
            .and_then(|c| c.plan_path.clone());
        if let Some(path) = plan_path {
            let full_path = std::path::PathBuf::from(&path);
            if full_path.exists() {
                self.push_nav_history();
                self.load_file(full_path);
            } else {
                self.set_status_message(format!("Plan file not found: {}", path));
            }
        }
    }

    fn process_run_plan(&mut self, cue_id: i64) {
        let plan_path = self
            .cues
            .iter()
            .find(|c| c.id == cue_id)
            .and_then(|c| c.plan_path.clone());
        if let Some(path) = plan_path {
            let plan_content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    self.set_status_message(format!("Failed to read plan: {}", e));
                    return;
                }
            };
            let reply = format!(
                "Execute the following plan:\n\n{}\n\nPlan file: {}",
                plan_content, path
            );
            let _ = self.db.log_activity(cue_id, "Running plan");
            self.trigger_claude_reply(cue_id, &reply, &[]);
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

    fn process_notion_done(&mut self, cue_id: i64) {
        if self.notion_done_in_progress {
            self.set_status_message("Notion update already in progress".into());
            return;
        }

        let cue = match self.cues.iter().find(|c| c.id == cue_id) {
            Some(c) => c.clone(),
            None => return,
        };
        let source_ref = match &cue.source_ref {
            Some(r) => r.clone(),
            None => {
                self.set_status_message("No Notion page reference on this cue".into());
                return;
            }
        };

        // Find the matching Notion source config to get token and page type.
        // If cue.source_id is present, match only by source id; otherwise fall back to label.
        let notion_source = if cue.source_id.is_some() {
            self.settings
                .sources
                .iter()
                .find(|s| {
                    s.kind == SourceKind::Notion
                        && s.id
                            .as_ref()
                            .map_or(false, |sid| cue.source_id.as_deref() == Some(sid.as_str()))
                })
                .cloned()
        } else {
            self.settings
                .sources
                .iter()
                .find(|s| {
                    s.kind == SourceKind::Notion && cue.source_label.as_deref() == Some(&s.label)
                })
                .cloned()
        };
        let Some(source) = notion_source else {
            self.set_status_message("Notion source config not found for this cue".into());
            return;
        };

        let token = crate::sources::resolve_source_token(&source, &self.project_root);

        let page_ref = source_ref.clone();
        let page_type = source.notion_page_type.clone();
        let done_value = source.notion_done_value.clone();
        let status_property = source.notion_status_property.clone();

        self.notion_done_in_progress = true;
        let (tx, rx) = std::sync::mpsc::channel();
        self.notion_done_rx = Some(rx);

        std::thread::spawn(move || {
            let result = crate::sources::mark_notion_done(
                &token,
                &page_ref,
                &page_type,
                &done_value,
                &status_property,
            )
            .map(|()| cue_id)
            .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });

        self.set_status_message("Updating Notion...".into());
    }

    pub(in crate::app) fn process_notion_done_result(&mut self) {
        let rx = match self.notion_done_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.notion_done_in_progress = false;
                self.notion_done_rx = None;
                self.set_status_message("Notion update failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.notion_done_in_progress = false;
        self.notion_done_rx = None;
        match result {
            Ok(cue_id) => {
                self.set_status_message("Marked done in Notion".into());
                let _ = self.db.log_activity(cue_id, "Marked done in Notion");
                self.notion_done_cache.insert(cue_id);
            }
            Err(e) => {
                self.set_status_message(format!("Notion done failed: {}", e));
            }
        }
    }
}
