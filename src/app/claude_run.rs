use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use super::notifications::send_macos_notification;
use super::tasks::TaskHandle;
use super::DirigentApp;
use crate::agents::AgentTrigger;
use crate::claude;
use crate::db::{Cue, CueStatus, Execution};
use crate::gemini;
use crate::git;
use crate::jj;
use crate::opencode;
use crate::settings::{self, CliProvider, CueCommand, VcsBackend};
use crate::telemetry;

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
    /// Per-cue timeline of recent log-arrival instants, used to draw the
    /// heartbeat strip beneath the running conversation view.
    pub(super) log_heartbeats: HashMap<i64, VecDeque<Instant>>,
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
            log_heartbeats: HashMap::new(),
            auto_continue_count: HashMap::new(),
            auto_continue_spawn_retries: HashMap::new(),
            workspace_paths: HashMap::new(),
            workspace_names: HashMap::new(),
            workspace_commit_failed: HashSet::new(),
        }
    }
}

/// Configuration extracted from Settings for a specific CLI provider.
struct ProviderConfig {
    model: String,
    cli_path: String,
    extra_args: String,
    /// Structured args that must not go through shlex::split (e.g. --mcp-config
    /// paths). Passed directly as individual Command::arg entries.
    extra_args_vec: Vec<String>,
    env_vars: String,
    pre_run_script: String,
    post_run_script: String,
    skip_permissions: bool,
    use_pty: bool,
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

fn should_inject_dirigent_mcp(prompt: &str) -> bool {
    prompt.to_lowercase().contains("dirigent")
}

fn write_dirigent_mcp_config(
    project_root: &std::path::Path,
    mcp_bin: &str,
    db_path: &std::path::Path,
) -> std::io::Result<std::path::PathBuf> {
    let dir = project_root.join(".Dirigent");
    std::fs::create_dir_all(&dir)?;
    let config_path = dir.join("mcp-config.json");
    let escaped_bin = mcp_bin.replace('\\', "\\\\").replace('"', "\\\"");
    let escaped_db = db_path
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let config = format!(
        r#"{{"mcpServers":{{"dirigent":{{"type":"stdio","command":"{}","args":["{}"]}}}}}}"#,
        escaped_bin, escaped_db,
    );
    std::fs::write(&config_path, config)?;
    Ok(config_path)
}

fn resolve_dirigent_db(
    project_root: &std::path::Path,
    settings: &settings::Settings,
) -> Option<std::path::PathBuf> {
    let local_db = project_root.join(".Dirigent").join("Dirigent.db");
    if local_db.exists() {
        return Some(local_db);
    }
    if !settings.dirigent_mcp_db_path.is_empty() {
        let path = std::path::PathBuf::from(&settings.dirigent_mcp_db_path);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

/// Build provider-specific configuration from settings, with optional command overrides.
/// When the prompt mentions "Dirigent" and the provider is Claude, the Dirigent MCP
/// server is conditionally injected via `--mcp-config` so Claude gains cue-management tools.
fn build_provider_config(
    settings: &settings::Settings,
    provider: &CliProvider,
    matched_command: &Option<CueCommand>,
    prompt: &str,
    project_root: &std::path::Path,
) -> ProviderConfig {
    let pf = settings.provider_fields(provider);
    let model = pf.model.to_string();
    let cli_path = pf.cli_path.to_string();
    let mut extra_args = pf.extra_args.to_string();
    let env_vars = pf.env_vars.to_string();
    let pre_run_script = matched_command
        .as_ref()
        .filter(|cmd| !cmd.pre_agent.is_empty())
        .map(|cmd| cmd.pre_agent.clone())
        .unwrap_or_else(|| pf.pre_run_script.to_string());
    let post_run_script = matched_command
        .as_ref()
        .filter(|cmd| !cmd.post_agent.is_empty())
        .map(|cmd| cmd.post_agent.clone())
        .unwrap_or_else(|| pf.post_run_script.to_string());
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
    // Conditionally inject the Dirigent MCP server when the prompt references
    // "Dirigent" — gives Claude tools to manage cues directly via --mcp-config.
    let mut extra_args_vec = Vec::new();
    if *provider == CliProvider::Claude && should_inject_dirigent_mcp(prompt) {
        if let Some(db_path) = resolve_dirigent_db(project_root, settings) {
            let mcp_bin = if settings.dirigent_mcp_server_path.is_empty() {
                "dirigent-mcp".to_string()
            } else {
                settings.dirigent_mcp_server_path.clone()
            };
            if which::which(&mcp_bin).is_ok() || std::path::Path::new(&mcp_bin).exists() {
                match write_dirigent_mcp_config(project_root, &mcp_bin, &db_path) {
                    Ok(config_path) => {
                        extra_args_vec.push("--mcp-config".to_string());
                        extra_args_vec.push(config_path.display().to_string());
                        log::info!(
                            "Dirigent MCP server injected: {} {}",
                            mcp_bin,
                            db_path.display()
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to write Dirigent MCP config: {e}");
                    }
                }
            } else {
                log::warn!(
                    "Dirigent MCP server binary '{}' not found — \
                     install with: cd dirigent-mcp-server && npm install && npm run build && npm link",
                    mcp_bin,
                );
            }
        }
    }
    ProviderConfig {
        model,
        cli_path,
        extra_args,
        extra_args_vec,
        env_vars,
        pre_run_script,
        post_run_script,
        skip_permissions: settings.allow_dangerous_skip_permissions,
        use_pty: settings.claude_use_pty,
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

fn vcs_get_working_diff(req: &RunRequest, files: &[String]) -> Option<String> {
    match req.vcs_backend {
        VcsBackend::Jj => jj::jj_get_working_diff(req.project_root, files, &req.jj_cli_path),
        VcsBackend::Git => git::get_working_diff(req.project_root, files),
    }
}

fn vcs_get_dirty_files(req: &RunRequest) -> HashMap<String, char> {
    match req.vcs_backend {
        VcsBackend::Jj => jj::jj_get_dirty_files(req.project_root, &req.jj_cli_path),
        VcsBackend::Git => git::get_dirty_files(req.project_root),
    }
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
        CliProvider::Gemini => run_gemini_provider(req, on_log, cancel),
    }
}

/// Normalized outcome from any CLI provider's streaming invocation. Each
/// provider adapts its specific response into this common shape so the
/// post-run diff resolution and `ClaudeResult` mapping can be shared via
/// `finalize_run` — keeping provider-specific drift out of the result path.
struct ProviderRunOutcome {
    stdout: String,
    edited_files: Vec<String>,
    metrics: claude::RunMetrics,
    parse_diff: fn(&str) -> Option<String>,
}

/// Resolve the run diff using a single canonical source order shared by all
/// providers. The order is, from most to least trustworthy:
///
///   1. `git diff` over files the provider explicitly reports it edited.
///   2. `git diff` over files whose content actually changed vs. the pre-run
///      baseline (`scoped_working_diff`).
///   3. A diff parsed out of the provider's response text
///      (`outcome.parse_diff`).
///
/// Filesystem reality (1 and 2) always wins over text-parsed diffs (3): the
/// working tree is the ground truth, while parsing a diff out of free-form
/// model output depends on provider-specific formatting and is easily fooled
/// by truncation, fenced examples, or multiple embedded hunks. Putting the
/// text-parsed diff last is what keeps the same model output from being
/// interpreted differently across Claude, Gemini, and OpenCode.
fn resolve_run_diff(
    req: &RunRequest,
    baseline: &HashMap<String, Option<u64>>,
    outcome: &ProviderRunOutcome,
) -> Option<String> {
    if !outcome.edited_files.is_empty() {
        if let Some(d) = vcs_get_working_diff(req, &outcome.edited_files) {
            return Some(d);
        }
    }
    scoped_working_diff(req, baseline).or_else(|| (outcome.parse_diff)(&outcome.stdout))
}

/// Build a `ClaudeResult` from a provider's outcome (or error). Centralizing
/// this here prevents the three providers from drifting in how they map
/// success/failure into the result type.
fn finalize_run<E: std::fmt::Display>(
    req: &RunRequest,
    baseline: &HashMap<String, Option<u64>>,
    res: Result<ProviderRunOutcome, E>,
) -> ClaudeResult {
    match res {
        Ok(outcome) => {
            let diff = resolve_run_diff(req, baseline, &outcome);
            ClaudeResult {
                cue_id: req.cue_id,
                exec_id: req.exec_id,
                diff,
                response: outcome.stdout,
                error: None,
                metrics: outcome.metrics,
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

fn run_claude_provider(
    req: &RunRequest,
    on_log: impl FnMut(&str) + Send + 'static,
    cancel: Arc<AtomicBool>,
) -> ClaudeResult {
    let baseline = snapshot_dirty_state(req);

    let res = claude::invoke_claude_streaming(
        req.prompt,
        req.project_root,
        &req.config.model,
        &req.config.cli_path,
        &req.config.extra_args,
        &req.config.extra_args_vec,
        &req.config.env_vars,
        &req.config.pre_run_script,
        &req.config.post_run_script,
        req.config.skip_permissions,
        req.config.use_pty,
        on_log,
        cancel,
    )
    .map(|response| ProviderRunOutcome {
        stdout: response.stdout,
        edited_files: Vec::new(),
        metrics: response.metrics,
        parse_diff: claude::parse_diff_from_response,
    });
    finalize_run(req, &baseline, res)
}

/// Fingerprint a file for change detection. Returns `None` when the file
/// cannot be stat'd (e.g. it has been deleted in the working tree).
///
/// Files at or below `HASH_CONTENT_CAP_BYTES` are hashed by content for
/// exact comparison. Larger files (large logs, binaries, datasets that
/// happen to be in the dirty set) fall back to a `(size, mtime)`
/// fingerprint — reading them on every run would otherwise burn memory
/// and stall the pre-run snapshot.
fn hash_file_bytes(path: &Path) -> Option<u64> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    const HASH_CONTENT_CAP_BYTES: u64 = 4 * 1024 * 1024;

    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let size = meta.len();
    let mut hasher = DefaultHasher::new();
    if size > HASH_CONTENT_CAP_BYTES {
        size.hash(&mut hasher);
        if let Ok(mt) = meta.modified() {
            if let Ok(d) = mt.duration_since(std::time::UNIX_EPOCH) {
                d.as_nanos().hash(&mut hasher);
            }
        }
        return Some(hasher.finish());
    }
    let bytes = std::fs::read(path).ok()?;
    bytes.hash(&mut hasher);
    Some(hasher.finish())
}

/// Snapshot every currently-dirty file's content hash before a run starts.
/// `scoped_working_diff` consults this to decide whether an already-dirty
/// file was further modified by the run.
fn snapshot_dirty_state(req: &RunRequest) -> HashMap<String, Option<u64>> {
    vcs_get_dirty_files(req)
        .into_keys()
        .map(|f| {
            let hash = hash_file_bytes(&req.project_root.join(&f));
            (f, hash)
        })
        .collect()
}

fn scoped_working_diff(
    req: &RunRequest,
    baseline: &HashMap<String, Option<u64>>,
) -> Option<String> {
    let current_dirty = vcs_get_dirty_files(req);
    let changed: Vec<String> = current_dirty
        .into_keys()
        .filter(|f| {
            let current = hash_file_bytes(&req.project_root.join(f));
            match baseline.get(f) {
                None => true,
                Some(prev) => prev != &current,
            }
        })
        .collect();
    if changed.is_empty() {
        return None;
    }
    vcs_get_working_diff(req, &changed)
}

fn run_gemini_provider(
    req: &RunRequest,
    on_log: impl FnMut(&str) + Send + 'static,
    cancel: Arc<AtomicBool>,
) -> ClaudeResult {
    let baseline = snapshot_dirty_state(req);

    let gemini_config = gemini::GeminiRunConfig {
        model: &req.config.model,
        cli_path: &req.config.cli_path,
        extra_args: &req.config.extra_args,
        env_vars: &req.config.env_vars,
        pre_run_script: &req.config.pre_run_script,
        post_run_script: &req.config.post_run_script,
    };
    let res = gemini::invoke_gemini_streaming(
        req.prompt,
        req.project_root,
        &gemini_config,
        on_log,
        cancel,
    )
    .map(|response| ProviderRunOutcome {
        stdout: response.stdout,
        edited_files: response.edited_files,
        metrics: claude::RunMetrics {
            cost_usd: response.cost_usd.unwrap_or(0.0),
            duration_ms: response.duration_ms.unwrap_or(0),
            num_turns: response.num_turns.unwrap_or(0),
            ..claude::RunMetrics::default()
        },
        parse_diff: gemini::parse_diff_from_response,
    });
    finalize_run(req, &baseline, res)
}

fn run_opencode_provider(
    req: &RunRequest,
    on_log: impl FnMut(&str) + Send + 'static,
    cancel: Arc<AtomicBool>,
) -> ClaudeResult {
    // Snapshot dirty files before the run so the fallback diff can detect
    // which files were actually modified — including ones that were already
    // dirty at baseline — by comparing post-run content to pre-run hashes.
    let baseline = snapshot_dirty_state(req);

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
    )
    .map(|response| ProviderRunOutcome {
        stdout: response.stdout,
        edited_files: response.edited_files,
        metrics: claude::RunMetrics {
            cost_usd: response.cost_usd.unwrap_or(0.0),
            duration_ms: response.duration_ms.unwrap_or(0),
            num_turns: response.num_turns.unwrap_or(0),
            ..claude::RunMetrics::default()
        },
        parse_diff: opencode::parse_diff_from_response,
    });
    finalize_run(req, &baseline, res)
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

        // For Claude provider, inject session continuity args.
        if provider == CliProvider::Claude {
            let session_id = if let Some(ref id) = resume_session_id {
                config.extra_args_vec.push("--resume".to_string());
                config.extra_args_vec.push(id.clone());
                id.clone()
            } else {
                let id = uuid::Uuid::new_v4().to_string();
                config.extra_args_vec.push("--session-id".to_string());
                config.extra_args_vec.push(id.clone());
                id
            };
            let _ = self.db.update_execution_session_id(exec_id, &session_id);
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
        if let Err(e) = settings::sync_home_guard_hook(
            &self.project_root,
            self.settings.allow_home_folder_access,
        ) {
            log::error!("Failed to sync home guard hook: {e:#}");
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
                self.claude.log_heartbeats.remove(&cue_id);
                let _ = self.db.update_cue_status(cue_id, CueStatus::Inbox);
                self.set_status_message("Failed to start run: cue not found".to_string());
                self.reload_cues();
                return;
            }
        };

        if matches!(self.settings.vcs_backend, VcsBackend::Jj)
            && !self.claude.workspace_paths.contains_key(&cue_id)
        {
            let ws_name = jj::cue_workspace_name(cue_id, &cue.text);
            match jj::jj_create_workspace(&self.project_root, &ws_name, &self.settings.jj_cli_path)
            {
                Ok(ws_path) => {
                    self.claude.workspace_paths.insert(cue_id, ws_path);
                    self.claude.workspace_names.insert(cue_id, ws_name);
                }
                Err(e) => {
                    log::error!("Failed to create jj workspace for cue {cue_id}: {e}");
                }
            }
        }

        let (effective_text, matched_command) =
            resolve_command_prefix(&cue.text, &self.settings.commands);
        let prompt = self.build_initial_prompt(&effective_text, &cue);

        if !self.insert_exec_and_spawn(cue_id, prompt, provider, &matched_command, None) {
            let _ = self.db.update_cue_status(cue_id, CueStatus::Inbox);
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

        let provider = self.settings.cli_provider.clone();

        let cue = match self.cues.iter().find(|c| c.id == cue_id) {
            Some(c) => c.clone(),
            None => {
                self.set_status_message("Failed to start reply: cue not found".to_string());
                return;
            }
        };

        if matches!(self.settings.vcs_backend, VcsBackend::Jj)
            && !self.claude.workspace_paths.contains_key(&cue_id)
        {
            let ws_name = jj::cue_workspace_name(cue_id, &cue.text);
            if let Some(parent) = self.project_root.parent() {
                let dir_name = ws_name.rsplit('/').next().unwrap_or(&ws_name);
                let expected = parent.join(dir_name);
                if expected.is_dir() {
                    self.claude.workspace_paths.insert(cue_id, expected);
                    self.claude.workspace_names.insert(cue_id, ws_name);
                } else if let Ok(ws_path) = jj::jj_create_workspace(
                    &self.project_root,
                    &ws_name,
                    &self.settings.jj_cli_path,
                ) {
                    self.claude.workspace_paths.insert(cue_id, ws_path);
                    self.claude.workspace_names.insert(cue_id, ws_name);
                }
            }
        }

        let (raw_text, matched_command) =
            resolve_command_prefix(&cue.text, &self.settings.commands);
        let original_text = build_pr_hint_text(&raw_text, cue.source_ref.as_deref());

        let mut all_images = cue.attached_images.clone();
        all_images.extend_from_slice(reply_images);

        let resume_session_id = self.db.get_cue_session_id(cue_id).ok().flatten();

        // When resuming a Claude session the previous context is already in the
        // conversation, so we only need to send the user's reply. For a fresh
        // session (or non-Claude provider) we build the full structured prompt.
        // We also need the full prompt when there are new images to attach.
        let prompt = if resume_session_id.is_some()
            && all_images.is_empty()
            && provider == CliProvider::Claude
        {
            reply.to_string()
        } else {
            let previous_diff = self
                .db
                .get_latest_execution(cue_id)
                .ok()
                .flatten()
                .and_then(|e| e.diff)
                .unwrap_or_default();

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

        let _ = self.db.update_cue_status(cue_id, CueStatus::Ready);
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
            let _ = self.db.update_cue_status(cue_id, CueStatus::Review);
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

        let prompt = if resume_session_id.is_some() && provider == CliProvider::Claude {
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

        let _ = self.db.update_cue_status(cue_id, CueStatus::Ready);
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
        let _ = self.db.update_cue_status(cue_id, CueStatus::Done);
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

        let _ = self.db.update_execution_metrics(
            result.exec_id,
            result.metrics.cost_usd,
            result.metrics.duration_ms,
            result.metrics.num_turns,
            result.metrics.input_tokens,
            result.metrics.output_tokens,
        );

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

        // Dispatch pending auto-continues now that tracking state is cleared.
        let auto_continues = std::mem::take(&mut self.pending_auto_continues);
        for cue_id in auto_continues {
            self.dispatch_auto_continue(cue_id);
        }

        self.on_workflow_cue_completed(result.cue_id);
    }

    /// Flush the running log to DB and remove tracking state for a completed result.
    fn flush_and_clear_tracking(&mut self, result: &ClaudeResult) {
        if let Some((log_text, _)) = self.claude.running_logs.get(&result.cue_id) {
            let _ = self.db.update_execution_log(result.exec_id, log_text);
        }
        self.claude.running_logs.remove(&result.cue_id);
        self.claude.exec_ids.remove(&result.cue_id);
        self.claude.start_times.remove(&result.cue_id);
        self.claude.log_heartbeats.remove(&result.cue_id);
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
        } else if let Some(ref limit_line) = usage_limit_msg {
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

    /// Post-processing after a successful (non-error) run: refresh git state,
    /// open tabs, and dispatch any queued follow-ups.
    fn handle_successful_run(&mut self, result: &ClaudeResult) {
        let plan_path = self
            .claude
            .running_logs
            .get(&result.cue_id)
            .and_then(|(log, _)| claude::extract_plan_path(log));
        let _ = self
            .db
            .update_cue_plan_path(result.cue_id, plan_path.as_deref());

        let _ = self.reload_open_tabs_and_notify_lsp();
        self.reload_git_info();
        self.reload_commit_history();
        self.try_dispatch_follow_up(result.cue_id);
    }

    /// Finalize a stale execution in the DB without touching live state.
    fn complete_stale_execution(&self, result: &ClaudeResult) {
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
        self.claude.auto_continue_count.remove(&result.cue_id);
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
        if !self.show_settings {
            self.reload_settings_from_disk();
        }
        let agents_started =
            self.trigger_agents_for(&AgentTrigger::AfterRun, Some(result.cue_id), &cue_prompt);

        if self.settings.auto_commit {
            if self.claude.show_log == Some(result.cue_id) {
                // User is watching the log — defer so they can review.
                // If they close the log without explicitly accepting,
                // the auto-commit is skipped.
                self.pending_auto_commits.push(result.cue_id);
                self.user_reviewed_auto_commits.insert(result.cue_id);
            } else if agents_started > 0 {
                self.pending_auto_commits.push(result.cue_id);
            } else {
                self.process_commit_review(result.cue_id);
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
            if self
                .db
                .insert_cue_from_source(
                    &item.text,
                    &source_label,
                    "",
                    &source_ref,
                    &item.file_path,
                    item.line_number,
                )
                .is_ok()
            {
                count += 1;
            }
        }

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
        let _ = self.db.update_cue_status(result.cue_id, CueStatus::Done);
        let activity = format!("Imported {} cue(s)", count);
        let _ = self.db.log_activity(result.cue_id, &activity);
        self.set_status_message(format!("Imported {} cue(s) from {}", count, source_label));
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
        let has_question = claude::response_has_question(&result.response);
        let preview = self.cue_preview(result.cue_id);
        if has_question {
            self.claude.auto_continue_count.remove(&result.cue_id);
            self.set_status_message(format!("Claude is asking a question about \"{}\"", preview));
            let _ = self.db.update_cue_has_question(result.cue_id, true);
            let _ = self.db.update_cue_status(result.cue_id, CueStatus::Done);
            let _ = self
                .db
                .log_activity(result.cue_id, "Run completed — question pending");
            self.reply_inputs
                .entry(result.cue_id)
                .or_insert_with(String::new);
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
            let _ = self.db.update_cue_has_question(result.cue_id, false);
            let _ = self.db.log_activity(
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
            let _ = self.db.update_cue_has_question(result.cue_id, false);
            let _ = self.db.update_cue_status(result.cue_id, CueStatus::Done);
            let _ = self
                .db
                .log_activity(result.cue_id, "Run completed — no changes");
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
