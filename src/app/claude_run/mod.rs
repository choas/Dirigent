use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use super::DirigentApp;
use super::notifications::send_macos_notification;
use super::tasks::TaskHandle;
use crate::agents::AgentTrigger;
use crate::ai_backend::ProviderRunOutcome;
use crate::claude;
use crate::db::{Cue, CueStatus, Execution};
use crate::jj;
use crate::settings::{self, CliProvider, CueCommand, VcsBackend};
use crate::telemetry;

mod mcp_inject;
mod provider_dispatch;
mod result_pipeline;
mod run_diff;

use provider_dispatch::{
    ProviderConfig, build_pr_hint_text, build_provider_config, resolve_command_prefix,
};
use result_pipeline::run_provider;

/// Result of a background Claude invocation.
struct ClaudeResult {
    cue_id: i64,
    exec_id: i64,
    diff: Option<String>,
    response: String,
    error: Option<String>,
    metrics: claude::RunMetrics,
    metadata: Option<claude::ClaudeRunMetadata>,
}

/// A log message from a running Claude worker thread.
struct LogUpdate {
    cue_id: i64,
    text: String,
    provider: CliProvider,
}

/// State for Claude execution and live log streaming.
pub(crate) struct ClaudeRunState {
    tx: mpsc::Sender<ClaudeResult>,
    rx: mpsc::Receiver<ClaudeResult>,
    log_tx: mpsc::Sender<LogUpdate>,
    log_rx: mpsc::Receiver<LogUpdate>,
    pub(super) running_logs: HashMap<i64, (String, CliProvider)>,
    pub(super) start_times: HashMap<i64, Instant>,
    pub(super) exec_ids: HashMap<i64, i64>,
    pub(super) show_log: Option<i64>,
    pub(super) last_log_flush: Instant,
    /// Expand the "Running" section on next frame (after user clicks Run).
    pub(super) expand_running: bool,
    /// Cached past executions for the conversation view (loaded on open).
    pub(super) conversation_history: Vec<Execution>,
    /// Per-cue id of the execution that was the latest run the last time that
    /// cue's conversation view rendered. When a newer run is appended the
    /// previously latest run must be force-collapsed once, because egui's
    /// persisted collapsing state ignores a later change to `default_open`.
    /// Keyed by `cue_id` so switching between conversations doesn't clobber one
    /// cue's demotion state with another's.
    pub(super) conversation_last_latest: HashMap<i64, i64>,
    /// Per-cue timeline of recent log-arrival instants, used to draw the
    /// heartbeat strip beneath the running conversation view.
    pub(super) log_heartbeats: HashMap<i64, VecDeque<Instant>>,
    /// Per-cue instant of the most recent log line ("ping") received from the
    /// CLI/PTY. Unlike `log_heartbeats` this is never pruned, so it can report
    /// how long it has been since Claude last sent anything.
    pub(super) last_message_times: HashMap<i64, Instant>,
    /// Consecutive auto-continue count per cue (reset on normal completion).
    pub(super) auto_continue_count: HashMap<i64, u32>,
    /// Spawn-failure retry count per cue for auto-continues.
    pub(super) auto_continue_spawn_retries: HashMap<i64, u32>,
    pub(super) workspace_paths: HashMap<i64, PathBuf>,
    pub(super) workspace_names: HashMap<i64, String>,
    pub(super) workspace_commit_failed: HashSet<i64>,
}

impl ClaudeRunState {
    pub(super) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let (log_tx, log_rx) = mpsc::channel();
        ClaudeRunState {
            tx,
            rx,
            log_tx,
            log_rx,
            running_logs: HashMap::new(),
            start_times: HashMap::new(),
            exec_ids: HashMap::new(),
            show_log: None,
            last_log_flush: Instant::now(),
            expand_running: false,
            conversation_history: Vec::new(),
            conversation_last_latest: HashMap::new(),
            log_heartbeats: HashMap::new(),
            last_message_times: HashMap::new(),
            auto_continue_count: HashMap::new(),
            auto_continue_spawn_retries: HashMap::new(),
            workspace_paths: HashMap::new(),
            workspace_names: HashMap::new(),
            workspace_commit_failed: HashSet::new(),
        }
    }
}

/// Bundles the parameters needed by a single CLI provider invocation.
struct RunRequest<'a> {
    provider: CliProvider,
    prompt: &'a str,
    project_root: &'a Path,
    config: &'a ProviderConfig,
    cue_id: i64,
    exec_id: i64,
    vcs_backend: VcsBackend,
    jj_cli_path: String,
}

/// Session continuity for Claude Code runs (direct or in a herdr pane):
/// `--resume <id>` to continue a conversation, else `--session-id <new id>` so
/// a later reply can resume it. The id also scopes the Stop hook to this run.
/// Returns the args and the id to store, or `None` for other providers.
fn session_continuity_args(
    provider: &CliProvider,
    resume_session_id: Option<String>,
    new_id: impl FnOnce() -> String,
) -> Option<(Vec<String>, String)> {
    if !provider.runs_claude_code() {
        return None;
    }
    let (flag, id) = match resume_session_id {
        Some(id) => ("--resume", id),
        None => ("--session-id", new_id()),
    };
    Some((vec![flag.to_string(), id.clone()], id))
}

impl DirigentApp {
    /// Build the initial prompt for a cue, gathering auto-context for Claude provider.
    fn build_initial_prompt(&self, effective_text: &str, cue: &Cue) -> String {
        if claude::is_import_request(effective_text) {
            return claude::build_import_prompt(effective_text, &cue.attached_images);
        }

        let want_file = self.settings.auto_context_file;
        let want_diff = self.settings.auto_context_git_diff;
        let auto_context = if want_file || want_diff {
            claude::gather_auto_context(
                &self.project_root,
                &cue.file_path,
                cue.line_number,
                cue.line_number_end,
                want_file,
                want_diff,
            )
        } else {
            String::new()
        };

        let effective_text = build_pr_hint_text(effective_text, cue.source_ref.as_deref());

        // Both providers use the same prompt structure with auto-context.
        claude::build_prompt_with_auto_context(
            &effective_text,
            &cue.file_path,
            cue.line_number,
            cue.line_number_end,
            &cue.attached_images,
            &auto_context,
        )
    }

    /// Spawn a background thread to run the CLI provider and return the task handle.
    fn spawn_provider_thread(
        &self,
        cue_id: i64,
        exec_id: i64,
        prompt: String,
        provider: CliProvider,
        config: ProviderConfig,
    ) -> TaskHandle {
        let project_root = self
            .claude
            .workspace_paths
            .get(&cue_id)
            .cloned()
            .unwrap_or_else(|| self.project_root.clone());
        let claude_tx = self.claude.tx.clone();
        let log_tx = self.claude.log_tx.clone();
        let vcs_backend = self.settings.vcs_backend.clone();
        let jj_cli_path = self.settings.jj_cli_path.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_thread = Arc::clone(&cancel);
        let provider_for_log = provider.clone();

        let join_handle = std::thread::spawn(move || {
            let on_log = move |text: &str| {
                let _ = log_tx.send(LogUpdate {
                    cue_id,
                    text: text.to_string(),
                    provider: provider_for_log.clone(),
                });
            };
            let req = RunRequest {
                provider,
                prompt: &prompt,
                project_root: &project_root,
                config: &config,
                cue_id,
                exec_id,
                vcs_backend: vcs_backend.clone(),
                jj_cli_path: jj_cli_path.clone(),
            };
            let result = run_provider(&req, on_log, cancel_thread);
            let _ = claude_tx.send(result);
        });

        TaskHandle {
            join_handle,
            cancel,
            cue_id: Some(cue_id),
            exec_id: Some(exec_id),
        }
    }

    /// Insert a new execution record and spawn the provider thread.
    /// Returns `true` on success, `false` if the execution record could not be
    /// created (the caller should skip any post-spawn cleanup in that case).
    ///
    /// When `resume_session_id` is `Some`, `--resume <id>` is passed to continue
    /// the Claude conversation. Otherwise a new session UUID is generated and
    /// passed via `--session-id` so it can be resumed on subsequent replies.
    fn insert_exec_and_spawn(
        &mut self,
        cue_id: i64,
        prompt: String,
        provider: CliProvider,
        matched_command: &Option<CueCommand>,
        resume_session_id: Option<String>,
    ) -> bool {
        let exec_id = match self.db.insert_execution(cue_id, &prompt, &provider) {
            Ok(id) => id,
            Err(e) => {
                log::error!("Failed to create execution record for cue {cue_id}: {e}");
                self.claude.running_logs.remove(&cue_id);
                self.claude.start_times.remove(&cue_id);
                self.claude.log_heartbeats.remove(&cue_id);
                self.claude.last_message_times.remove(&cue_id);
                self.set_status_message(format!("Failed to start run: {e}"));
                return false;
            }
        };

        self.claude.exec_ids.insert(cue_id, exec_id);
        if let Ok(execs) = self.db.get_all_executions(cue_id) {
            self.claude.conversation_history = execs;
        }

        let mut config = build_provider_config(
            &self.settings,
            &provider,
            matched_command,
            &prompt,
            &self.project_root,
        );

        if let Some((args, session_id)) =
            session_continuity_args(&provider, resume_session_id, || {
                uuid::Uuid::new_v4().to_string()
            })
        {
            config.extra_args_vec.extend(args);
            // Losing this breaks --resume for the run, but the run itself is
            // fine, so log rather than interrupt the user with a status message.
            self.db_log(
                "store session id",
                self.db.update_execution_session_id(exec_id, &session_id),
            );
        }

        telemetry::emit_execution_started(
            &self.project_name(),
            cue_id,
            provider.display_name(),
            &config.model,
        );

        let handle = self.spawn_provider_thread(cue_id, exec_id, prompt, provider, config);
        self.task_handles.push(handle);
        true
    }

    pub(super) fn trigger_claude(&mut self, cue_id: i64) {
        let provider = self.settings.cli_provider.clone();
        self.trigger_run_with_provider(cue_id, provider);
    }

    /// Run a cue with `provider` instead of the configured one (the prompt
    /// field's experimental herdr button).
    pub(super) fn trigger_run_with_provider(&mut self, cue_id: i64, provider: CliProvider) {
        if let Err(e) = settings::sync_home_guard_hook(
            &self.project_root,
            self.settings.allow_home_folder_access,
        ) {
            log::error!("Failed to sync home guard hook: {e:#}");
            return;
        }

        // Start tracking immediately so the timer appears in the UI while
        // we build the prompt (which may involve blocking I/O for auto-context).
        self.claude
            .running_logs
            .insert(cue_id, (String::new(), provider.clone()));
        self.claude.start_times.insert(cue_id, Instant::now());
        self.cue_warnings.remove(&cue_id);

        let cue = match self.cues.iter().find(|c| c.id == cue_id) {
            Some(c) => c.clone(),
            None => {
                // Cue not found — revert tracking state and status so the card
                // doesn't stay stuck in "Running" forever.
                self.claude.running_logs.remove(&cue_id);
                self.claude.start_times.remove(&cue_id);
                self.claude.log_heartbeats.remove(&cue_id);
                self.claude.last_message_times.remove(&cue_id);
                self.db_log(
                    "move cue back to Inbox",
                    self.db.update_cue_status(cue_id, CueStatus::Inbox),
                );
                self.set_status_message("Failed to start run: cue not found".to_string());
                self.reload_cues();
                return;
            }
        };

        let (effective_text, matched_command) =
            resolve_command_prefix(&cue.text, &self.settings.commands);
        let prompt = self.build_initial_prompt(&effective_text, &cue);

        if !self.insert_exec_and_spawn(cue_id, prompt, provider, &matched_command, None) {
            self.db_report(
                "move cue back to Inbox",
                self.db.update_cue_status(cue_id, CueStatus::Inbox),
            );
            self.cleanup_jj_workspace(cue_id);
            self.reload_cues();
        }
    }

    pub(super) fn trigger_claude_reply(
        &mut self,
        cue_id: i64,
        reply: &str,
        reply_images: &[String],
    ) {
        if let Err(e) = settings::sync_home_guard_hook(
            &self.project_root,
            self.settings.allow_home_folder_access,
        ) {
            log::error!("Failed to sync home guard hook: {e:#}");
            return;
        }

        let cue = match self.cues.iter().find(|c| c.id == cue_id) {
            Some(c) => c.clone(),
            None => {
                self.set_status_message("Failed to start reply: cue not found".to_string());
                return;
            }
        };

        let latest_exec = self.db.get_latest_execution(cue_id).ok().flatten();

        let provider = latest_exec
            .as_ref()
            .map(|e| e.provider.clone())
            .unwrap_or_else(|| self.settings.cli_provider.clone());

        let (raw_text, matched_command) =
            resolve_command_prefix(&cue.text, &self.settings.commands);
        let original_text = build_pr_hint_text(&raw_text, cue.source_ref.as_deref());

        // Remember the raw follow-up text so a later commit can use it as the
        // commit message basis instead of the original cue text.
        if !reply.trim().is_empty() {
            self.last_follow_up.insert(cue_id, reply.trim().to_string());
        }

        let mut all_images = cue.attached_images.clone();
        all_images.extend_from_slice(reply_images);

        let resume_session_id = latest_exec.as_ref().and_then(|e| e.session_id.clone());

        // When resuming a Claude Code session the previous context is already
        // in the conversation, so we only need to send the user's reply. For a
        // fresh session (or another provider) we build the full structured
        // prompt.
        // We also need the full prompt when there are new images to attach.
        let prompt = if resume_session_id.is_some()
            && all_images.is_empty()
            && provider.runs_claude_code()
        {
            reply.to_string()
        } else {
            let previous_diff = latest_exec.and_then(|e| e.diff).unwrap_or_default();

            claude::build_reply_prompt(
                &original_text,
                &cue.file_path,
                cue.line_number,
                cue.line_number_end,
                &previous_diff,
                reply,
                &all_images,
                Some(&self.project_root),
            )
        };

        self.db_report(
            "move cue to Ready",
            self.db.update_cue_status(cue_id, CueStatus::Ready),
        );
        self.claude.expand_running = true;
        self.reload_cues();

        // Start tracking immediately so the timer appears in the UI.
        self.claude
            .running_logs
            .insert(cue_id, (String::new(), provider.clone()));
        self.claude.start_times.insert(cue_id, Instant::now());
        let started = self.insert_exec_and_spawn(
            cue_id,
            prompt,
            provider,
            &matched_command,
            resume_session_id,
        );

        if started {
            if self.diff_review.as_ref().map(|r| r.cue_id) == Some(cue_id) {
                self.diff_review = None;
            }
        } else {
            // Restore the cue to Review (its state before we attempted the reply).
            self.db_report(
                "restore cue to Review",
                self.db.update_cue_status(cue_id, CueStatus::Review),
            );
            self.reload_cues();
        }
    }

    /// Dispatch an auto-continue with rollback appropriate for auto-continues.
    /// On failure the cue is restored to Done (its state before the auto-continue
    /// was enqueued) and re-enqueued into `pending_auto_continues` for one retry.
    /// After a second failure the auto-continue budget is cleared.
    fn dispatch_auto_continue(&mut self, cue_id: i64) {
        if let Err(e) = settings::sync_home_guard_hook(
            &self.project_root,
            self.settings.allow_home_folder_access,
        ) {
            log::error!("Auto-continue sync_home_guard_hook failed for cue {cue_id}: {e:#}");
            self.rollback_auto_continue(cue_id);
            return;
        }

        let cue = match self.cues.iter().find(|c| c.id == cue_id) {
            Some(c) => c.clone(),
            None => {
                self.claude.auto_continue_count.remove(&cue_id);
                self.claude.auto_continue_spawn_retries.remove(&cue_id);
                return;
            }
        };

        let latest_exec = self.db.get_latest_execution(cue_id).ok().flatten();

        let provider = latest_exec
            .as_ref()
            .map(|e| e.provider.clone())
            .unwrap_or_else(|| self.settings.cli_provider.clone());

        let resume_session_id = latest_exec.as_ref().and_then(|e| e.session_id.clone());

        let (raw_text, matched_command) =
            resolve_command_prefix(&cue.text, &self.settings.commands);

        let prompt = if resume_session_id.is_some() && provider.runs_claude_code() {
            "continue".to_string()
        } else {
            let original_text = build_pr_hint_text(&raw_text, cue.source_ref.as_deref());
            let previous_diff = latest_exec.and_then(|e| e.diff).unwrap_or_default();

            claude::build_reply_prompt(
                &original_text,
                &cue.file_path,
                cue.line_number,
                cue.line_number_end,
                &previous_diff,
                "continue",
                &[],
                Some(&self.project_root),
            )
        };

        self.db_report(
            "move cue to Ready",
            self.db.update_cue_status(cue_id, CueStatus::Ready),
        );
        self.claude.expand_running = true;
        self.reload_cues();

        self.claude
            .running_logs
            .insert(cue_id, (String::new(), provider.clone()));
        self.claude.start_times.insert(cue_id, Instant::now());

        if self.insert_exec_and_spawn(
            cue_id,
            prompt,
            provider,
            &matched_command,
            resume_session_id,
        ) {
            self.claude.auto_continue_spawn_retries.remove(&cue_id);
        } else {
            self.claude.running_logs.remove(&cue_id);
            self.claude.start_times.remove(&cue_id);
            self.rollback_auto_continue(cue_id);
        }
    }

    /// Roll back a failed auto-continue dispatch: restore the cue to Done and
    /// re-enqueue for one retry.  After a second failure, give up entirely.
    fn rollback_auto_continue(&mut self, cue_id: i64) {
        let retries = self
            .claude
            .auto_continue_spawn_retries
            .entry(cue_id)
            .or_insert(0);
        *retries += 1;
        if *retries > 1 {
            log::warn!("Auto-continue spawn failed twice for cue {cue_id}, giving up");
            self.claude.auto_continue_count.remove(&cue_id);
            self.claude.auto_continue_spawn_retries.remove(&cue_id);
        } else {
            self.pending_auto_continues.push(cue_id);
        }
        // A failure here leaves the cue stuck in Ready with nothing running.
        self.db_report(
            "restore cue to Done",
            self.db.update_cue_status(cue_id, CueStatus::Done),
        );
        self.reload_cues();
    }

    pub(super) fn process_claude_results(&mut self) {
        self.drain_log_channel();

        let results: Vec<ClaudeResult> = self.claude.rx.try_iter().collect();
        let had_results = !results.is_empty();

        for result in results {
            self.process_single_result(result);
        }

        if had_results {
            self.cached_total_cost = self.db.total_cost().unwrap_or(self.cached_total_cost);
        }

        // Drain any pending auto-continues that were re-enqueued due to spawn
        // failure (process_single_result only drains after a result arrives).
        if !self.pending_auto_continues.is_empty() && !had_results {
            let auto_continues = std::mem::take(&mut self.pending_auto_continues);
            for cue_id in auto_continues {
                self.dispatch_auto_continue(cue_id);
            }
        }
    }

    fn process_single_result(&mut self, result: ClaudeResult) {
        // Check if a newer run has started for the same cue. If so, this is
        // a stale result (e.g. from a cancelled run) — update the DB record
        // but don't touch the live tracking state or cue status.
        let is_stale = self
            .claude
            .exec_ids
            .get(&result.cue_id)
            .is_some_and(|&current| current != result.exec_id);

        self.db_log(
            "store execution metrics",
            self.db.update_execution_metrics(
                result.exec_id,
                result.metrics.cost_usd,
                result.metrics.duration_ms,
                result.metrics.num_turns,
                result.metrics.input_tokens,
                result.metrics.output_tokens,
            ),
        );
        self.persist_execution_metadata(&result);

        // Detect usage limits and emit telemetry BEFORE flushing tracking
        // state, because both need data from running_logs.
        let usage_limit_msg = self.detect_result_usage_limit(&result, is_stale);

        self.emit_completion_telemetry(&result, is_stale, &usage_limit_msg);

        if is_stale {
            self.complete_stale_execution(&result);
            return;
        }

        let is_error = self.dispatch_result(&result, &usage_limit_msg);

        if !is_error {
            self.handle_successful_run(&result);
        }

        // Always refresh git state: even error/rate-limited runs may have
        // committed or pushed during execution.
        self.reload_git_info();
        self.reload_commit_history();

        // Flush log to DB and clear tracking state after all consumers have
        // read running_logs (usage-limit check, telemetry, plan path, etc.).
        self.flush_and_clear_tracking(&result);

        self.refresh_conversation_history(result.cue_id);
        self.reload_cues();

        // Dispatch a queued follow-up now that the completed run's tracking
        // state has been cleared. Doing this earlier (inside
        // `handle_successful_run`) would let `flush_and_clear_tracking` wipe
        // the follow-up's freshly inserted `running_logs`/`exec_ids`/
        // `start_times` — leaving the resumed run with no `current_exec_id`,
        // so its live output never rendered in the conversation view.
        if !is_error {
            self.try_dispatch_follow_up(result.cue_id);
        }

        // Dispatch pending auto-continues now that tracking state is cleared.
        let auto_continues = std::mem::take(&mut self.pending_auto_continues);
        for cue_id in auto_continues {
            self.dispatch_auto_continue(cue_id);
        }

        self.on_workflow_cue_completed(result.cue_id);
    }

    fn persist_execution_metadata(&self, result: &ClaudeResult) {
        let Some(metadata) = result.metadata.as_ref() else {
            return;
        };
        if let Ok(payload) = serde_json::to_value(metadata) {
            self.db_log(
                "store pty metadata event",
                self.db
                    .insert_execution_event(result.exec_id, "pty_metadata", &payload),
            );
        }
        for summary in &metadata.permission_summaries {
            self.db_log(
                "store permission event",
                self.db.insert_execution_event(
                    result.exec_id,
                    "permission",
                    &serde_json::json!({ "summary": summary }),
                ),
            );
        }
        for warning in &metadata.parser_warnings {
            self.db_log(
                "store parser warning event",
                self.db.insert_execution_event(
                    result.exec_id,
                    "parser_warning",
                    &serde_json::json!({ "message": warning }),
                ),
            );
        }
        if let Some(reason) = metadata.completion_reason.as_ref() {
            self.db_log(
                "store completion event",
                self.db.insert_execution_event(
                    result.exec_id,
                    "completion",
                    &serde_json::json!({ "reason": reason }),
                ),
            );
        }
    }

    /// Flush the running log to DB and remove tracking state for a completed result.
    fn flush_and_clear_tracking(&mut self, result: &ClaudeResult) {
        if let Some((log_text, _)) = self.claude.running_logs.get(&result.cue_id) {
            self.db_log(
                "flush execution log",
                self.db.update_execution_log(result.exec_id, log_text),
            );
        }
        self.claude.running_logs.remove(&result.cue_id);
        self.claude.exec_ids.remove(&result.cue_id);
        self.claude.start_times.remove(&result.cue_id);
        self.claude.log_heartbeats.remove(&result.cue_id);
        self.claude.last_message_times.remove(&result.cue_id);
    }

    /// Detect usage-limit messages in the response or log.
    ///
    /// `running_logs` is keyed by `cue_id`, so for stale results (where a
    /// newer execution has already started) the buffer belongs to a different
    /// run.  Only consult it when the result is current.
    fn detect_result_usage_limit(&self, result: &ClaudeResult, is_stale: bool) -> Option<String> {
        claude::detect_usage_limit(&result.response)
            .or_else(|| {
                if is_stale {
                    return None;
                }
                self.claude
                    .running_logs
                    .get(&result.cue_id)
                    .and_then(|(log, _)| claude::detect_usage_limit(log))
            })
            .map(|s| s.to_string())
    }

    /// Route the result to the appropriate handler. Returns `true` if the
    /// result represents an error condition.
    fn dispatch_result(&mut self, result: &ClaudeResult, usage_limit_msg: &Option<String>) -> bool {
        if let Some(ref error) = result.error {
            self.handle_run_error(result, error);
            true
        } else if let Some(limit_line) = usage_limit_msg {
            self.handle_rate_limit(result, limit_line);
            true
        } else {
            let imported = self.extract_imported_cues(result);
            if !imported.is_empty() {
                self.handle_import_result(result, &imported);
                false
            } else if result.diff.is_some() {
                self.handle_run_with_diff(result);
                false
            } else {
                self.handle_run_no_changes(result);
                false
            }
        }
    }

    /// Post-processing after a successful (non-error) run: refresh git state
    /// and open tabs. Queued follow-ups are dispatched separately, *after*
    /// `flush_and_clear_tracking`, so the new run's tracking state survives —
    /// see `process_single_result`.
    fn handle_successful_run(&mut self, result: &ClaudeResult) {
        let plan_path = self
            .claude
            .running_logs
            .get(&result.cue_id)
            .and_then(|(log, _)| claude::extract_plan_path(log));
        self.db_log(
            "store plan path",
            self.db
                .update_cue_plan_path(result.cue_id, plan_path.as_deref()),
        );

        let _ = self.reload_open_tabs_and_notify_lsp();
        self.reload_git_info();
        self.reload_commit_history();
    }

    /// Finalize a stale execution in the DB without touching live state.
    fn complete_stale_execution(&self, result: &ClaudeResult) {
        if let Some(ref error) = result.error {
            self.db_log(
                "record stale execution failure",
                self.db.fail_execution(result.exec_id, error),
            );
        } else {
            self.db_log(
                "record stale execution result",
                self.db.complete_execution(
                    result.exec_id,
                    &result.response,
                    result.diff.as_deref(),
                    result.metrics.cost_usd,
                    Some(result.metrics.duration_ms),
                    result.metrics.num_turns,
                ),
            );
        }
    }

    /// Emit telemetry for a completed execution (unless it errored or hit a
    /// usage limit).
    fn emit_completion_telemetry(
        &self,
        result: &ClaudeResult,
        is_stale: bool,
        usage_limit_msg: &Option<String>,
    ) {
        if result.error.is_some() || usage_limit_msg.is_some() {
            return;
        }
        let provider_name = if !is_stale {
            self.claude
                .running_logs
                .get(&result.cue_id)
                .map(|(_, p)| p.display_name().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        } else {
            "unknown".to_string()
        };
        telemetry::emit_execution_completed(&telemetry::ExecutionCompleted {
            project: &self.project_name(),
            cue_id: result.cue_id,
            provider: &provider_name,
            cost_usd: result.metrics.cost_usd,
            duration_ms: result.metrics.duration_ms,
            num_turns: result.metrics.num_turns,
            input_tokens: result.metrics.input_tokens,
            output_tokens: result.metrics.output_tokens,
            has_diff: result.diff.is_some(),
        });
    }

    /// Reload the conversation history panel if it is showing this cue.
    fn refresh_conversation_history(&mut self, cue_id: i64) {
        if self.claude.show_log == Some(cue_id)
            && let Ok(execs) = self.db.get_all_executions(cue_id)
        {
            self.claude.conversation_history = execs;
        }
    }

    /// If there are queued follow-up prompts for this cue, pop the first one
    /// and trigger a reply run automatically.
    fn try_dispatch_follow_up(&mut self, cue_id: i64) {
        let next = self.follow_up_queue.get_mut(&cue_id).and_then(|queue| {
            if queue.is_empty() {
                None
            } else {
                Some(queue.remove(0))
            }
        });
        if let Some(follow_up_text) = next {
            // Clean up empty queue entry
            if self
                .follow_up_queue
                .get(&cue_id)
                .map(|q| q.is_empty())
                .unwrap_or(false)
            {
                self.follow_up_queue.remove(&cue_id);
            }
            let remaining = self
                .follow_up_queue
                .get(&cue_id)
                .map(|v| v.len())
                .unwrap_or(0);
            let msg = if remaining > 0 {
                format!("Auto-sending follow-up ({} more queued)", remaining)
            } else {
                "Auto-sending follow-up".to_string()
            };
            self.log_cue_activity(cue_id, &msg);
            self.trigger_claude_reply(cue_id, &follow_up_text, &[]);
        }
    }

    fn handle_run_error(&mut self, result: &ClaudeResult, error: &str) {
        let preview = self.cue_preview(result.cue_id);
        let provider_name = self
            .claude
            .running_logs
            .get(&result.cue_id)
            .map(|(_, p)| p.display_name().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        self.set_status_message(format!("{provider_name} error for \"{preview}\": {error}"));
        // Reported after the run error above: if the DB is also failing, that
        // is the more actionable message — the cue would otherwise be shown as
        // running forever.
        self.db_report(
            "record execution failure",
            self.db.fail_execution(result.exec_id, error),
        );
        self.db_report(
            "move cue back to Inbox",
            self.db.update_cue_status(result.cue_id, CueStatus::Inbox),
        );
        self.log_cue_activity(result.cue_id, "Run failed");
        telemetry::emit_execution_failed(
            &self.project_name(),
            result.cue_id,
            &provider_name,
            error,
        );
    }

    fn handle_rate_limit(&mut self, result: &ClaudeResult, limit_line: &str) {
        let preview = self.cue_preview(result.cue_id);
        self.set_status_message(format!("Rate limited: \"{}\" — {}", preview, limit_line));
        self.db_report(
            "record execution failure",
            self.db.fail_execution(result.exec_id, limit_line),
        );
        self.db_report(
            "move cue back to Inbox",
            self.db.update_cue_status(result.cue_id, CueStatus::Inbox),
        );
        let activity = format!("Rate limited — {}", limit_line);
        self.log_cue_activity(result.cue_id, &activity);
        self.cue_warnings
            .insert(result.cue_id, limit_line.to_string());
        telemetry::emit_execution_rate_limited(&self.project_name(), result.cue_id, limit_line);
    }

    fn handle_run_with_diff(&mut self, result: &ClaudeResult) {
        self.claude.auto_continue_count.remove(&result.cue_id);
        let Some(diff) = result.diff.as_ref() else {
            return;
        };
        let (m_cost, m_dur, m_turns) = (
            result.metrics.cost_usd,
            Some(result.metrics.duration_ms),
            result.metrics.num_turns,
        );
        self.db_report(
            "record execution result",
            self.db.complete_execution(
                result.exec_id,
                &result.response,
                Some(diff),
                m_cost,
                m_dur,
                m_turns,
            ),
        );
        self.db_report(
            "move cue to Review",
            self.db.update_cue_status(result.cue_id, CueStatus::Review),
        );
        self.log_cue_activity(result.cue_id, "Run completed — review ready");

        // Claude can make changes AND still end by asking the user something
        // (e.g. "I updated X — should I also do Y?"). Surface that so the user
        // can answer instead of the question being lost behind the diff; on the
        // PTY this is the only signal the user gets that Claude is blocked.
        if self.run_has_question(result) {
            // Claude is blocked waiting for an answer — flag it and stop here.
            // Skipping the auto-commit/AfterRun path keeps the in-progress work
            // from being committed before the user resolves the question.
            self.flag_pending_question(result.cue_id);
            return;
        }
        self.db_log(
            "clear pending-question flag",
            self.db.update_cue_has_question(result.cue_id, false),
        );
        self.notify_review_ready(result.cue_id);

        let cue_prompt = self
            .cues
            .iter()
            .find(|c| c.id == result.cue_id)
            .map(|c| c.text.clone())
            .unwrap_or_default();
        if !self.dialogs.is_open(super::DialogId::Settings) {
            self.reload_settings_from_disk();
        }
        let agents_started =
            self.trigger_agents_for(&AgentTrigger::AfterRun, Some(result.cue_id), &cue_prompt);

        // Auto-commit only when the cue is still part of an actively executing
        // workflow. The persisted `auto_commit` flag is only cleared on
        // cancellation, so a completed workflow cue keeps it set; without this
        // gate, a later manual rerun of that cue would still be silently
        // auto-committed even though it is no longer driven by a workflow.
        let cue_auto_commit = self
            .cues
            .iter()
            .any(|c| c.id == result.cue_id && c.auto_commit)
            && self.is_cue_in_active_workflow(result.cue_id);
        if cue_auto_commit {
            if self.claude.show_log == Some(result.cue_id) {
                // User is watching the log — defer so they can review.
                // If they close the log without explicitly accepting,
                // the auto-commit is skipped.
                self.pending_auto_commits.push(result.cue_id);
                self.user_reviewed_auto_commits.insert(result.cue_id);
            } else if agents_started > 0 {
                self.pending_auto_commits.push(result.cue_id);
            } else {
                self.process_commit_review_auto(result.cue_id);
            }
        }
    }

    /// Check response and running log for structured import data.
    fn extract_imported_cues(&self, result: &ClaudeResult) -> Vec<claude::ImportedCue> {
        claude::parse_import_cues(&result.response)
            .or_else(|| {
                self.claude
                    .running_logs
                    .get(&result.cue_id)
                    .and_then(|(log, _)| claude::parse_import_cues(log))
            })
            .unwrap_or_default()
    }

    fn handle_import_result(&mut self, result: &ClaudeResult, imported: &[claude::ImportedCue]) {
        let cue_text = self
            .cues
            .iter()
            .find(|c| c.id == result.cue_id)
            .map(|c| c.text.clone())
            .unwrap_or_default();
        let source_label = claude::extract_pr_label(&cue_text);

        let mut count = 0;
        let mut failed = 0;
        for item in imported {
            if item.text.trim().is_empty() {
                continue;
            }
            let source_ref = format!("import:{}", item.id);
            if self
                .db
                .cue_exists_by_source_ref(&source_ref)
                .unwrap_or(false)
            {
                continue;
            }
            if self.db_log(
                "create imported cue",
                self.db.insert_cue_from_source(
                    &item.text,
                    &source_label,
                    "",
                    &source_ref,
                    &item.file_path,
                    item.line_number,
                ),
            ) {
                count += 1;
            } else {
                failed += 1;
            }
        }

        let (m_cost, m_dur, m_turns) = (
            result.metrics.cost_usd,
            Some(result.metrics.duration_ms),
            result.metrics.num_turns,
        );
        self.db_report(
            "record execution result",
            self.db.complete_execution(
                result.exec_id,
                &result.response,
                None,
                m_cost,
                m_dur,
                m_turns,
            ),
        );
        let moved = self.db_report(
            "move cue to Done",
            self.db.update_cue_status(result.cue_id, CueStatus::Done),
        );
        let activity = format!("Imported {} cue(s)", count);
        self.log_cue_activity(result.cue_id, &activity);
        if failed > 0 {
            self.set_status_message(format!(
                "Imported {} cue(s) from {} — {} could not be saved",
                count, source_label, failed
            ));
        } else if moved {
            self.set_status_message(format!("Imported {} cue(s) from {}", count, source_label));
        }
    }

    /// Detect whether a completed run ended with a question for the user.
    ///
    /// Checks the captured response first, then falls back to the live
    /// PTY/provider log: under the Claude PTY the trailing question is rendered
    /// to the interactive TUI and may only survive in the streamed screen
    /// output, not in `result.response`. Without this fallback Claude can ask a
    /// question and silently wait while the cue looks finished.
    fn run_has_question(&self, result: &ClaudeResult) -> bool {
        if claude::response_has_question(&result.response) {
            return true;
        }
        self.claude
            .running_logs
            .get(&result.cue_id)
            .map(|(log, _)| claude::response_has_question(log))
            .unwrap_or(false)
    }

    /// Flag that Claude is blocked on a question for `cue_id`: mark the cue,
    /// open its reply box so the user can answer immediately, notify, and log.
    fn flag_pending_question(&mut self, cue_id: i64) {
        self.claude.auto_continue_count.remove(&cue_id);
        let preview = self.cue_preview(cue_id);
        self.set_status_message(format!(
            "Claude is waiting for your answer about \"{preview}\""
        ));
        // If this flag is lost the cue looks finished while Claude is actually
        // blocked waiting, and a workflow would start dependent steps early.
        self.db_report(
            "flag pending question",
            self.db.update_cue_has_question(cue_id, true),
        );
        self.log_cue_activity(cue_id, "Run completed — question pending");
        self.reply_inputs.entry(cue_id).or_default();
        self.notify_run_complete(cue_id, true);
    }

    fn handle_run_no_changes(&mut self, result: &ClaudeResult) {
        let (m_cost, m_dur, m_turns) = (
            result.metrics.cost_usd,
            Some(result.metrics.duration_ms),
            result.metrics.num_turns,
        );
        self.db_report(
            "record execution result",
            self.db.complete_execution(
                result.exec_id,
                &result.response,
                None,
                m_cost,
                m_dur,
                m_turns,
            ),
        );
        let has_question = self.run_has_question(result);
        let preview = self.cue_preview(result.cue_id);
        if has_question {
            self.db_report(
                "move cue to Done",
                self.db.update_cue_status(result.cue_id, CueStatus::Done),
            );
            self.flag_pending_question(result.cue_id);
        } else if self.should_auto_continue(result.cue_id) {
            let count = self
                .claude
                .auto_continue_count
                .entry(result.cue_id)
                .or_insert(0);
            *count += 1;
            let n = *count;
            let max = self.settings.auto_continue_max;
            self.set_status_message(format!("Auto-continuing \"{}\" ({}/{})", preview, n, max,));
            self.db_log(
                "clear pending-question flag",
                self.db.update_cue_has_question(result.cue_id, false),
            );
            self.log_cue_activity(
                result.cue_id,
                &format!("Stopped early — auto-continue {}/{}", n, max),
            );
            self.pending_auto_continues.push(result.cue_id);
        } else {
            self.claude.auto_continue_count.remove(&result.cue_id);
            self.set_status_message(format!(
                "Claude completed but no file changes detected for \"{}\"",
                preview
            ));
            self.db_log(
                "clear pending-question flag",
                self.db.update_cue_has_question(result.cue_id, false),
            );
            self.db_report(
                "move cue to Done",
                self.db.update_cue_status(result.cue_id, CueStatus::Done),
            );
            self.log_cue_activity(result.cue_id, "Run completed — no changes");
        }
    }

    /// Check whether a stopped-early run should be auto-continued.
    fn should_auto_continue(&self, cue_id: i64) -> bool {
        if !self.settings.auto_continue {
            return false;
        }
        let count = self
            .claude
            .auto_continue_count
            .get(&cue_id)
            .copied()
            .unwrap_or(0);
        if count >= self.settings.auto_continue_max {
            return false;
        }
        let log = self
            .claude
            .running_logs
            .get(&cue_id)
            .map(|(l, _)| l.as_str())
            .unwrap_or("");
        claude::detect_stopped_early(log)
    }

    /// Maximum size (in bytes) of a single cue's running log buffer.
    /// Once exceeded the oldest half is discarded to keep memory bounded.
    const RUNNING_LOG_CAP: usize = 2 * 1024 * 1024; // 2 MiB

    /// Drain the log channel, appending text to the per-cue log buffers.
    pub(super) fn drain_log_channel(&mut self) {
        for update in self.claude.log_rx.try_iter() {
            let cue_id = update.cue_id;

            // Heartbeat accounting: each `\0` is a tick-only sentinel
            // (emitted by the PTY consumer for filtered empty lines) and
            // every `\n` in the visible text marks one PTY/provider line.
            // Count both, then strip sentinels before storing the text.
            let sentinel_beats = update.text.matches('\0').count();
            let line_beats = update.text.matches('\n').count();
            let total_beats = sentinel_beats + line_beats;
            let cleaned_owned: Option<String> = if sentinel_beats > 0 {
                Some(update.text.replace('\0', ""))
            } else {
                None
            };
            let visible_text: &str = cleaned_owned.as_deref().unwrap_or(update.text.as_str());

            let buf = &mut self
                .claude
                .running_logs
                .entry(cue_id)
                .or_insert_with(|| (String::new(), update.provider.clone()))
                .0;
            buf.push_str(visible_text);
            // Trim to cap: keep the most recent half when the limit is exceeded.
            if buf.len() > Self::RUNNING_LOG_CAP {
                let keep_from = buf.len() - Self::RUNNING_LOG_CAP / 2;
                let start = buf.ceil_char_boundary(keep_from);
                let trimmed = buf[start..].to_string();
                *buf = format!("… (log truncated) …\n{}", trimmed);
            }

            // Record a heartbeat tick per PTY/provider line so the strip
            // pulses for each line — including the empty ones we filter
            // from the visible output.
            if total_beats > 0 {
                let now = Instant::now();
                self.claude.last_message_times.insert(cue_id, now);
                let beats = self.claude.log_heartbeats.entry(cue_id).or_default();
                for _ in 0..total_beats {
                    beats.push_back(now);
                }
                let cutoff = now - Self::HEARTBEAT_WINDOW;
                while beats.front().is_some_and(|t| *t < cutoff) {
                    beats.pop_front();
                }
            }
        }
    }

    /// Sliding window of activity shown by the heartbeat strip.
    pub(super) const HEARTBEAT_WINDOW: Duration = Duration::from_secs(10);

    /// Periodically flush local running logs to DB (for cross-instance visibility)
    /// and reload remote running logs from DB (for viewing another instance's run).
    pub(super) fn sync_running_logs(&mut self) {
        self.drain_log_channel();
        self.flush_local_logs_to_db();
        self.reload_remote_log_if_needed();
        self.claude.last_log_flush = Instant::now();
    }

    /// Flush all local running logs to the database.
    fn flush_local_logs_to_db(&self) {
        // This runs on a timer for every live run, so failures are counted and
        // logged once per sweep rather than once per cue — a broken DB would
        // otherwise flood the log every interval.
        let mut failed = 0;
        for (&cue_id, (log_text, _)) in &self.claude.running_logs {
            if let Some(&exec_id) = self.claude.exec_ids.get(&cue_id)
                && self.db.update_execution_log(exec_id, log_text).is_err()
            {
                failed += 1;
            }
        }
        if failed > 0 {
            log::error!("[db] flush execution log failed for {} run(s)", failed);
        }
    }

    /// Reload the log from DB for the currently viewed cue if it is a remote run.
    fn reload_remote_log_if_needed(&mut self) {
        let cue_id = match self.claude.show_log {
            Some(id) => id,
            None => return,
        };
        if self.claude.exec_ids.contains_key(&cue_id) {
            return; // Local run — already tracking via channel
        }
        let is_running = self
            .cues
            .iter()
            .any(|c| c.id == cue_id && c.status == CueStatus::Ready);
        if !is_running {
            return;
        }
        let log_text = self.db.get_latest_execution(cue_id).ok().flatten();
        if let Some(exec) = log_text {
            let provider = exec.provider;
            if let Some(text) = exec.log {
                self.claude.running_logs.insert(cue_id, (text, provider));
            }
        }
    }

    pub(super) fn cleanup_jj_workspace(&mut self, cue_id: i64) {
        self.claude.workspace_commit_failed.remove(&cue_id);
        if let Some(ws_path) = self.claude.workspace_paths.remove(&cue_id) {
            let ws_name = self.claude.workspace_names.remove(&cue_id);
            if let Some(name) = &ws_name {
                let _ =
                    jj::jj_remove_workspace(&self.project_root, name, &self.settings.jj_cli_path);
            }
            if ws_path.is_dir() {
                let _ = std::fs::remove_dir_all(&ws_path);
            }
        }
    }

    fn notify_review_ready(&self, cue_id: i64) {
        self.notify_run_complete(cue_id, false);
    }

    /// Play the completion sound and (optionally) post a desktop notification.
    /// When `question` is set, the popup makes clear Claude is blocked waiting
    /// for the user to answer, rather than just announcing a ready review.
    fn notify_run_complete(&self, cue_id: i64, question: bool) {
        if self.settings.notify_sound {
            std::thread::spawn(|| {
                // Play the embedded Glass sound via `afplay` (a separate process).
                // We CANNOT use NSSound or any Apple audio framework in-process —
                // they all load CoreAudio/MediaToolbox which triggers the macOS
                // TCC "would like to access Apple Music" permission dialog.
                use std::io::Write;
                static GLASS_AIFF: &[u8] = include_bytes!("../../../assets/Glass.aiff");
                let tmp = std::env::temp_dir().join("dirigent_glass.aiff");
                if !tmp.exists()
                    && let Ok(mut f) = std::fs::File::create(&tmp)
                {
                    let _ = f.write_all(GLASS_AIFF);
                }
                let _ = std::process::Command::new("/usr/bin/afplay")
                    .arg(&tmp)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            });
        }
        if self.settings.notify_popup {
            let preview = self.cue_preview(cue_id);
            let project_name = self
                .project_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            let subtitle = if question {
                format!("{project_name} — Claude is waiting for your answer")
            } else {
                project_name
            };
            send_macos_notification("Dirigent", &subtitle, &preview);
        }
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn herdr_gets_the_same_session_args_as_claude() {
        for provider in [CliProvider::Claude, CliProvider::Herdr] {
            assert_eq!(
                session_continuity_args(&provider, None, || "new-id".to_string()),
                Some((
                    vec!["--session-id".to_string(), "new-id".to_string()],
                    "new-id".to_string()
                )),
                "{provider:?}"
            );
            assert_eq!(
                session_continuity_args(&provider, Some("old".to_string()), || {
                    panic!("no new id when resuming")
                }),
                Some((
                    vec!["--resume".to_string(), "old".to_string()],
                    "old".to_string()
                )),
                "{provider:?}"
            );
        }
    }

    #[test]
    fn other_providers_get_no_session_args() {
        for provider in [
            CliProvider::OpenCode,
            CliProvider::Gemini,
            CliProvider::Codex,
            CliProvider::Acp,
        ] {
            assert_eq!(
                session_continuity_args(&provider, Some("x".to_string()), String::new),
                None
            );
        }
    }
}
