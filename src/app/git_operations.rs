use std::sync::mpsc;
use std::time::Instant;

use crate::db::CueStatus;
use crate::git;
use crate::jj;
use crate::settings::VcsBackend;

use super::{detect_pr_number_from_branch, DirigentApp};

impl DirigentApp {
    /// Start an async push operation (git push or jj git push).
    pub(super) fn start_git_push(&mut self) {
        if self.git.pushing {
            return;
        }
        self.git.pushing = true;
        let (tx, rx) = mpsc::channel();
        self.git.push_rx = Some(rx);
        let root = self.project_root.clone();
        let backend = self.settings.vcs_backend.clone();
        let jj_path = self.settings.jj_cli_path.clone();
        std::thread::spawn(move || {
            let result = match backend {
                VcsBackend::Jj => jj::jj_push(&root, &jj_path).map_err(|e| e.to_string()),
                VcsBackend::Git => git::git_push(&root).map_err(|e| e.to_string()),
            };
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
                self.git.push_error_message = e;
                self.git.show_push_error = true;
                self.set_status_message("Push failed — see dialog".to_string());
            }
        }
    }

    /// Start an async pull/fetch operation.
    pub(super) fn start_git_pull(&mut self) {
        match self.settings.vcs_backend {
            VcsBackend::Jj => self.start_jj_fetch(),
            VcsBackend::Git => self.start_git_pull_with_strategy(git::PullStrategy::FfOnly),
        }
    }

    /// Start an async jj git fetch.
    fn start_jj_fetch(&mut self) {
        if self.git.pulling {
            return;
        }
        self.git.pulling = true;
        let (tx, rx) = mpsc::channel();
        self.git.pull_rx = Some(rx);
        let root = self.project_root.clone();
        let jj_path = self.settings.jj_cli_path.clone();
        std::thread::spawn(move || {
            let result = jj::jj_pull(&root, &jj_path).map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.set_status_message("Fetching...".to_string());
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

    /// Open the "Switch Branch" dialog with available branches populated.
    pub(super) fn open_switch_branch_dialog(&mut self) {
        match self.settings.vcs_backend {
            VcsBackend::Jj => {
                let infos = jj::jj_list_bookmarks_with_status(
                    &self.project_root,
                    &self.settings.jj_cli_path,
                )
                .unwrap_or_default();
                self.git.bookmark_push_statuses = infos
                    .iter()
                    .map(|b| (b.name.clone(), b.push_status))
                    .collect();
                self.git.available_branches = infos.into_iter().map(|b| b.name).collect();
                // Git ownership doesn't apply to JJ bookmarks; clear any stale
                // Git-only names so render_switch_branch_body doesn't highlight them.
                self.git.own_branches.clear();
            }
            VcsBackend::Git => {
                self.git.available_branches =
                    git::list_branches(&self.project_root).unwrap_or_default();
                self.git.own_branches = git::own_branches(&self.project_root);
                self.git.bookmark_push_statuses.clear();
            }
        }
        self.git.show_switch_branch = true;
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
        let backend = self.settings.vcs_backend.clone();
        let jj_path = self.settings.jj_cli_path.clone();
        std::thread::spawn(move || {
            // Push first so the remote branch exists
            let push_result = match backend {
                VcsBackend::Jj => jj::jj_push(&root, &jj_path),
                VcsBackend::Git => git::git_push(&root),
            };
            if let Err(e) = push_result {
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
                self.git.pr_error_message = e;
                self.git.show_pr_error = true;
                self.set_status_message("PR creation failed — see dialog".to_string());
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
                if let Some(start) = self.git.importing_pr_start {
                    let secs = start.elapsed().as_secs();
                    let pr = self.git.import_pr_number.trim();
                    self.set_status_message(format!("Refreshing PR #{}… ({}s)", pr, secs));
                }
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
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

        let findings = match result {
            Ok(f) => f,
            Err(e) => {
                self.set_status_message(format!("PR import failed: {}", e));
                return;
            }
        };

        if findings.is_empty() {
            self.set_status_message("No actionable findings found in PR".to_string());
            return;
        }

        let total_fetched = findings.len();
        let new_findings = self.filter_already_imported(findings);
        let skipped = total_fetched - new_findings.len();

        let skip_note = if skipped > 0 {
            format!(" ({} already imported)", skipped)
        } else {
            String::new()
        };
        self.set_status_message(format!(
            "Fetched {} new findings – review and filter before importing{}",
            new_findings.len(),
            skip_note
        ));
        self.git.pr_findings_pending = new_findings;
        self.git.pr_findings_excluded.clear();
        self.git.pr_filter_patterns_page = false;

        self.auto_exclude_by_patterns();

        self.git.show_import_pr = false;
        self.git.show_pr_filter = true;
    }

    /// Filter out findings that are already imported (exist in DB by source_ref).
    fn filter_already_imported(
        &self,
        findings: Vec<crate::sources::PrFinding>,
    ) -> Vec<crate::sources::PrFinding> {
        findings
            .into_iter()
            .filter(|f| !matches!(self.db.cue_exists_by_source_ref(&f.external_id), Ok(true)))
            .collect()
    }

    /// Load filter patterns and auto-exclude matching findings.
    fn auto_exclude_by_patterns(&mut self) {
        self.git.pr_filter_patterns = self.db.list_pr_filter_patterns().unwrap_or_default();
        for (idx, finding) in self.git.pr_findings_pending.iter().enumerate() {
            let dominated = self.git.pr_filter_patterns.iter().any(|pat| {
                let haystack = match pat.match_field.as_str() {
                    "file_path" => &finding.file_path,
                    _ => &finding.text,
                };
                haystack
                    .to_lowercase()
                    .contains(&pat.pattern.to_lowercase())
            });
            if dominated {
                self.git.pr_findings_excluded.insert(idx);
            }
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
                            log::error!("DB error updating finding {}: {e}", finding.external_id);
                            error_count += 1;
                        }
                    }
                    continue;
                }
                Ok(None) => {} // New finding — fall through to insert
                Err(e) => {
                    log::error!(
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
                "",
                &finding.external_id,
                &finding.file_path,
                finding.line_number,
            ) {
                Ok(id) => {
                    if let Err(e) = self.db.update_cue_tag(id, Some(tag)) {
                        log::error!("DB error tagging new cue {id}: {e}");
                        error_count += 1;
                    } else {
                        new_count += 1;
                    }
                }
                Err(e) => {
                    log::error!("DB error inserting finding {}: {e}", finding.external_id);
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
        let backend = self.settings.vcs_backend.clone();
        let jj_path = self.settings.jj_cli_path.clone();

        std::thread::spawn(move || {
            // First push
            let push_result = match backend {
                VcsBackend::Jj => crate::jj::jj_push(&project_root, &jj_path),
                VcsBackend::Git => crate::git::git_push(&project_root),
            };
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

    /// Open the Move to Branch dialog.
    pub(super) fn open_move_to_branch_dialog(&mut self) {
        self.git.move_to_branch_name.clear();
        self.git.move_to_branch_needs_focus = true;
        self.git.show_move_to_branch = true;
    }

    /// Start an async move-to-branch operation: create a new branch at HEAD,
    /// then reset the current branch to its remote tracking branch.
    pub(super) fn start_move_to_branch(&mut self) {
        let name = self.git.move_to_branch_name.trim().to_string();
        if name.is_empty() {
            self.set_status_message("Branch name cannot be empty".into());
            return;
        }
        if self.git.moving_to_branch {
            return;
        }
        self.git.moving_to_branch = true;
        self.git.show_move_to_branch = false;
        let (tx, rx) = mpsc::channel();
        self.git.move_to_branch_rx = Some(rx);
        let root = self.project_root.clone();
        let branch_name = name.clone();
        std::thread::spawn(move || {
            let result = git::move_to_new_branch(&root, &branch_name).map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.set_status_message(format!("Moving commits to branch '{}'...", name));
    }

    /// Check for completed move-to-branch operation.
    pub(super) fn process_move_to_branch_result(&mut self) {
        let rx = match self.git.move_to_branch_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.git.moving_to_branch = false;
                self.git.move_to_branch_rx = None;
                self.show_move_to_branch_error(
                    "The move operation ended unexpectedly before reporting a result.".into(),
                );
                // The operation is non-atomic: the new branch may have been
                // created before the failure, so refresh state.
                self.reload_git_info();
                self.reload_commit_history();
                return;
            }
            Ok(r) => r,
        };
        self.git.moving_to_branch = false;
        self.git.move_to_branch_rx = None;
        match result {
            Ok(branch_name) => {
                self.set_status_message(format!(
                    "Moved commits to '{}' — open Worktrees to switch and create a PR",
                    branch_name
                ));
            }
            Err(e) => {
                self.show_move_to_branch_error(e);
            }
        }
        // Always refresh: the operation is non-atomic (branch created before
        // reset), so even failures may have changed repo state.
        self.reload_git_info();
        self.reload_commit_history();
    }

    /// Surface a move-to-branch failure in a dialog (not just a toast), so the
    /// user can read the error and any suggested remedy instead of missing it.
    pub(super) fn show_move_to_branch_error(&mut self, message: String) {
        self.set_status_message(format!("Move to branch failed: {}", message));
        self.git.move_to_branch_error_message = message;
        self.git.show_move_to_branch_error = true;
    }

    /// Open the Create Bookmark dialog (jj only).
    pub(super) fn open_create_bookmark_dialog(&mut self) {
        self.git.create_bookmark_name.clear();
        self.git.create_bookmark_needs_focus = true;
        self.git.show_create_bookmark = true;
    }

    /// Create a jj bookmark at the current commit (runs off the UI thread).
    pub(super) fn start_create_bookmark(&mut self) {
        let name = self.git.create_bookmark_name.trim().to_string();
        if name.is_empty() {
            self.set_status_message("Bookmark name cannot be empty".into());
            return;
        }
        if self.git.creating_bookmark {
            return;
        }
        self.git.active_bookmark = Some(name.clone());
        self.git.show_create_bookmark = false;
        self.git.creating_bookmark = true;
        let (tx, rx) = mpsc::channel();
        self.git.create_bookmark_rx = Some(rx);
        let root = self.project_root.clone();
        let jj_path = self.settings.jj_cli_path.clone();
        let bm = name.clone();
        std::thread::spawn(move || {
            let result = jj::jj_create_bookmark(&root, &bm, &jj_path)
                .map(|()| format!("Created bookmark '{}'", bm))
                .map_err(|e| format!("Create bookmark failed: {}", e));
            let _ = tx.send(result);
        });
        self.set_status_message(format!("Creating bookmark '{}'...", name));
    }

    /// Check for completed bookmark creation.
    pub(super) fn process_create_bookmark_result(&mut self) {
        let rx = match self.git.create_bookmark_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.git.creating_bookmark = false;
                self.git.create_bookmark_rx = None;
                self.set_status_message("Create bookmark failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.git.creating_bookmark = false;
        self.git.create_bookmark_rx = None;
        match result {
            Ok(msg) => self.set_status_message(msg),
            Err(e) => self.set_status_message(e),
        }
        self.reload_git_info();
        self.reload_commit_history();
    }

    /// Squash all commits on the current bookmark into a single commit (jj only).
    pub(super) fn start_squash_current_bookmark(&mut self) {
        if self.git.squashing {
            return;
        }
        let bookmark = self
            .git
            .info
            .as_ref()
            .map(|i| i.branch.clone())
            .unwrap_or_default();
        if bookmark.is_empty() {
            self.set_status_message("No bookmark to squash".into());
            return;
        }
        self.git.squashing = true;
        let (tx, rx) = mpsc::channel();
        self.git.squash_rx = Some(rx);
        let root = self.project_root.clone();
        let jj_path = self.settings.jj_cli_path.clone();
        let bm = bookmark.clone();
        std::thread::spawn(move || {
            let result = jj::jj_squash_bookmark(&root, &bm, &jj_path)
                .map(|n| {
                    if n == 0 {
                        format!("Nothing to squash \u{2014} '{}' has 0 or 1 commits", bm)
                    } else {
                        let plural = if n == 1 { "" } else { "s" };
                        format!("Squashed {} commit{} on '{}' into one", n, plural, bm)
                    }
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.set_status_message(format!("Squashing commits on '{}'...", bookmark));
    }

    /// Check for completed squash operation.
    pub(super) fn process_squash_result(&mut self) {
        let rx = match self.git.squash_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.git.squashing = false;
                self.git.squash_rx = None;
                self.set_status_message("Squash failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.git.squashing = false;
        self.git.squash_rx = None;
        match result {
            Ok(msg) => self.set_status_message(msg),
            Err(e) => self.set_status_message(format!("Squash failed: {}", e)),
        }
        self.git.history_cache_key = (String::new(), 0);
        self.reload_git_info();
        self.reload_commit_history();
    }

    /// Undo the last jj operation (runs off the UI thread).
    pub(super) fn start_jj_undo(&mut self) {
        if self.git.undoing {
            return;
        }
        self.git.undoing = true;
        let (tx, rx) = mpsc::channel();
        self.git.undo_rx = Some(rx);
        let root = self.project_root.clone();
        let jj_path = self.settings.jj_cli_path.clone();
        std::thread::spawn(move || {
            let result = jj::jj_undo(&root, &jj_path).map_err(|e| format!("Undo failed: {}", e));
            let _ = tx.send(result);
        });
        self.set_status_message("Undoing last operation...".into());
    }

    /// Check for completed undo operation.
    pub(super) fn process_undo_result(&mut self) {
        let rx = match self.git.undo_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.git.undoing = false;
                self.git.undo_rx = None;
                self.set_status_message("Undo failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.git.undoing = false;
        self.git.undo_rx = None;
        match result {
            Ok(msg) => self.set_status_message(msg),
            Err(e) => self.set_status_message(e),
        }
        self.git.history_cache_key = (String::new(), 0);
        self.reload_git_info();
        self.reload_commit_history();
    }

    /// Open the Delete Bookmark dialog (jj only).
    pub(super) fn open_delete_bookmark_dialog(&mut self) {
        let infos =
            jj::jj_list_bookmarks_with_status(&self.project_root, &self.settings.jj_cli_path)
                .unwrap_or_default();
        self.git.available_branches = infos.into_iter().map(|b| b.name).collect();
        self.git.merged_bookmarks =
            jj::jj_merged_bookmarks(&self.project_root, &self.settings.jj_cli_path);
        self.git.trunk_bookmarks =
            jj::jj_trunk_bookmarks(&self.project_root, &self.settings.jj_cli_path);
        self.git.show_delete_bookmark = true;
    }

    /// Whether a bookmark is the repository's trunk and must not be deleted.
    ///
    /// Prefers the trunk bookmark(s) resolved from jj's `trunk()` revset so a
    /// repo whose trunk is not literally `main`/`master` (e.g. `develop`) is
    /// still protected; falls back to the conventional names when `trunk()`
    /// could not be resolved.
    pub(in crate::app) fn is_protected_bookmark(&self, name: &str) -> bool {
        if self.git.trunk_bookmarks.iter().any(|b| b == name) {
            return true;
        }
        self.git.trunk_bookmarks.is_empty() && (name == "main" || name == "master")
    }

    /// Start an async delete-bookmark operation (jj only).
    pub(super) fn start_delete_bookmark(&mut self, name: &str) {
        if self.git.deleting_bookmark {
            return;
        }
        // Never delete the repository's trunk bookmark.
        if self.is_protected_bookmark(name) {
            self.set_status_message(format!("Refusing to delete protected bookmark '{}'", name));
            return;
        }
        self.git.deleting_bookmark = true;
        let (tx, rx) = mpsc::channel();
        self.git.delete_bookmark_rx = Some(rx);
        let root = self.project_root.clone();
        let jj_path = self.settings.jj_cli_path.clone();
        let bm = name.to_string();
        self.set_status_message(format!("Deleting bookmark '{}'...", bm));
        std::thread::spawn(move || {
            let result = jj::jj_delete_bookmark(&root, &bm, &jj_path)
                .map(|()| format!("Deleted bookmark '{}'", bm))
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
    }

    /// Start an async delete of every bookmark fully merged into trunk (jj only).
    ///
    /// The repository's trunk bookmark is never deleted, even though it is
    /// itself reported as merged into `trunk()`.
    pub(super) fn start_delete_merged_bookmarks(&mut self) {
        if self.git.deleting_bookmark {
            return;
        }
        let merged: Vec<String> = self
            .git
            .merged_bookmarks
            .iter()
            .filter(|b| !self.is_protected_bookmark(b))
            .cloned()
            .collect();
        if merged.is_empty() {
            self.set_status_message("No merged bookmarks to delete".to_string());
            return;
        }
        self.git.deleting_bookmark = true;
        let (tx, rx) = mpsc::channel();
        self.git.delete_bookmark_rx = Some(rx);
        let root = self.project_root.clone();
        let jj_path = self.settings.jj_cli_path.clone();
        self.set_status_message(format!("Deleting {} merged bookmark(s)...", merged.len()));
        std::thread::spawn(move || {
            let mut deleted = 0usize;
            let mut errors: Vec<String> = Vec::new();
            for bm in &merged {
                match jj::jj_delete_bookmark(&root, bm, &jj_path) {
                    Ok(()) => deleted += 1,
                    Err(e) => errors.push(format!("{}: {}", bm, e)),
                }
            }
            let result = if errors.is_empty() {
                Ok(format!("Deleted {} merged bookmark(s)", deleted))
            } else {
                Err(format!(
                    "Deleted {}; failed — {}",
                    deleted,
                    errors.join("; ")
                ))
            };
            let _ = tx.send(result);
        });
    }

    /// Check for completed delete-bookmark operation.
    pub(super) fn process_delete_bookmark_result(&mut self) {
        let rx = match self.git.delete_bookmark_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.git.deleting_bookmark = false;
                self.git.delete_bookmark_rx = None;
                self.set_status_message("Bookmark deletion failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.git.deleting_bookmark = false;
        self.git.delete_bookmark_rx = None;
        match result {
            Ok(msg) => self.set_status_message(msg),
            Err(e) => self.set_status_message(format!("Delete failed: {}", e)),
        }
        // Refresh the branch list in the dialog if still open. This must run for
        // both success and failure: a partial-success deletion returns an Err but
        // still removes some bookmarks, so the list would otherwise stay stale.
        if self.git.show_delete_bookmark {
            let infos =
                jj::jj_list_bookmarks_with_status(&self.project_root, &self.settings.jj_cli_path)
                    .unwrap_or_default();
            self.git.available_branches = infos.into_iter().map(|b| b.name).collect();
            self.git.merged_bookmarks =
                jj::jj_merged_bookmarks(&self.project_root, &self.settings.jj_cli_path);
        }
        self.git.history_cache_key = (String::new(), 0);
        self.reload_git_info();
        self.reload_commit_history();
    }

    /// Open the Merge Bookmark dialog (jj only).
    pub(super) fn open_merge_bookmark_dialog(&mut self) {
        let infos =
            jj::jj_list_bookmarks_with_status(&self.project_root, &self.settings.jj_cli_path)
                .unwrap_or_default();
        self.git.bookmark_push_statuses = infos
            .iter()
            .map(|b| (b.name.clone(), b.push_status))
            .collect();
        self.git.available_branches = infos.into_iter().map(|b| b.name).collect();
        self.git.show_merge_bookmark = true;
    }

    /// Start an async merge-bookmark operation (jj only).
    pub(super) fn start_merge_bookmark(&mut self, source: &str) {
        if self.git.merging_bookmark {
            return;
        }
        self.git.merging_bookmark = true;
        let (tx, rx) = mpsc::channel();
        self.git.merge_bookmark_rx = Some(rx);
        let root = self.project_root.clone();
        let jj_path = self.settings.jj_cli_path.clone();
        let source_owned = source.to_string();
        let dest_bm = self.git.active_bookmark.clone();
        self.set_status_message(format!("Merging '{}'...", source_owned));
        std::thread::spawn(move || {
            let result = jj::jj_merge_bookmark(&root, &source_owned, &jj_path, dest_bm.as_deref())
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
    }

    /// Check for completed merge-bookmark operation.
    pub(super) fn process_merge_bookmark_result(&mut self) {
        let rx = match self.git.merge_bookmark_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.git.merging_bookmark = false;
                self.git.merge_bookmark_rx = None;
                self.set_status_message("Merge failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.git.merging_bookmark = false;
        self.git.merge_bookmark_rx = None;
        match result {
            Ok(msg) => self.set_status_message(msg),
            Err(e) => self.set_status_message(format!("Merge failed: {}", e)),
        }
        self.git.history_cache_key = (String::new(), 0);
        self.reload_git_info();
        self.reload_commit_history();
        self.force_reload_open_tabs();
        self.reload_file_tree();
    }

    /// Open the Commit dialog (jj only).
    pub(super) fn open_commit_dialog(&mut self) {
        self.git.commit_message_input.clear();
        self.git.commit_review_cue_id = None;
        self.git.commit_needs_focus = true;
        self.git.show_commit_dialog = true;
    }

    /// Commit the working copy with the user's message (runs off the UI thread).
    ///
    /// Backend-aware: routes through [`vcs_dispatch`](super::vcs_dispatch) so the
    /// editable commit dialog works for both git and jj. When a review cue is
    /// pending its execution diff is committed directly; otherwise the whole
    /// working copy is committed.
    pub(super) fn start_commit(&mut self) {
        let msg = self.git.commit_message_input.trim().to_string();
        if msg.is_empty() {
            self.set_status_message("Commit message cannot be empty".into());
            return;
        }
        if self.git.committing {
            return;
        }

        let cue_id = self.git.commit_review_cue_id.take();

        let diff_text = cue_id.and_then(|id| {
            self.db
                .get_latest_execution(id)
                .ok()
                .flatten()
                .and_then(|e| e.diff)
        });

        self.git.show_commit_dialog = false;
        self.git.committing = true;
        let (tx, rx) = mpsc::channel();
        self.git.commit_rx = Some(rx);
        let root = self.project_root.clone();
        let backend = self.settings.vcs_backend.clone();
        let jj_path = self.settings.jj_cli_path.clone();
        let active_bm = self.git.active_bookmark.clone();
        std::thread::spawn(move || {
            let result = if let Some(ref diff) = diff_text {
                super::vcs_dispatch::commit_diff(
                    &backend,
                    &jj_path,
                    &root,
                    diff,
                    &msg,
                    active_bm.as_deref(),
                )
            } else {
                super::vcs_dispatch::commit_all(
                    &backend,
                    &jj_path,
                    &root,
                    &msg,
                    active_bm.as_deref(),
                )
            };
            let result = result
                .map(|change_id| format!("Committed: {}", &change_id[..7.min(change_id.len())]))
                .map_err(|e| format!("Commit failed: {}", e));
            let _ = tx.send(result);
        });

        self.git.commit_pending_cue_id = cue_id;
        self.set_status_message("Committing...".into());
    }

    /// Check for completed commit operation.
    pub(super) fn process_commit_result(&mut self) {
        let rx = match self.git.commit_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.git.committing = false;
                self.git.commit_rx = None;
                if let Some(id) = self.git.commit_pending_cue_id.take() {
                    let _ = self.db.log_activity(id, "Commit failed");
                }
                self.set_status_message("Commit failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.git.committing = false;
        self.git.commit_rx = None;
        let pending_cue = self.git.commit_pending_cue_id.take();
        match result {
            Ok(msg) => {
                if let Some(id) = pending_cue {
                    let _ = self.db.update_cue_status(id, CueStatus::Done);
                    let _ = self.db.log_activity(id, "Committed");
                    self.clear_review_question_and_recheck_workflow(id);
                }
                self.set_status_message(msg);
            }
            Err(e) => {
                if let Some(id) = pending_cue {
                    let _ = self.db.log_activity(id, "Commit failed");
                }
                self.set_status_message(e);
            }
        }
        self.reload_git_info();
        self.reload_commit_history();
    }

    pub(super) fn reload_git_info(&mut self) {
        match self.settings.vcs_backend {
            VcsBackend::Jj => {
                let jj_path = &self.settings.jj_cli_path;
                self.git.info = jj::jj_read_info(&self.project_root, jj_path);
                self.git.dirty_files = jj::jj_get_dirty_files(&self.project_root, jj_path);
                self.git.ahead_of_remote = jj::jj_get_ahead_of_remote(&self.project_root, jj_path);
            }
            VcsBackend::Git => {
                self.git.info = git::read_git_info(&self.project_root);
                self.git.dirty_files = git::get_dirty_files(&self.project_root);
                self.git.ahead_of_remote = git::get_ahead_of_remote(&self.project_root);
            }
        }
        self.git.diff_lines = super::vcs_dispatch::compute_diff_lines(
            &self.settings.vcs_backend,
            &self.settings.jj_cli_path,
            &self.project_root,
        );
        self.maybe_detect_pr();
    }

    /// Kick off async PR detection if the branch changed since the last detection.
    pub(super) fn maybe_detect_pr(&mut self) {
        let branch = self
            .git
            .info
            .as_ref()
            .map(|i| i.branch.clone())
            .unwrap_or_default();
        if branch == self.git.pr_detect_branch {
            return;
        }
        self.git.pr_detect_branch = branch;
        self.git.pr_number = None;
        self.git.pr_url = None;
        let project_root = self.project_root.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.git.pr_detect_rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(super::detect_pr_info(&project_root));
        });
    }

    pub(super) fn reload_commit_history(&mut self) {
        let limit = self.git.commit_history_limit.max(self.git.ahead_of_remote);
        // Cache key: HEAD hash + requested limit. Skip if unchanged.
        let head_hash = self
            .git
            .info
            .as_ref()
            .map(|i| i.last_commit_hash.clone())
            .unwrap_or_default();
        let cache_key = (head_hash, limit);
        if cache_key == self.git.history_cache_key {
            return;
        }
        match self.settings.vcs_backend {
            VcsBackend::Jj => {
                let jj_path = &self.settings.jj_cli_path;
                self.git.commit_history =
                    jj::jj_read_commit_history(&self.project_root, limit, jj_path);
                self.git.commit_history_total = jj::jj_count_commits(&self.project_root, jj_path);
            }
            VcsBackend::Git => {
                self.git.commit_history = git::read_commit_history(&self.project_root, limit);
                self.git.commit_history_total = git::count_commits(&self.project_root);
            }
        }
        let (rows, max_lanes) = git::graph::compute_graph(&self.git.commit_history);
        self.git.graph_rows = rows;
        self.git.graph_max_lanes = max_lanes;
        self.git.history_cache_key = cache_key;
    }

    /// Find empty head commits and abandon them in one shot (jj only).
    pub(super) fn start_abandon_empty_heads(&mut self) {
        if self.git.abandoning_empty {
            return;
        }
        self.git.abandoning_empty = true;
        let (tx, rx) = mpsc::channel();
        self.git.abandon_empty_rx = Some(rx);
        let root = self.project_root.clone();
        let jj_path = self.settings.jj_cli_path.clone();
        std::thread::spawn(move || {
            let heads = jj::jj_find_empty_heads(&root, &jj_path);
            if heads.is_empty() {
                let _ = tx.send(Ok("No empty head commits found".to_string()));
                return;
            }
            let ids: Vec<String> = heads.iter().map(|(id, _)| id.clone()).collect();
            let count = ids.len();
            let result = jj::jj_abandon(&root, &ids, &jj_path)
                .map(|_| {
                    let plural = if count == 1 { "" } else { "s" };
                    format!("Abandoned {} empty head commit{}", count, plural)
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.set_status_message("Finding empty heads...".into());
    }

    /// Check for completed abandon-empty-heads operation.
    pub(super) fn process_abandon_empty_result(&mut self) {
        let rx = match self.git.abandon_empty_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.git.abandoning_empty = false;
                self.git.abandon_empty_rx = None;
                self.set_status_message("Abandon empty heads failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.git.abandoning_empty = false;
        self.git.abandon_empty_rx = None;
        match result {
            Ok(msg) => self.set_status_message(msg),
            Err(e) => self.set_status_message(format!("Abandon failed: {}", e)),
        }
        self.git.history_cache_key = (String::new(), 0);
        self.reload_git_info();
        self.reload_commit_history();
    }

    /// Open the Clean Up Bookmarks dialog, scanning for suspicious bookmarks.
    pub(super) fn open_cleanup_bookmarks_dialog(&mut self) {
        match jj::jj_find_suspicious_bookmarks(&self.project_root, &self.settings.jj_cli_path) {
            Ok(suspicious) => {
                self.git.suspicious_bookmarks = suspicious;
            }
            Err(e) => {
                self.set_status_message(format!("Failed to scan bookmarks: {}", e));
                return;
            }
        }
        self.git.show_cleanup_bookmarks = true;
    }

    /// Delete a suspicious bookmark (runs off the UI thread).
    pub(super) fn start_delete_suspicious_bookmark(&mut self, name: String) {
        if self.git.cleaning_bookmark {
            return;
        }
        self.git.cleaning_bookmark = true;
        let (tx, rx) = mpsc::channel();
        self.git.cleanup_bookmark_rx = Some(rx);
        let root = self.project_root.clone();
        let jj_path = self.settings.jj_cli_path.clone();
        let bm = name.clone();
        std::thread::spawn(move || {
            let result = jj::jj_delete_bookmark(&root, &bm, &jj_path)
                .map(|()| bm)
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.set_status_message(format!("Deleting bookmark '{}'...", name));
    }

    /// Check for completed cleanup-bookmark deletion.
    pub(super) fn process_cleanup_bookmark_result(&mut self) {
        let rx = match self.git.cleanup_bookmark_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.git.cleaning_bookmark = false;
                self.git.cleanup_bookmark_rx = None;
                self.set_status_message("Bookmark deletion failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.git.cleaning_bookmark = false;
        self.git.cleanup_bookmark_rx = None;
        match result {
            Ok(deleted_name) => {
                self.git
                    .suspicious_bookmarks
                    .retain(|b| b.name != deleted_name);
                self.set_status_message(format!("Deleted bookmark '{}'", deleted_name));
            }
            Err(e) => {
                self.set_status_message(format!("Delete bookmark failed: {}", e));
            }
        }
        self.reload_git_info();
        self.reload_commit_history();
    }
}
