use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};
use std::time::Instant;

use crate::agents::AgentTrigger;
use crate::claude;
use crate::db::{Cue, CueStatus, Execution};
use crate::git;
use crate::opencode;
use crate::settings::{self, CliProvider, CueCommand};
use crate::telemetry;

use super::notifications::send_macos_notification;
use super::tasks::TaskHandle;
use super::DirigentApp;

/// Result of a background Claude invocation.
struct ClaudeResult {
    cue_id: i64,
    exec_id: i64,
    diff: Option<String>,
    response: String,
    error: Option<String>,
    metrics: claude::RunMetrics,
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
        }
    }
}

/// Configuration extracted from Settings for a specific CLI provider.
struct ProviderConfig {
    model: String,
    cli_path: String,
    extra_args: String,
    env_vars: String,
    pre_run_script: String,
    post_run_script: String,
    skip_permissions: bool,
}

/// Resolve a `[command]` prefix from cue text, returning the effective prompt text
/// and the matched command config (if any).
fn resolve_command_prefix(cue_text: &str, commands: &[CueCommand]) -> (String, Option<CueCommand>) {
    let parsed = claude::parse_command_prefix(cue_text);
    let (cmd_name, rest) = match parsed {
        Some((name, rest)) => (name, rest),
        None => return (cue_text.to_string(), None),
    };
    match commands.iter().find(|c| c.name == cmd_name) {
        Some(cmd) => {
            let expanded = cmd.prompt.replace("{task}", rest);
            (expanded, Some(cmd.clone()))
        }
        None => (cue_text.to_string(), None),
    }
}

/// For PR-sourced cues, append a hint so the AI can fetch more context.
/// The hint is added at prompt time (not stored in cue text) to avoid
/// triggering spurious text-changed updates on existing cues.
fn build_pr_hint_text(text: &str, source_ref: Option<&str>) -> String {
    if let Some(sref) = source_ref {
        if let Some(pr_num) = sref
            .strip_prefix("pr")
            .and_then(|s| s.split(':').next())
            .and_then(|n| n.parse::<u64>().ok())
        {
            return format!(
                "{}\n\n[Hint: use `gh pr view {} --comments` to read the full PR discussion for additional context.]",
                text, pr_num,
            );
        }
    }
    text.to_string()
}

/// Build provider-specific configuration from settings, with optional command overrides.
fn build_provider_config(
    settings: &settings::Settings,
    provider: &CliProvider,
    matched_command: &Option<CueCommand>,
) -> ProviderConfig {
    let (model, cli_path, mut extra_args, env_vars, default_pre, default_post) = match provider {
        CliProvider::Claude => (
            settings.claude_model.clone(),
            settings.claude_cli_path.clone(),
            settings.claude_extra_args.clone(),
            settings.claude_env_vars.clone(),
            settings.claude_pre_run_script.clone(),
            settings.claude_post_run_script.clone(),
        ),
        CliProvider::OpenCode => (
            settings.opencode_model.clone(),
            settings.opencode_cli_path.clone(),
            settings.opencode_extra_args.clone(),
            settings.opencode_env_vars.clone(),
            settings.opencode_pre_run_script.clone(),
            settings.opencode_post_run_script.clone(),
        ),
    };
    let pre_run_script = matched_command
        .as_ref()
        .filter(|cmd| !cmd.pre_agent.is_empty())
        .map(|cmd| cmd.pre_agent.clone())
        .unwrap_or(default_pre);
    let post_run_script = matched_command
        .as_ref()
        .filter(|cmd| !cmd.post_agent.is_empty())
        .map(|cmd| cmd.post_agent.clone())
        .unwrap_or(default_post);
    // Append command-specific CLI args (e.g. --plan) to the provider extra_args.
    if let Some(cmd) = matched_command.as_ref() {
        let cmd_args = cmd.cli_args.trim();
        if !cmd_args.is_empty() {
            if extra_args.is_empty() {
                extra_args = cmd_args.to_string();
            } else {
                extra_args = format!("{} {}", extra_args, cmd_args);
            }
        }
    }
    ProviderConfig {
        model,
        cli_path,
        extra_args,
        env_vars,
        pre_run_script,
        post_run_script,
        skip_permissions: settings.allow_dangerous_skip_permissions,
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
}

/// Run the CLI provider on a background thread and produce a `ClaudeResult`.
fn run_provider(
    req: &RunRequest,
    on_log: impl FnMut(&str) + Send + 'static,
    cancel: Arc<AtomicBool>,
) -> ClaudeResult {
    match req.provider {
        CliProvider::Claude => run_claude_provider(req, on_log, cancel),
        CliProvider::OpenCode => run_opencode_provider(req, on_log, cancel),
    }
}

fn run_claude_provider(
    req: &RunRequest,
    on_log: impl FnMut(&str) + Send + 'static,
    cancel: Arc<AtomicBool>,
) -> ClaudeResult {
    let res = claude::invoke_claude_streaming(
        req.prompt,
        req.project_root,
        &req.config.model,
        &req.config.cli_path,
        &req.config.extra_args,
        &req.config.env_vars,
        &req.config.pre_run_script,
        &req.config.post_run_script,
        req.config.skip_permissions,
        on_log,
        cancel,
    );
    match res {
        Ok(response) => {
            let diff = if response.edited_files.is_empty() {
                claude::parse_diff_from_response(&response.stdout)
            } else {
                git::get_working_diff(req.project_root, &response.edited_files)
            };
            ClaudeResult {
                cue_id: req.cue_id,
                exec_id: req.exec_id,
                diff,
                response: response.stdout,
                error: None,
                metrics: response.metrics,
            }
        }
        Err(e) => ClaudeResult {
            cue_id: req.cue_id,
            exec_id: req.exec_id,
            diff: None,
            response: String::new(),
            error: Some(e.to_string()),
            metrics: claude::RunMetrics::default(),
        },
    }
}

/// Compute a working-tree diff scoped to only files that were *newly* changed
/// since a pre-run baseline snapshot. This avoids capturing pre-existing local
/// modifications in the fallback diff path.
fn scoped_working_diff(
    project_root: &Path,
    baseline_dirty: &std::collections::HashSet<String>,
) -> Option<String> {
    let current_dirty = git::get_dirty_files(project_root);
    let new_files: Vec<String> = current_dirty
        .keys()
        .filter(|f| !baseline_dirty.contains(f.as_str()))
        .cloned()
        .collect();
    if new_files.is_empty() {
        return None;
    }
    git::get_working_diff(project_root, &new_files)
}

fn run_opencode_provider(
    req: &RunRequest,
    on_log: impl FnMut(&str) + Send + 'static,
    cancel: Arc<AtomicBool>,
) -> ClaudeResult {
    // Snapshot dirty files before the run so we can scope the fallback diff
    // to only files newly changed by this run, avoiding the risk of
    // committing pre-existing local modifications.
    let baseline_dirty: std::collections::HashSet<String> = git::get_dirty_files(req.project_root)
        .keys()
        .cloned()
        .collect();

    let opencode_config = opencode::OpenCodeRunConfig {
        model: &req.config.model,
        cli_path: &req.config.cli_path,
        extra_args: &req.config.extra_args,
        env_vars: &req.config.env_vars,
        pre_run_script: &req.config.pre_run_script,
        post_run_script: &req.config.post_run_script,
    };
    let res = opencode::invoke_opencode_streaming(
        req.prompt,
        req.project_root,
        &opencode_config,
        on_log,
        cancel,
    );
    match res {
        Ok(response) => {
            let diff = if response.edited_files.is_empty() {
                opencode::parse_diff_from_response(&response.stdout)
                    .or_else(|| scoped_working_diff(req.project_root, &baseline_dirty))
            } else {
                git::get_working_diff(req.project_root, &response.edited_files)
                    .or_else(|| opencode::parse_diff_from_response(&response.stdout))
            };
            let metrics = claude::RunMetrics {
                cost_usd: response.cost_usd.unwrap_or(0.0),
                duration_ms: response.duration_ms.unwrap_or(0),
                num_turns: response.num_turns.unwrap_or(0),
                ..claude::RunMetrics::default()
            };
            ClaudeResult {
                cue_id: req.cue_id,
                exec_id: req.exec_id,
                diff,
                response: response.stdout,
                error: None,
                metrics,
            }
        }
        Err(e) => ClaudeResult {
            cue_id: req.cue_id,
            exec_id: req.exec_id,
            diff: None,
            response: String::new(),
            error: Some(e.to_string()),
            metrics: claude::RunMetrics::default(),
        },
    }
}

impl DirigentApp {
    /// Build the initial prompt for a cue, gathering auto-context for Claude provider.
    fn build_initial_prompt(&self, effective_text: &str, cue: &Cue) -> String {
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
        let project_root = self.project_root.clone();
        let claude_tx = self.claude.tx.clone();
        let log_tx = self.claude.log_tx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_thread = Arc::clone(&cancel);
        let provider_for_log = provider.clone();

        let join_handle = std::thread::spawn(move || {
            let on_log = move |text: &str| {
                let _ = log_tx.send(LogUpdate {
                    cue_id,
                    text: super::util::strip_ansi(text),
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

    /// Insert a new execution record, emit telemetry, and spawn the provider thread.
    /// Insert a new execution record and spawn the provider thread.
    /// Returns `true` on success, `false` if the execution record could not be
    /// created (the caller should skip any post-spawn cleanup in that case).
    fn insert_exec_and_spawn(
        &mut self,
        cue_id: i64,
        prompt: String,
        provider: CliProvider,
        matched_command: &Option<CueCommand>,
    ) -> bool {
        let exec_id = match self.db.insert_execution(cue_id, &prompt, &provider) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("Failed to create execution record for cue {cue_id}: {e}");
                self.claude.running_logs.remove(&cue_id);
                self.claude.start_times.remove(&cue_id);
                self.set_status_message(format!("Failed to start run: {e}"));
                self.reload_cues();
                return false;
            }
        };

        self.claude.exec_ids.insert(cue_id, exec_id);
        if let Ok(execs) = self.db.get_all_executions(cue_id) {
            self.claude.conversation_history = execs;
        }

        let config = build_provider_config(&self.settings, &provider, matched_command);

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
        if let Err(e) = settings::sync_home_guard_hook(
            &self.project_root,
            self.settings.allow_home_folder_access,
        ) {
            eprintln!("Failed to sync home guard hook: {e:#}");
            return;
        }

        // Start tracking immediately so the timer appears in the UI while
        // we build the prompt (which may involve blocking I/O for auto-context).
        let provider = self.settings.cli_provider.clone();
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
                let _ = self.db.update_cue_status(cue_id, CueStatus::Inbox);
                self.set_status_message("Failed to start run: cue not found".to_string());
                self.reload_cues();
                return;
            }
        };

        let (effective_text, matched_command) =
            resolve_command_prefix(&cue.text, &self.settings.commands);
        let prompt = self.build_initial_prompt(&effective_text, &cue);

        if !self.insert_exec_and_spawn(cue_id, prompt, provider, &matched_command) {
            let _ = self.db.update_cue_status(cue_id, CueStatus::Inbox);
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
            eprintln!("Failed to sync home guard hook: {e:#}");
            return;
        }

        let provider = self.settings.cli_provider.clone();

        let cue = match self.cues.iter().find(|c| c.id == cue_id) {
            Some(c) => c.clone(),
            None => {
                self.set_status_message("Failed to start reply: cue not found".to_string());
                return;
            }
        };

        let (raw_text, matched_command) =
            resolve_command_prefix(&cue.text, &self.settings.commands);
        let original_text = build_pr_hint_text(&raw_text, cue.source_ref.as_deref());

        let previous_diff = self
            .db
            .get_latest_execution(cue_id)
            .ok()
            .flatten()
            .and_then(|e| e.diff)
            .unwrap_or_default();

        let mut all_images = cue.attached_images.clone();
        all_images.extend_from_slice(reply_images);

        // Both providers use the same reply prompt structure.
        let prompt = claude::build_reply_prompt(
            &original_text,
            &cue.file_path,
            cue.line_number,
            cue.line_number_end,
            &previous_diff,
            reply,
            &all_images,
            Some(&self.project_root),
        );

        let _ = self.db.update_cue_status(cue_id, CueStatus::Ready);
        self.claude.expand_running = true;
        self.reload_cues();

        // Start tracking immediately so the timer appears in the UI.
        self.claude
            .running_logs
            .insert(cue_id, (String::new(), provider.clone()));
        self.claude.start_times.insert(cue_id, Instant::now());

        let started = self.insert_exec_and_spawn(cue_id, prompt, provider, &matched_command);

        if started {
            if self.diff_review.as_ref().map(|r| r.cue_id) == Some(cue_id) {
                self.diff_review = None;
            }
        } else {
            // Restore the cue to Review (its state before we attempted the reply).
            let _ = self.db.update_cue_status(cue_id, CueStatus::Review);
            self.reload_cues();
        }
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

        if !is_stale {
            if let Some((log_text, _)) = self.claude.running_logs.get(&result.cue_id) {
                let _ = self.db.update_execution_log(result.exec_id, log_text);
            }
            self.claude.exec_ids.remove(&result.cue_id);
            self.claude.start_times.remove(&result.cue_id);
        }

        let _ = self.db.update_execution_metrics(
            result.exec_id,
            result.metrics.cost_usd,
            result.metrics.duration_ms,
            result.metrics.num_turns,
            result.metrics.input_tokens,
            result.metrics.output_tokens,
        );

        // Detect usage-limit messages in the response or log before deciding
        // which handler to use.  When a hard rate/usage limit is hit, Claude
        // exits without changes and we must NOT treat it as "no changes needed".
        let usage_limit_msg: Option<String> = claude::detect_usage_limit(&result.response)
            .or_else(|| {
                self.claude
                    .running_logs
                    .get(&result.cue_id)
                    .and_then(|(log, _)| claude::detect_usage_limit(log))
            })
            .map(|s| s.to_string());

        // Emit telemetry for every completed execution (including stale ones),
        // but only after ruling out rate limits — otherwise the same response
        // would be counted as both completed and rate-limited.
        let provider_name = self
            .claude
            .running_logs
            .get(&result.cue_id)
            .map(|(_, p)| p.display_name().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        if result.error.is_none() && usage_limit_msg.is_none() {
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

        if is_stale {
            // Stale result — just mark the old execution in the DB, don't change cue status.
            if let Some(ref error) = result.error {
                let _ = self.db.fail_execution(result.exec_id, error);
            } else {
                let _ = self.db.complete_execution(
                    result.exec_id,
                    &result.response,
                    result.diff.as_deref(),
                    Some(result.metrics.cost_usd),
                    Some(result.metrics.duration_ms),
                    Some(result.metrics.num_turns),
                );
            }
            return;
        }

        let is_error = if let Some(ref error) = result.error {
            self.handle_run_error(&result, error);
            true
        } else if let Some(ref limit_line) = usage_limit_msg {
            self.handle_rate_limit(&result, limit_line);
            true
        } else if result.diff.is_some() {
            self.handle_run_with_diff(&result);
            false
        } else {
            self.handle_run_no_changes(&result);
            false
        };

        // After every successful run, refresh git state and open tabs so that
        // commits made by Claude Code (or any other file changes) are visible
        // immediately in the UI — git log, dirty-file markers, tab contents.
        if !is_error {
            // Detect Claude Code plan (ExitPlanMode) in the log output.
            let plan_path = self
                .claude
                .running_logs
                .get(&result.cue_id)
                .and_then(|(log, _)| claude::extract_plan_path(log));
            let _ = self
                .db
                .update_cue_plan_path(result.cue_id, plan_path.as_deref());

            self.refresh_open_tabs();
            self.reload_git_info();
            self.reload_commit_history();
            self.try_dispatch_follow_up(result.cue_id);
        }

        self.refresh_conversation_history(result.cue_id);
        self.reload_cues();

        // If this cue is part of a workflow, check if the step is complete.
        if !is_stale {
            self.on_workflow_cue_completed(result.cue_id);
        }
    }

    /// Reload the conversation history panel if it is showing this cue.
    fn refresh_conversation_history(&mut self, cue_id: i64) {
        if self.claude.show_log == Some(cue_id) {
            if let Ok(execs) = self.db.get_all_executions(cue_id) {
                self.claude.conversation_history = execs;
            }
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
            let _ = self.db.log_activity(cue_id, &msg);
            self.trigger_claude_reply(cue_id, &follow_up_text, &[]);
        }
    }

    fn handle_run_error(&mut self, result: &ClaudeResult, error: &str) {
        let preview = self.cue_preview(result.cue_id);
        self.set_status_message(format!("Claude error for \"{}\": {}", preview, error));
        let _ = self.db.fail_execution(result.exec_id, error);
        let _ = self.db.update_cue_status(result.cue_id, CueStatus::Inbox);
        let _ = self.db.log_activity(result.cue_id, "Run failed");
        let provider_name = self
            .claude
            .running_logs
            .get(&result.cue_id)
            .map(|(_, p)| p.display_name().to_string())
            .unwrap_or_else(|| "unknown".to_string());
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
        let _ = self.db.fail_execution(result.exec_id, limit_line);
        let _ = self.db.update_cue_status(result.cue_id, CueStatus::Inbox);
        let activity = format!("Rate limited — {}", limit_line);
        let _ = self.db.log_activity(result.cue_id, &activity);
        self.cue_warnings
            .insert(result.cue_id, limit_line.to_string());
        telemetry::emit_execution_rate_limited(&self.project_name(), result.cue_id, limit_line);
    }

    fn handle_run_with_diff(&mut self, result: &ClaudeResult) {
        let Some(diff) = result.diff.as_ref() else {
            return;
        };
        let (m_cost, m_dur, m_turns) = (
            Some(result.metrics.cost_usd),
            Some(result.metrics.duration_ms),
            Some(result.metrics.num_turns),
        );
        let _ = self.db.complete_execution(
            result.exec_id,
            &result.response,
            Some(diff),
            m_cost,
            m_dur,
            m_turns,
        );
        let _ = self.db.update_cue_status(result.cue_id, CueStatus::Review);
        let _ = self
            .db
            .log_activity(result.cue_id, "Run completed — review ready");
        self.notify_review_ready(result.cue_id);

        let cue_prompt = self
            .cues
            .iter()
            .find(|c| c.id == result.cue_id)
            .map(|c| c.text.clone())
            .unwrap_or_default();
        self.trigger_agents_for(&AgentTrigger::AfterRun, Some(result.cue_id), &cue_prompt);
    }

    fn handle_run_no_changes(&mut self, result: &ClaudeResult) {
        let (m_cost, m_dur, m_turns) = (
            Some(result.metrics.cost_usd),
            Some(result.metrics.duration_ms),
            Some(result.metrics.num_turns),
        );
        let _ = self.db.complete_execution(
            result.exec_id,
            &result.response,
            None,
            m_cost,
            m_dur,
            m_turns,
        );
        let preview = self.cue_preview(result.cue_id);
        self.set_status_message(format!(
            "Claude completed but no file changes detected for \"{}\"",
            preview
        ));
        // Moving to Done even when Claude produced no file changes is intentional:
        // the AI examined the cue, decided nothing needed changing, and that
        // counts as "addressed."  The cue should not stay in Review or loop
        // back to Inbox — the human can always re-open it from Done if they
        // disagree with the AI's assessment.
        let _ = self.db.update_cue_status(result.cue_id, CueStatus::Done);
        let _ = self
            .db
            .log_activity(result.cue_id, "Run completed — no changes");
    }

    /// Reload all open tabs so the user sees file changes made by the CLI.
    fn refresh_open_tabs(&mut self) {
        for tab in &mut self.viewer.tabs {
            let content = match std::fs::read_to_string(&tab.file_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if tab.markdown_blocks.is_some() {
                tab.markdown_blocks = Some(super::markdown_parser::parse_markdown(&content));
            }
            tab.content = content.lines().map(String::from).collect();
            let ext = tab
                .file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            tab.symbols = super::symbols::parse_symbols(&tab.content, ext);
        }
    }

    /// Drain the log channel, appending text to the per-cue log buffers.
    pub(super) fn drain_log_channel(&mut self) {
        for update in self.claude.log_rx.try_iter() {
            self.claude
                .running_logs
                .entry(update.cue_id)
                .or_insert_with(|| (String::new(), update.provider.clone()))
                .0
                .push_str(&update.text);
        }
    }

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
        for (&cue_id, (log_text, _)) in &self.claude.running_logs {
            if let Some(&exec_id) = self.claude.exec_ids.get(&cue_id) {
                let _ = self.db.update_execution_log(exec_id, log_text);
            }
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
        let log_text = self
            .db
            .get_latest_execution(cue_id)
            .ok()
            .flatten()
            .and_then(|e| e.log);
        if let Some(text) = log_text {
            self.claude
                .running_logs
                .insert(cue_id, (text, CliProvider::Claude));
        }
    }

    fn notify_review_ready(&self, cue_id: i64) {
        if self.settings.notify_sound {
            std::thread::spawn(|| {
                // Play the embedded Glass sound via `afplay` (a separate process).
                // We CANNOT use NSSound or any Apple audio framework in-process —
                // they all load CoreAudio/MediaToolbox which triggers the macOS
                // TCC "would like to access Apple Music" permission dialog.
                use std::io::Write;
                static GLASS_AIFF: &[u8] = include_bytes!("../../assets/Glass.aiff");
                let tmp = std::env::temp_dir().join("dirigent_glass.aiff");
                if !tmp.exists() {
                    if let Ok(mut f) = std::fs::File::create(&tmp) {
                        let _ = f.write_all(GLASS_AIFF);
                    }
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
            send_macos_notification("Dirigent", &project_name, &preview);
        }
    }
}
