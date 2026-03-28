use std::sync::mpsc;
use std::time::Instant;

use crate::db::CueStatus;
use crate::git;

use super::{detect_pr_number_from_branch, DirigentApp};

impl DirigentApp {
    /// Start an async git push operation.
    pub(super) fn start_git_push(&mut self) {
        if self.git.pushing {
            return;
        }
        self.git.pushing = true;
        let (tx, rx) = mpsc::channel();
        self.git.push_rx = Some(rx);
        let root = self.project_root.clone();
        std::thread::spawn(move || {
            let result = git::git_push(&root).map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.set_status_message("Pushing...".to_string());
    }

    /// Check for completed git push.
    pub(super) fn process_push_result(&mut self) {
        let rx = match self.git.push_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.git.pushing = false;
                self.git.push_rx = None;
                self.set_status_message("Git push failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.git.pushing = false;
        self.git.push_rx = None;
        match result {
            Ok(msg) => {
                self.set_status_message(msg);
                self.reload_git_info();
                self.reload_commit_history();
            }
            Err(e) => {
                self.set_status_message(format!("Push failed: {}", e));
            }
        }
    }

    /// Start an async git pull operation.
    pub(super) fn start_git_pull(&mut self) {
        self.start_git_pull_with_strategy(git::PullStrategy::FfOnly);
    }

    /// Start an async git pull with a specific strategy.
    pub(super) fn start_git_pull_with_strategy(&mut self, strategy: git::PullStrategy) {
        if self.git.pulling {
            return;
        }
        self.git.pulling = true;
        let (tx, rx) = mpsc::channel();
        self.git.pull_rx = Some(rx);
        let root = self.project_root.clone();
        std::thread::spawn(move || {
            let result = git::git_pull(&root, strategy).map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        let label = match strategy {
            git::PullStrategy::FfOnly => "Pulling...",
            git::PullStrategy::Merge => "Pulling (merge)...",
            git::PullStrategy::Rebase => "Pulling (rebase)...",
        };
        self.set_status_message(label.to_string());
    }

    /// Open the Create PR dialog with pre-filled fields.
    pub(super) fn open_create_pr_dialog(&mut self) {
        let branch = self
            .git
            .info
            .as_ref()
            .map(|i| i.branch.clone())
            .unwrap_or_default();
        let base = git::get_default_branch(&self.project_root);
        let body = git::build_pr_body(&self.project_root, &base);
        // Use branch name as default title (replace hyphens/underscores with spaces)
        let title = branch.replace(['-', '_'], " ");
        self.git.pr_title = title;
        self.git.pr_body = body;
        self.git.pr_base = base;
        self.git.pr_draft = false;
        self.git.show_create_pr = true;
    }

    /// Start an async PR creation (pushes first, then creates the PR).
    pub(super) fn start_create_pr(&mut self) {
        if self.git.creating_pr {
            return;
        }
        self.git.creating_pr = true;
        self.git.show_create_pr = false;
        let (tx, rx) = mpsc::channel();
        self.git.pr_rx = Some(rx);
        let root = self.project_root.clone();
        let title = self.git.pr_title.clone();
        let body = self.git.pr_body.clone();
        let base = self.git.pr_base.clone();
        let draft = self.git.pr_draft;
        std::thread::spawn(move || {
            // Push first so the remote branch exists
            if let Err(e) = git::git_push(&root) {
                let _ = tx.send(Err(format!("Push failed: {}", e)));
                return;
            }
            let result = git::create_pull_request(&root, &title, &body, &base, draft)
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.set_status_message("Pushing & creating PR...".to_string());
    }

    /// Check for completed PR creation.
    pub(super) fn process_pr_result(&mut self) {
        let rx = match self.git.pr_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.git.creating_pr = false;
                self.git.pr_rx = None;
                self.set_status_message("PR creation failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.git.creating_pr = false;
        self.git.pr_rx = None;
        match result {
            Ok(url) => {
                self.set_status_message(format!("PR created: {}", url));
                self.reload_git_info();
                self.reload_commit_history();
            }
            Err(e) => {
                self.set_status_message(format!("PR failed: {}", e));
            }
        }
    }

    pub(super) fn open_import_pr_dialog(&mut self) {
        self.git.show_import_pr = true;
        // Pre-fill with current branch PR number if available
        if self.git.import_pr_number.is_empty() {
            // Try to detect PR number from current branch
            if let Some(ref info) = self.git.info {
                if let Some(num) = detect_pr_number_from_branch(&self.project_root, &info.branch) {
                    self.git.import_pr_number = num.to_string();
                }
            }
        }
    }

    pub(super) fn start_import_pr_findings(&mut self) {
        if self.git.importing_pr {
            self.set_status_message("PR import already in progress".to_string());
            return;
        }

        let pr_number: u32 = match self.git.import_pr_number.trim().parse() {
            Ok(n) if n > 0 => n,
            _ => {
                self.set_status_message("Invalid PR number".to_string());
                return;
            }
        };

        self.git.importing_pr = true;
        self.git.importing_pr_start = Some(Instant::now());
        self.git.show_import_pr = false;
        self.set_status_message(format!("Refreshing PR #{}…", pr_number));
        let project_root = self.project_root.clone();
        let (tx, rx) = mpsc::channel();
        self.git.import_pr_rx = Some(rx);

        std::thread::spawn(move || {
            let result = crate::sources::fetch_pr_findings(&project_root, pr_number)
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
    }

    pub(super) fn process_import_pr_result(&mut self) {
        let rx = match self.git.import_pr_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Update status with elapsed time so user knows it's still running
                if let Some(start) = self.git.importing_pr_start {
                    let secs = start.elapsed().as_secs();
                    let pr = self.git.import_pr_number.trim();
                    self.set_status_message(format!("Refreshing PR #{}… ({}s)", pr, secs));
                }
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // Thread panicked or was dropped
                self.git.importing_pr = false;
                self.git.importing_pr_start = None;
                self.git.import_pr_rx = None;
                self.set_status_message("PR import failed unexpectedly".into());
                return;
            }
            Ok(result) => result,
        };
        self.git.importing_pr = false;
        self.git.importing_pr_start = None;
        self.git.import_pr_rx = None;
        match result {
            Ok(findings) => {
                if findings.is_empty() {
                    self.set_status_message("No actionable findings found in PR".to_string());
                } else {
                    self.set_status_message(format!(
                        "Fetched {} findings – review and filter before importing",
                        findings.len()
                    ));
                    self.git.pr_findings_pending = findings;
                    self.git.pr_findings_excluded.clear();
                    self.git.show_pr_filter = true;
                }
            }
            Err(e) => self.set_status_message(format!("PR import failed: {}", e)),
        }
    }

    /// Process successfully fetched PR findings: upsert cues and report results.
    pub(super) fn handle_pr_findings(&mut self, findings: Vec<crate::sources::PrFinding>) {
        if findings.is_empty() {
            self.set_status_message("No actionable findings found in PR".to_string());
            return;
        }
        let pr_number = self.git.import_pr_number.trim().to_string();
        let tag = format!("PR{}", pr_number);
        let (new_count, updated_count, error_count) = self.upsert_pr_findings(&findings, &tag);
        let has_changes = new_count > 0 || updated_count > 0;
        self.reload_cues();
        if error_count > 0 && !has_changes {
            self.set_status_message(format!(
                "PR #{}: import failed ({} DB errors across {} findings)",
                pr_number,
                error_count,
                findings.len()
            ));
        } else if error_count > 0 {
            let summary = Self::build_findings_summary(new_count, updated_count);
            self.set_status_message(format!(
                "PR #{}: {} (tag: {}) (partial failure: {} DB errors)",
                pr_number, summary, tag, error_count
            ));
        } else if has_changes {
            let summary = Self::build_findings_summary(new_count, updated_count);
            self.set_status_message(format!("PR #{}: {} (tag: {})", pr_number, summary, tag));
        } else {
            self.set_status_message(format!(
                "PR #{}: all {} findings already imported",
                pr_number,
                findings.len()
            ));
        }
    }

    /// Upsert each PR finding: update existing cues or insert new ones.
    /// Returns (new_count, updated_count, error_count).
    fn upsert_pr_findings(
        &mut self,
        findings: &[crate::sources::PrFinding],
        tag: &str,
    ) -> (usize, usize, usize) {
        let mut new_count = 0;
        let mut updated_count = 0;
        let mut error_count = 0;
        for finding in findings {
            match self.db.get_cue_by_source_ref(&finding.external_id) {
                Ok(Some((
                    existing_id,
                    existing_text,
                    _existing_status,
                    existing_path,
                    existing_line,
                ))) => {
                    match self.update_existing_finding(
                        finding,
                        existing_id,
                        &existing_text,
                        &existing_path,
                        existing_line,
                    ) {
                        Ok(Some("updated")) => updated_count += 1,
                        Ok(_) => {}
                        Err(e) => {
                            eprintln!("DB error updating finding {}: {e}", finding.external_id);
                            error_count += 1;
                        }
                    }
                    continue;
                }
                Ok(None) => {} // New finding — fall through to insert
                Err(e) => {
                    eprintln!(
                        "DB error looking up source ref {}: {e}",
                        finding.external_id
                    );
                    error_count += 1;
                    continue;
                }
            }
            match self.db.insert_cue_from_source(
                &finding.text,
                "PR Review",
                &finding.external_id,
                &finding.file_path,
                finding.line_number,
            ) {
                Ok(id) => {
                    if let Err(e) = self.db.update_cue_tag(id, Some(tag)) {
                        eprintln!("DB error tagging new cue {id}: {e}");
                        error_count += 1;
                    } else {
                        new_count += 1;
                    }
                }
                Err(e) => {
                    eprintln!("DB error inserting finding {}: {e}", finding.external_id);
                    error_count += 1;
                }
            }
        }
        (new_count, updated_count, error_count)
    }

    /// Handle a single existing finding: update text/location if changed.
    /// Returns `Ok(Some("updated"))` when the cue was updated and reset to Inbox,
    /// `Ok(None)` when no change was needed, or `Err` if a DB write failed.
    ///
    /// Done/Archived cues are left alone when their text and location match —
    /// only actual content changes from the PR reviewer warrant a re-open.
    fn update_existing_finding(
        &mut self,
        finding: &crate::sources::PrFinding,
        existing_id: i64,
        existing_text: &str,
        existing_path: &str,
        existing_line: usize,
    ) -> anyhow::Result<Option<&'static str>> {
        let clean_existing = crate::sources::strip_html_tags(existing_text);
        let clean_finding = crate::sources::strip_html_tags(&finding.text);
        let text_changed = crate::sources::strip_pr_context_hint(&clean_existing)
            != crate::sources::strip_pr_context_hint(&clean_finding);
        let location_changed =
            existing_path != finding.file_path || existing_line != finding.line_number;
        if text_changed || location_changed {
            // Text or location changed: update and reset to Inbox
            self.db.update_cue_by_source_ref(
                &finding.external_id,
                &finding.text,
                &finding.file_path,
                finding.line_number,
            )?;
            self.db.update_cue_status(existing_id, CueStatus::Inbox)?;
            let _ = self
                .db
                .log_activity(existing_id, "PR comment updated, reset to Inbox");
            Ok(Some("updated"))
        } else {
            Ok(None)
        }
    }

    /// Build a human-readable summary of finding counts.
    fn build_findings_summary(new_count: usize, updated_count: usize) -> String {
        let mut parts = Vec::new();
        if new_count > 0 {
            parts.push(format!("{} new", new_count));
        }
        if updated_count > 0 {
            parts.push(format!("{} updated", updated_count));
        }
        format!("{} finding(s)", parts.join(", "))
    }

    /// Notify a single PR comment that a finding was fixed.
    pub(super) fn start_notify_pr_single(&mut self, cue_id: i64) {
        if self.git.notifying_pr {
            self.set_status_message("PR notification already in progress".to_string());
            return;
        }
        // Look up the cue's source_ref and commit hash
        let cue = match self.cues.iter().find(|c| c.id == cue_id) {
            Some(c) => c.clone(),
            None => return,
        };
        let source_ref = match cue.source_ref {
            Some(ref s) if s.starts_with("pr") => s.clone(),
            _ => {
                self.set_status_message("Cue has no PR source reference".to_string());
                return;
            }
        };
        // Extract commit hash from activity log
        let commit_hash = self
            .db
            .get_last_activity_matching(cue_id, "Committed")
            .ok()
            .flatten()
            .and_then(|event| {
                // Activity format: "Committed (abc1234)"
                event
                    .strip_prefix("Committed (")
                    .and_then(|s| s.strip_suffix(')'))
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "latest commit".to_string());

        self.git.notifying_pr = true;
        let project_root = self.project_root.clone();
        let (tx, rx) = mpsc::channel();
        self.git.pr_notify_rx = Some(rx);

        std::thread::spawn(move || {
            let result =
                crate::sources::notify_pr_finding_fixed(&project_root, &source_ref, &commit_hash);
            let _ = tx.send(match result {
                Ok(true) => Ok(format!("Notified PR comment for cue #{}", cue_id)),
                Ok(false) => Err("Could not parse PR source reference".to_string()),
                Err(e) => Err(e.to_string()),
            });
        });
    }

    /// Push and notify all Done PR-sourced cues.
    pub(super) fn start_push_and_notify_pr(&mut self) {
        if self.git.notifying_pr {
            self.set_status_message("PR notification already in progress".to_string());
            return;
        }

        // Collect all Done cues with PR source_ref
        let pr_cues: Vec<(i64, String, String)> = self
            .cues
            .iter()
            .filter(|c| c.status == CueStatus::Done)
            .filter_map(|c| {
                let source_ref = c.source_ref.as_ref()?;
                if !source_ref.starts_with("pr") {
                    return None;
                }
                // Check if already notified
                let already_notified = self
                    .db
                    .get_last_activity_matching(c.id, "Notified PR")
                    .ok()
                    .flatten()
                    .is_some();
                if already_notified {
                    return None;
                }
                let commit_hash = self
                    .db
                    .get_last_activity_matching(c.id, "Committed")
                    .ok()
                    .flatten()
                    .and_then(|event| {
                        event
                            .strip_prefix("Committed (")
                            .and_then(|s| s.strip_suffix(')'))
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_else(|| "latest commit".to_string());
                Some((c.id, source_ref.clone(), commit_hash))
            })
            .collect();

        if pr_cues.is_empty() {
            self.set_status_message("No un-notified PR findings in Done".to_string());
            return;
        }

        self.git.notifying_pr = true;
        let project_root = self.project_root.clone();
        let (tx, rx) = mpsc::channel();
        self.git.pr_notify_rx = Some(rx);

        std::thread::spawn(move || {
            // First push
            let push_result = crate::git::git_push(&project_root);
            if let Err(e) = push_result {
                let _ = tx.send(Err(format!("Push failed: {}", e)));
                return;
            }

            // Then notify each PR comment, collecting only IDs that were actually notified
            let mut notified_ids: Vec<i64> = Vec::new();
            let mut errors = Vec::new();
            for (cue_id, source_ref, commit_hash) in &pr_cues {
                match crate::sources::notify_pr_finding_fixed(
                    &project_root,
                    source_ref,
                    commit_hash,
                ) {
                    Ok(true) => notified_ids.push(*cue_id),
                    Ok(false) => {}
                    Err(e) => errors.push(e.to_string()),
                }
            }

            let mut msg = format!("Pushed and notified {} PR comment(s)", notified_ids.len());
            if !errors.is_empty() {
                msg.push_str(&format!(
                    "; {} error(s): {}",
                    errors.len(),
                    errors.join(", ")
                ));
            }
            // Encode only actually-notified cue IDs in the result for activity logging
            let ids_str: Vec<String> = notified_ids.iter().map(|id| id.to_string()).collect();
            let _ = tx.send(Ok(format!("{}|{}", msg, ids_str.join(","))));
        });
    }

    pub(super) fn process_pr_notify_result(&mut self) {
        let rx = match self.git.pr_notify_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.git.notifying_pr = false;
                self.git.pr_notify_rx = None;
                self.set_status_message("PR notify failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.git.notifying_pr = false;
        self.git.pr_notify_rx = None;
        match result {
            Ok(msg) => {
                // Parse "message|id1,id2,..." format
                let parts: Vec<&str> = msg.splitn(2, '|').collect();
                let display_msg = parts[0].to_string();
                if parts.len() == 2 {
                    self.log_activity_for_ids(parts[1]);
                }
                self.set_status_message(display_msg);
                self.reload_git_info();
                self.reload_commit_history();
            }
            Err(e) => {
                self.set_status_message(format!("PR notify failed: {}", e));
            }
        }
    }

    /// Log "Notified PR" activity for a comma-separated list of cue IDs.
    fn log_activity_for_ids(&mut self, ids_csv: &str) {
        for id_str in ids_csv.split(',') {
            if let Ok(cue_id) = id_str.parse::<i64>() {
                let _ = self.db.log_activity(cue_id, "Notified PR");
            }
        }
    }

    /// Check for completed git pull.
    pub(super) fn process_pull_result(&mut self) {
        let rx = match self.git.pull_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.git.pulling = false;
                self.git.pull_rx = None;
                self.set_status_message("Git pull failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.git.pulling = false;
        self.git.pull_rx = None;
        match result {
            Ok(msg) => {
                self.set_status_message(msg);
                self.reload_git_info();
                self.reload_commit_history();
            }
            Err(e) => self.handle_pull_error(&e),
        }
    }

    /// Classify a pull error and show the appropriate dialog or message.
    pub(super) fn handle_pull_error(&mut self, e: &str) {
        let is_diverged = e.contains("Not possible to fast-forward")
            || e.contains("Diverging branches")
            || e.contains("not possible to fast-forward");
        let is_conflict = e.contains("CONFLICT")
            || e.contains("Automatic merge failed")
            || e.contains("could not apply");
        let is_unmerged = e.contains("unmerged files") || e.contains("unresolved conflict");

        if is_diverged {
            self.git.show_pull_diverged = true;
            self.set_status_message("Pull: branches have diverged — choose a strategy".to_string());
        } else if is_conflict || is_unmerged {
            self.open_merge_conflict_dialog();
        } else {
            self.set_status_message(format!("Pull failed: {}", e));
        }
    }

    /// Populate conflict state and show the merge conflict resolution dialog.
    pub(super) fn open_merge_conflict_dialog(&mut self) {
        let op = git::detect_merge_operation(&self.project_root);
        let files = git::get_conflicted_files(&self.project_root);
        if files.is_empty() && op.is_none() {
            // No active operation and no conflicts — fall back to the old informational dialog
            self.git.show_pull_unmerged = true;
            self.set_status_message("Pull: resolve unmerged files first".to_string());
            return;
        }
        self.git.merge_operation = op;
        self.git.conflict_files = files;
        self.git.show_merge_conflicts = true;
        let label = match op {
            Some(git::MergeOperation::Merge) => "Merge",
            Some(git::MergeOperation::Rebase) => "Rebase",
            None => "Operation",
        };
        self.set_status_message(format!(
            "{}: {} conflicted file(s) — resolve and continue",
            label,
            self.git.conflict_files.len()
        ));
    }

    pub(super) fn reload_git_info(&mut self) {
        self.git.info = git::read_git_info(&self.project_root);
        self.git.dirty_files = git::get_dirty_files(&self.project_root);
        self.git.ahead_of_remote = git::get_ahead_of_remote(&self.project_root);
    }

    pub(super) fn reload_commit_history(&mut self) {
        let limit = self.git.commit_history_limit.max(self.git.ahead_of_remote);
        self.git.commit_history = git::read_commit_history(&self.project_root, limit);
        self.git.commit_history_total = git::count_commits(&self.project_root);
    }
}
