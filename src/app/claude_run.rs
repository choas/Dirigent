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

/// Build provider-specific configuration from settings, with optional command overrides.
fn build_provider_config(
    settings: &settings::Settings,
    provider: &CliProvider,
    matched_command: &Option<CueCommand>,
) -> ProviderConfig {
    let (model, cli_path, extra_args, env_vars, default_pre, default_post) = match provider {
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
    ProviderConfig {
        model,
        cli_path,
        extra_args,
        env_vars,
        pre_run_script,
        post_run_script,
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
    on_log: impl FnMut(&str) + Send,
    cancel: Arc<AtomicBool>,
) -> ClaudeResult {
    match req.provider {
        CliProvider::Claude => run_claude_provider(req, on_log, cancel),
        CliProvider::OpenCode => run_opencode_provider(req, on_log, cancel),
    }
}

fn run_claude_provider(
    req: &RunRequest,
    on_log: impl FnMut(&str) + Send,
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

fn run_opencode_provider(
    req: &RunRequest,
    on_log: impl FnMut(&str) + Send,
    cancel: Arc<AtomicBool>,
) -> ClaudeResult {
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
            } else {
                git::get_working_diff(req.project_root, &response.edited_files)
                    .or_else(|| opencode::parse_diff_from_response(&response.stdout))
            };
            ClaudeResult {
                cue_id: req.cue_id,
                exec_id: req.exec_id,
                diff,
                response: response.stdout,
                error: None,
                metrics: claude::RunMetrics::default(),
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
        let auto_context =
            if (want_file || want_diff) && self.settings.cli_provider == CliProvider::Claude {
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

        match self.settings.cli_provider {
            CliProvider::Claude => claude::build_prompt_with_auto_context(
                effective_text,
                &cue.file_path,
                cue.line_number,
                cue.line_number_end,
                &cue.attached_images,
                &auto_context,
            ),
            CliProvider::OpenCode => opencode::build_prompt(
                effective_text,
                &cue.file_path,
                cue.line_number,
                cue.line_number_end,
                &cue.attached_images,
            ),
        }
    }

    /// Initialize run-tracking state (log buffer, start time, exec id, conversation history).
    fn init_run_tracking(&mut self, cue_id: i64, exec_id: i64, provider: &CliProvider) {
        self.claude
            .running_logs
            .insert(cue_id, (String::new(), provider.clone()));
        self.claude.start_times.insert(cue_id, Instant::now());
        self.claude.exec_ids.insert(cue_id, exec_id);
        if let Ok(execs) = self.db.get_all_executions(cue_id) {
            self.claude.conversation_history = execs;
        }
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
            let on_log = |text: &str| {
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

    pub(super) fn trigger_claude(&mut self, cue_id: i64) {
        settings::sync_home_guard_hook(&self.project_root, self.settings.allow_home_folder_access);

        let cue = match self.cues.iter().find(|c| c.id == cue_id) {
            Some(c) => c.clone(),
            None => return,
        };

        let (effective_text, matched_command) =
            resolve_command_prefix(&cue.text, &self.settings.commands);
        let prompt = self.build_initial_prompt(&effective_text, &cue);

        let provider = self.settings.cli_provider.clone();
        let exec_id = self
            .db
            .insert_execution(cue_id, &prompt, &provider)
            .unwrap_or(0);

        self.init_run_tracking(cue_id, exec_id, &provider);

        let config = build_provider_config(&self.settings, &provider, &matched_command);
        let handle = self.spawn_provider_thread(cue_id, exec_id, prompt, provider, config);
        self.task_handles.push(handle);
    }

    pub(super) fn trigger_claude_reply(
        &mut self,
        cue_id: i64,
        reply: &str,
        reply_images: &[String],
    ) {
        settings::sync_home_guard_hook(&self.project_root, self.settings.allow_home_folder_access);

        let cue = match self.cues.iter().find(|c| c.id == cue_id) {
            Some(c) => c.clone(),
            None => return,
        };

        let (original_text, matched_command) =
            resolve_command_prefix(&cue.text, &self.settings.commands);

        let previous_diff = self
            .db
            .get_latest_execution(cue_id)
            .ok()
            .flatten()
            .and_then(|e| e.diff)
            .unwrap_or_default();

        let mut all_images = cue.attached_images.clone();
        all_images.extend_from_slice(reply_images);

        let prompt = match self.settings.cli_provider {
            CliProvider::Claude => claude::build_reply_prompt(
                &original_text,
                &cue.file_path,
                cue.line_number,
                cue.line_number_end,
                &previous_diff,
                reply,
                &all_images,
                Some(&self.project_root),
            ),
            CliProvider::OpenCode => opencode::build_reply_prompt(
                &original_text,
                &cue.file_path,
                cue.line_number,
                cue.line_number_end,
                &previous_diff,
                reply,
                &all_images,
            ),
        };

        let _ = self.db.update_cue_status(cue_id, CueStatus::Ready);
        self.claude.expand_running = true;
        self.reload_cues();

        let provider = self.settings.cli_provider.clone();
        let exec_id = self
            .db
            .insert_execution(cue_id, &prompt, &provider)
            .unwrap_or(0);

        self.init_run_tracking(cue_id, exec_id, &provider);

        let config = build_provider_config(&self.settings, &provider, &matched_command);
        let handle = self.spawn_provider_thread(cue_id, exec_id, prompt, provider, config);
        self.task_handles.push(handle);

        if self.diff_review.as_ref().map(|r| r.cue_id) == Some(cue_id) {
            self.diff_review = None;
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
        if let Some((log_text, _)) = self.claude.running_logs.get(&result.cue_id) {
            let _ = self.db.update_execution_log(result.exec_id, log_text);
        }
        self.claude.exec_ids.remove(&result.cue_id);
        self.claude.start_times.remove(&result.cue_id);

        let _ = self.db.update_execution_metrics(
            result.exec_id,
            result.metrics.cost_usd,
            result.metrics.duration_ms,
            result.metrics.num_turns,
            result.metrics.input_tokens,
            result.metrics.output_tokens,
        );

        let is_error = result.error.is_some();
        if let Some(ref error) = result.error {
            self.handle_run_error(&result, error);
        } else if result.diff.is_some() {
            self.handle_run_with_diff(&result);
        } else {
            self.handle_run_no_changes(&result);
        }

        // Auto-trigger queued follow-ups on successful completion.
        if !is_error {
            self.try_dispatch_follow_up(result.cue_id);
        }

        if self.claude.show_log == Some(result.cue_id) {
            if let Ok(execs) = self.db.get_all_executions(result.cue_id) {
                self.claude.conversation_history = execs;
            }
        }
        self.reload_cues();
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
    }

    fn handle_run_with_diff(&mut self, result: &ClaudeResult) {
        let diff = result.diff.as_ref().unwrap();
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
        self.refresh_open_tabs();
        self.reload_git_info();
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
