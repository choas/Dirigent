use std::collections::HashSet;
use std::time::Instant;

use super::super::{CueAction, DirigentApp};
use crate::claude;
use crate::db::{Cue, CueStatus};
use crate::diff_view::{self, DiffViewMode};
use crate::git;
use crate::settings::{SourceKind, VcsBackend};
use crate::telemetry;

use super::super::vcs_dispatch;
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
                if let Err(e) = self.db.update_cue_text(id, &new_text) {
                    self.set_status_message(format!("Failed to save edit: {}", e));
                } else {
                    let _ = self.db.log_activity(id, "Edited");
                }
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
            CueAction::MarkReviewDone(cue_id) => {
                // Clear any pending question first: a run can change files
                // (moving the cue to Review) and still end by asking the user
                // something. on_workflow_cue_completed holds back a cue with an
                // unanswered question, so accepting it via Done without clearing
                // the flag would leave the workflow step permanently blocked.
                let was_blocked = self.cues.iter().any(|c| c.id == cue_id && c.has_question);
                // The DB flag must be cleared before moving to Done: if this
                // write fails and we proceed anyway, on_workflow_cue_completed
                // would still see has_question and block the step permanently.
                // Abort the move and surface the error so the state stays
                // consistent and the user can retry.
                if let Err(e) = self.db.update_cue_has_question(cue_id, false) {
                    self.set_status_message(format!("Failed to clear question flag: {}", e));
                    return;
                }
                self.process_move_to(cue_id, CueStatus::Done);
                if was_blocked && self.workflow_plan.is_some() {
                    // process_move_to only reloads cues when moving to Ready,
                    // so refresh in-memory state before re-checking the step.
                    self.reload_cues();
                    self.on_workflow_cue_completed(cue_id);
                }
            }
            CueAction::ReplyReview(cue_id, reply_text) => {
                self.reply_inputs.remove(&cue_id);
                let _ = self.db.update_cue_has_question(cue_id, false);
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
            CueAction::CreateWorkflow => {
                self.create_workflow();
            }
            CueAction::CancelWorkflow => {
                self.cancel_workflow();
            }
            CueAction::StartWorkflow => {
                self.start_workflow();
            }
            CueAction::ResumeWorkflow => {
                self.resume_workflow();
            }
            CueAction::TogglePause(step_idx) => {
                self.toggle_workflow_pause(step_idx);
            }
            CueAction::RemoveFromWorkflow(cue_id) => {
                self.remove_from_workflow(cue_id);
            }
            CueAction::ArchiveAllDone => {
                self.process_archive_all_done();
            }
            CueAction::DeleteAllArchived => {
                self.process_delete_all_archived();
            }
            CueAction::SplitCue => {
                self.start_split_cue(id);
            }
            CueAction::SquashBookmark => {
                self.process_squash_bookmark(id);
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
        if new_status == CueStatus::Archived {
            self.conversation_replies.remove(&id);
            self.conversation_reply_images.remove(&id);
        }
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
        self.last_follow_up.remove(&id);
        self.scheduled_runs.remove(&id);
        self.schedule_inputs.remove(&id);
        self.reply_inputs.remove(&id);
        self.cue_move_flash.remove(&id);
        self.latest_exec_cache.remove(&id);
        self.cue_warnings.remove(&id);
        self.tag_inputs.remove(&id);
        self.claude.running_logs.remove(&id);
        self.claude.exec_ids.remove(&id);
        self.claude.start_times.remove(&id);
        self.claude.log_heartbeats.remove(&id);
        self.claude.last_message_times.remove(&id);
        self.notion_done_cache.remove(&id);
        self.conversation_replies.remove(&id);
        self.conversation_reply_images.remove(&id);
        let _ = std::fs::remove_file(
            self.project_root
                .join(".Dirigent")
                .join(format!("split-ref-{}.md", id)),
        );
        if let Err(e) = self.db.delete_cue(id) {
            self.set_status_message(format!("Failed to delete cue: {}", e));
        }
    }

    fn process_navigate(&mut self, file_path: &str, line: usize, _line_end: Option<usize>) {
        self.push_nav_history();
        let full_path = self.project_root.join(file_path);
        let rel = std::path::Path::new(file_path);
        if rel.is_absolute()
            || rel.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::Prefix(_)
                )
            })
        {
            self.set_status_message(format!("Cannot navigate: invalid path \"{}\"", file_path));
            return;
        }
        if let (Ok(canon_root), Ok(canon_path)) = (
            std::fs::canonicalize(&self.project_root),
            std::fs::canonicalize(&full_path),
        ) {
            if !canon_path.starts_with(&canon_root) {
                self.set_status_message("Cannot navigate: path is outside the project".into());
                return;
            }
        }
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
        let exec = match self.db.get_latest_execution(cue_id) {
            Ok(Some(e)) => e,
            Ok(None) => {
                self.set_status_message("No execution found for this cue".into());
                return;
            }
            Err(e) => {
                self.set_status_message(format!("Failed to load execution: {}", e));
                return;
            }
        };
        let Some(diff) = exec.diff else {
            self.set_status_message("No diff available for this execution".into());
            return;
        };
        let cue = self.cues.iter().find(|c| c.id == cue_id);
        let text = cue.map(|c| c.text.clone()).unwrap_or_default();
        let read_only = cue.map(|c| c.status != CueStatus::Review).unwrap_or(true);
        let parsed = diff_view::parse_unified_diff(&diff);
        self.dismiss_central_overlays();
        self.diff_review = Some(super::super::DiffReview {
            cue_id,
            diff,
            cue_text: text,
            commit_hash: None,
            commit_author: None,
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

    pub(in crate::app) fn process_commit_review(&mut self, cue_id: i64) {
        self.process_commit_review_inner(cue_id, true);
    }

    pub(in crate::app) fn process_commit_review_auto(&mut self, cue_id: i64) {
        self.process_commit_review_inner(cue_id, false);
    }

    fn process_commit_review_inner(&mut self, cue_id: i64, show_dialog: bool) {
        // For jj workspace cues the commit+bookmark was already done in
        // handle_run_with_diff. Just transition to Done and clean up.
        // Skip the fast-path if the workspace commit previously failed.
        if self.claude.workspace_paths.contains_key(&cue_id)
            && !self.claude.workspace_commit_failed.contains(&cue_id)
        {
            self.set_status_message("Accepted — changes committed in workspace".to_string());
            let _ = self.db.update_cue_status(cue_id, CueStatus::Done);
            let _ = self.db.log_activity(cue_id, "Accepted (workspace commit)");
            self.cleanup_jj_workspace(cue_id);
            self.clear_review_question_and_recheck_workflow(cue_id);
            self.reload_git_info();
            self.reload_commit_history();
            return;
        }

        match self.db.get_latest_execution(cue_id) {
            Ok(Some(exec)) => {
                if exec.diff.is_some() {
                    // Prefer the most recent follow-up prompt as the commit
                    // message basis — it reflects the latest instruction that
                    // produced these changes. Fall back to the original cue text.
                    let cue_text = self
                        .last_follow_up
                        .get(&cue_id)
                        .filter(|t| !t.trim().is_empty())
                        .cloned()
                        .unwrap_or_else(|| {
                            self.cues
                                .iter()
                                .find(|c| c.id == cue_id)
                                .map(|c| c.text.clone())
                                .unwrap_or_default()
                        });
                    let extracted = exec
                        .response
                        .as_deref()
                        .and_then(claude::extract_commit_message);
                    let commit_msg = git::generate_commit_message(&cue_text, extracted.as_deref());

                    if show_dialog && self.settings.vcs_backend == VcsBackend::Jj {
                        self.git.commit_message_input = commit_msg;
                        self.git.commit_review_cue_id = Some(cue_id);
                        self.git.commit_needs_focus = true;
                        self.git.show_commit_dialog = true;
                    } else {
                        self.apply_commit_result(
                            cue_id,
                            vcs_dispatch::commit_diff(
                                &self.settings.vcs_backend,
                                &self.settings.jj_cli_path,
                                &self.project_root,
                                exec.diff.as_deref().unwrap(),
                                &commit_msg,
                                self.git.active_bookmark.as_deref(),
                            ),
                        );
                        self.reload_git_info();
                        self.reload_commit_history();
                    }
                } else {
                    self.set_status_message("Nothing to commit — no diff in execution".into());
                }
            }
            Ok(None) => {
                self.set_status_message("Nothing to commit — no execution found".into());
            }
            Err(e) => {
                self.set_status_message(format!("Commit failed: {}", e));
            }
        }
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
                self.clear_review_question_and_recheck_workflow(cue_id);
                let dirty_count = vcs_dispatch::get_dirty_files(
                    &self.settings.vcs_backend,
                    &self.settings.jj_cli_path,
                    &self.project_root,
                )
                .len();
                telemetry::emit_git_commit(&self.project_name(), dirty_count);
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
                    self.clear_review_question_and_recheck_workflow(cue_id);
                } else {
                    self.set_status_message(format!("Commit failed: {}", e));
                }
            }
        }
    }

    /// After a review-commit has moved a cue to Done, clear any pending
    /// question and re-check the owning workflow step. A run can change files
    /// (moving the cue to Review) and still end by asking the user something;
    /// `on_workflow_cue_completed` holds back a cue with an unanswered question.
    /// Committing the changed files without clearing the flag would otherwise
    /// leave the workflow step permanently blocked even though the cue is no
    /// longer actionable. Call this *after* the cue has reached Done.
    pub(in crate::app) fn clear_review_question_and_recheck_workflow(&mut self, cue_id: i64) {
        let was_blocked = self.cues.iter().any(|c| c.id == cue_id && c.has_question);
        if !was_blocked {
            return;
        }
        // If clearing the flag fails, abort: proceeding would leave the DB
        // still flagged while in-memory state is recomputed, so the workflow
        // step could be (re)blocked inconsistently. Surface the error instead.
        if let Err(e) = self.db.update_cue_has_question(cue_id, false) {
            self.set_status_message(format!("Failed to clear question flag: {}", e));
            return;
        }
        if self.workflow_plan.is_some() {
            // Refresh in-memory state so the re-check sees the cleared flag and
            // the Done status before deciding whether the step is complete.
            self.reload_cues();
            self.on_workflow_cue_completed(cue_id);
        }
    }

    fn process_revert_review(&mut self, cue_id: i64) {
        // For jj workspace cues: delete the bookmark and forget the workspace.
        // The commit stays in the repo (hidden) but is no longer referenced.
        if self.claude.workspace_paths.contains_key(&cue_id) {
            let cue_text = self
                .cues
                .iter()
                .find(|c| c.id == cue_id)
                .map(|c| c.text.clone())
                .unwrap_or_default();
            let bookmark = crate::jj::cue_bookmark_name(cue_id, &cue_text);
            match crate::jj::jj_delete_bookmark(
                &self.project_root,
                &bookmark,
                &self.settings.jj_cli_path,
            ) {
                Ok(()) => {
                    self.cleanup_jj_workspace(cue_id);
                    let _ = self.db.update_cue_status(cue_id, CueStatus::Inbox);
                    let _ = self.db.log_activity(cue_id, "Reverted (bookmark deleted)");
                    self.set_status_message(
                        "Reverted — bookmark deleted, workspace removed".to_string(),
                    );
                    self.reload_git_info();
                }
                Err(e) => {
                    let msg = e.to_string().to_lowercase();
                    if msg.contains("no such bookmark") || msg.contains("not found") {
                        self.cleanup_jj_workspace(cue_id);
                        let _ = self.db.update_cue_status(cue_id, CueStatus::Inbox);
                        let _ = self.db.log_activity(
                            cue_id,
                            "Reverted (bookmark not found, cleaned workspace)",
                        );
                        self.set_status_message(
                            "Reverted — bookmark already gone, workspace removed".to_string(),
                        );
                        self.reload_git_info();
                    } else {
                        let _ = self.db.log_activity(
                            cue_id,
                            &format!("Revert failed: jj bookmark delete error: {e}"),
                        );
                        self.set_status_message(format!("Revert failed: {e}"));
                    }
                }
            }
            return;
        }

        let reverted = match self.db.get_latest_execution(cue_id) {
            Ok(Some(exec)) => {
                if let Some(ref diff) = exec.diff {
                    let file_paths = git::parse_diff_file_paths_for_repo(&self.project_root, diff);
                    match vcs_dispatch::revert_files(
                        &self.settings.vcs_backend,
                        &self.settings.jj_cli_path,
                        &self.project_root,
                        &file_paths,
                    ) {
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
        self.dismiss_central_overlays();
        match self.db.get_all_executions(cue_id) {
            Ok(execs) => {
                if let std::collections::hash_map::Entry::Vacant(e) =
                    self.claude.running_logs.entry(cue_id)
                {
                    if let Some(last) = execs.last() {
                        if let Some(ref log_text) = last.log {
                            e.insert((log_text.clone(), last.provider.clone()));
                        }
                    }
                }
                self.claude.conversation_history = execs;
            }
            Err(e) => {
                self.set_status_message(format!("Failed to load execution log: {}", e));
                return;
            }
        }
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
        let review_ids: Vec<i64> = review_cues.iter().map(|c| c.id).collect();

        // Handle workspace cues individually via the same path as
        // process_commit_review so cleanup_jj_workspace is invoked.
        let mut workspace_accepted = 0usize;
        let mut non_workspace_ids: Vec<i64> = Vec::new();
        for &cue_id in &review_ids {
            if self.claude.workspace_paths.contains_key(&cue_id)
                && !self.claude.workspace_commit_failed.contains(&cue_id)
            {
                let _ = self.db.update_cue_status(cue_id, CueStatus::Done);
                let _ = self.db.log_activity(cue_id, "Accepted (workspace commit)");
                self.cleanup_jj_workspace(cue_id);
                workspace_accepted += 1;
            } else {
                non_workspace_ids.push(cue_id);
            }
        }

        // Commit remaining non-workspace cues in bulk.
        if !non_workspace_ids.is_empty() {
            let non_ws_cues: Vec<&Cue> = self
                .cues
                .iter()
                .filter(|c| non_workspace_ids.contains(&c.id))
                .collect();
            let subject = build_commit_all_subject(&non_ws_cues);
            let cue_details: Vec<String> = non_ws_cues
                .iter()
                .map(|c| format!("- {}", c.text.trim()))
                .collect();
            let commit_msg = format!(
                "{}\n\n{}\n\n{}",
                subject,
                cue_details.join("\n\n"),
                git::DIRIGENT_FOOTER,
            );
            match vcs_dispatch::commit_all(
                &self.settings.vcs_backend,
                &self.settings.jj_cli_path,
                &self.project_root,
                &commit_msg,
                self.git.active_bookmark.as_deref(),
            ) {
                Ok(hash) => {
                    let short = &hash[..7.min(hash.len())];
                    let total = workspace_accepted + non_workspace_ids.len();
                    let plural = if total == 1 { "" } else { "s" };
                    self.set_status_message(format!(
                        "Committed all: {} ({} cue{})",
                        short, total, plural,
                    ));
                    for cue_id in &non_workspace_ids {
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
        } else if workspace_accepted > 0 {
            let plural = if workspace_accepted == 1 { "" } else { "s" };
            self.set_status_message(format!(
                "Accepted {} workspace cue{}",
                workspace_accepted, plural,
            ));
        }

        self.reload_git_info();
        self.reload_commit_history();
    }

    fn process_queue_next(&mut self, id: i64) {
        if self.run_queue.contains(&id) {
            self.set_status_message("Cue is already queued".into());
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
        let Some(path) = plan_path else {
            self.set_status_message("No plan available for this cue".into());
            return;
        };
        let full_path = std::path::PathBuf::from(&path);
        if !self.is_within_project_root(&full_path) && !Self::is_valid_plan_path(&full_path) {
            self.set_status_message("Plan file is outside allowed directories".to_string());
            return;
        }
        if full_path.exists() {
            self.push_nav_history();
            self.load_file(full_path);
        } else {
            self.set_status_message(format!("Plan file not found: {}", path));
        }
    }

    fn process_run_plan(&mut self, cue_id: i64) {
        let plan_path = self
            .cues
            .iter()
            .find(|c| c.id == cue_id)
            .and_then(|c| c.plan_path.clone());
        if let Some(path) = plan_path {
            let full_path = std::path::PathBuf::from(&path);
            if !self.is_within_project_root(&full_path) && !Self::is_valid_plan_path(&full_path) {
                self.set_status_message("Plan file is outside allowed directories".to_string());
                return;
            }
            let plan_content = match std::fs::read_to_string(&full_path) {
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
        } else {
            self.set_status_message("No plan available for this cue".into());
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

    fn process_archive_all_done(&mut self) {
        match self.db.archive_all_done() {
            Ok(ids) if ids.is_empty() => {
                self.set_status_message("No cues in Done".into());
            }
            Ok(ids) => {
                for cue_id in &ids {
                    self.conversation_replies.remove(cue_id);
                    self.conversation_reply_images.remove(cue_id);
                }
                let plural = if ids.len() == 1 { "" } else { "s" };
                self.set_status_message(format!("Archived {} Done cue{}", ids.len(), plural));
            }
            Err(e) => {
                self.set_status_message(format!("Failed to archive Done cues: {e}"));
            }
        }
    }

    fn process_delete_all_archived(&mut self) {
        if self.archived_cue_count == 0 {
            self.set_status_message("No cues in Archived".into());
            return;
        }
        let total = self.archived_cue_count;
        if let Err(e) = self.db.delete_all_archived() {
            self.set_status_message(format!("Failed to delete archived cues: {e}"));
            return;
        }
        let archived_ids: Vec<i64> = self
            .cues
            .iter()
            .filter(|c| c.status == CueStatus::Archived)
            .map(|c| c.id)
            .collect();
        for cue_id in &archived_ids {
            self.latest_exec_cache.remove(cue_id);
            self.cue_warnings.remove(cue_id);
            self.notion_done_cache.remove(cue_id);
            self.conversation_replies.remove(cue_id);
            self.conversation_reply_images.remove(cue_id);
            let path = self
                .project_root
                .join(".Dirigent")
                .join(format!("split-ref-{}.md", cue_id));
            if let Err(e) = std::fs::remove_file(&path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::error!("Failed to remove {}: {e}", path.display());
                }
            }
        }
        let plural = if total == 1 { "" } else { "s" };
        self.set_status_message(format!("Deleted {} archived cue{}", total, plural));
    }

    fn process_squash_bookmark(&mut self, cue_id: i64) {
        use crate::settings::VcsBackend;
        if self.settings.vcs_backend != VcsBackend::Jj {
            self.set_status_message("Squash is only available with the jj backend".into());
            return;
        }
        let cue_text = self
            .cues
            .iter()
            .find(|c| c.id == cue_id)
            .map(|c| c.text.clone())
            .unwrap_or_default();
        let bookmark = crate::jj::cue_bookmark_name(cue_id, &cue_text);
        let jj_path = self.settings.jj_cli_path.clone();

        match crate::jj::jj_squash_bookmark(&self.project_root, &bookmark, &jj_path) {
            Ok(0) => {
                self.set_status_message(format!(
                    "Nothing to squash — bookmark \"{}\" has 0 or 1 commits",
                    bookmark
                ));
            }
            Ok(n) => {
                let plural = if n == 1 { "" } else { "s" };
                self.set_status_message(format!(
                    "Squashed {} commit{} on \"{}\" into one",
                    n, plural, bookmark
                ));
                let _ = self.db.log_activity(
                    cue_id,
                    &format!("Squashed {} commit{} on {}", n, plural, bookmark),
                );
            }
            Err(e) => {
                self.set_status_message(format!("Squash failed: {}", e));
            }
        }
        self.reload_git_info();
        self.reload_commit_history();
    }

    fn process_notion_done(&mut self, cue_id: i64) {
        if self.notion_done_in_progress {
            self.set_status_message("Notion update already in progress".into());
            return;
        }

        let cue = match self.cues.iter().find(|c| c.id == cue_id) {
            Some(c) => c.clone(),
            None => {
                self.set_status_message("Cue not found".into());
                return;
            }
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
                            .is_some_and(|sid| cue.source_id.as_deref() == Some(sid.as_str()))
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
