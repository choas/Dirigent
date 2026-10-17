//! The seam between the app and the AI CLI backends.
//!
//! Every backend (Claude, Gemini, OpenCode, Codex, ACP agents) is invoked through the
//! [`AiBackend`] trait, whose provided [`AiBackend::invoke`] method runs the
//! one canonical sequence:
//!
//!   1. pre-run hook — blocking; a failure aborts before the CLI is spawned
//!      (and before `.Dirigent/.env` is read, so the hook may rewrite it),
//!   2. the backend-specific CLI invocation ([`AiBackend::invoke_cli`]),
//!   3. post-run hook — non-blocking and ALWAYS executed, even when the CLI
//!      errored or the user cancelled, so cleanup hooks reliably run,
//!   4. cancellation wins: a cancelled run reports [`BackendError::Cancelled`]
//!      regardless of what the CLI invocation returned,
//!   5. the single [`BackendResponse`] → [`ProviderRunOutcome`] metrics
//!      mapping, with the run duration measured here when the CLI stream did
//!      not report one.
//!
//! Before this trait existed each backend hand-rolled its own ordering, which
//! diverged four ways (Claude always ran the post-run hook, Gemini/OpenCode
//! returned `Cancelled` before it, Codex ran it and then checked cancel) and
//! left Claude runs with all-zero metrics.
//!
//! Hook-script trust used to be a per-backend decision too, and diverged the
//! same way: Codex refused every script (breaking the backend outright when one
//! was configured) while the other three handed identical config to `sh -c`
//! unconditionally. It is now a property of the *configuration source*
//! ([`BackendRequest::hook_trust`], set from `settings::trust`) and enforced in
//! one place, [`ai_process::run_hook_script`]. Response-diff parsing is the only
//! per-backend decision that remains, declared on the impls below so all four
//! are visible side by side.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::acp;
use crate::ai_process;
use crate::claude::{self, ClaudeRunMetadata, RunMetrics};
use crate::codex;
use crate::gemini;
use crate::opencode;
use crate::settings::CliProvider;

/// Log sink shared between the invoking thread and helper threads (stderr
/// collectors). Backends log through [`ai_process::with_locked_log`].
pub(crate) type LogFn = Box<dyn FnMut(&str) + Send + 'static>;
pub(crate) type SharedLog = Arc<Mutex<LogFn>>;

/// How much the *configuration source* is trusted for pre/post-run hook
/// scripts, which come from a `.Dirigent/settings.json` an attacker-controlled
/// repository can supply.
///
/// This is a property of where the settings came from, not of which backend is
/// running — the same configuration must get the same treatment from all four.
/// `settings::trust` makes the decision; [`BackendRequest::hook_trust`] carries
/// it, and `ai_process::run_hook_script` is the single place it is enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HookScriptTrust {
    /// Scripts originate from project-local configuration the user has not
    /// approved. They are skipped, never executed — and never fatal.
    ProjectLocal,
    /// The user approved this repository's execution-capable settings, so the
    /// scripts may run.
    Trusted,
}

/// Error type shared by all four AI CLI backends. `backend` carries the CLI
/// name (e.g. "gemini") so messages keep their old per-backend wording.
#[derive(Debug)]
pub(crate) enum BackendError {
    NotFound {
        backend: &'static str,
    },
    SpawnFailed {
        backend: &'static str,
        source: std::io::Error,
    },
    StreamReadError {
        backend: &'static str,
        source: std::io::Error,
    },
    WaitFailed {
        backend: &'static str,
        source: std::io::Error,
    },
    Cancelled,
    NonZeroExit {
        backend: &'static str,
        status: std::process::ExitStatus,
        /// Readable cause when the CLI reported one (stream error, stderr tail).
        reason: Option<String>,
    },
    InvalidExtraArgs {
        backend: &'static str,
        args: String,
    },
    /// Backend-specific failure that already carries a readable message
    /// (ACP protocol errors, authentication, refusals, …).
    Failed {
        message: String,
    },
}

impl std::error::Error for BackendError {}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendError::NotFound { backend } => {
                write!(f, "{backend} CLI not found on PATH")
            }
            BackendError::SpawnFailed { backend, source } => {
                write!(f, "failed to spawn {backend}: {source}")
            }
            BackendError::StreamReadError { backend, source } => {
                write!(f, "failed to read {backend} stdout: {source}")
            }
            BackendError::WaitFailed { backend, source } => {
                write!(f, "failed to wait for {backend}: {source}")
            }
            BackendError::Cancelled => write!(f, "cancelled"),
            BackendError::NonZeroExit {
                backend,
                status,
                reason: None,
            } => write!(f, "{backend} exited with {status}"),
            BackendError::NonZeroExit {
                backend,
                status,
                reason: Some(reason),
            } => write!(f, "{backend} exited with {status}: {reason}"),
            BackendError::InvalidExtraArgs { backend, args } => {
                write!(
                    f,
                    "failed to parse {backend} extra_args (unmatched quote?): {args}"
                )
            }
            BackendError::Failed { message } => write!(f, "{message}"),
        }
    }
}

/// Parameters for one CLI invocation. Backends ignore the fields that don't
/// apply to them (`extra_args_vec` and `use_pty` are Claude-only today).
pub(crate) struct BackendRequest<'a> {
    pub prompt: &'a str,
    pub project_root: &'a Path,
    pub model: &'a str,
    pub cli_path: &'a str,
    pub extra_args: &'a str,
    /// Structured args that must not go through shlex::split (e.g.
    /// --mcp-config paths). Passed directly as individual Command::arg entries.
    pub extra_args_vec: &'a [String],
    pub env_vars: &'a str,
    pub pre_run_script: &'a str,
    pub post_run_script: &'a str,
    /// Whether the settings the hook scripts came from were approved by the
    /// user. See [`HookScriptTrust`].
    pub hook_trust: HookScriptTrust,
    pub skip_permissions: bool,
    pub use_pty: bool,
    /// Analysis prompts that must not change the project. Only the ACP backend
    /// can enforce this today (it refuses writes and non-read permissions).
    pub read_only: bool,
}

/// Raw result of one CLI invocation, before normalization. Metric fields are
/// `None` when the backend's stream did not report them; [`AiBackend::invoke`]
/// fills the gaps (duration from wall clock, the rest defaulting to zero) so
/// consumers no longer repeat that mapping per backend.
#[derive(Debug, Clone, Default)]
pub(crate) struct BackendResponse {
    pub stdout: String,
    pub edited_files: Vec<String>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    pub num_turns: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub metadata: Option<ClaudeRunMetadata>,
}

/// Normalized outcome from any CLI backend's invocation. The post-run diff
/// resolution and result mapping (`finalize_run` in `app::claude_run`) consume
/// this common shape, keeping provider-specific drift out of the result path.
pub(crate) struct ProviderRunOutcome {
    pub stdout: String,
    pub edited_files: Vec<String>,
    pub metrics: RunMetrics,
    pub metadata: Option<ClaudeRunMetadata>,
    /// Fallback: extract a diff from the response text when the working tree
    /// shows no change (see `resolve_run_diff`).
    pub parse_diff: fn(&str) -> Option<String>,
}

/// A single AI CLI backend (Claude, Gemini, OpenCode or Codex).
///
/// Implementations provide the CLI-specific pieces; the run sequence itself is
/// the provided [`AiBackend::invoke`] and must not be overridden — it is the
/// whole point of this seam.
pub(crate) trait AiBackend: Sync {
    /// Human-readable name for status messages ("Claude", "Gemini", …).
    fn name(&self) -> &'static str;

    /// CLI name used in error messages ("claude", "gemini", …).
    fn cli_name(&self) -> &'static str;

    /// Parser used as the last-resort diff source for this backend's output.
    fn parse_diff_fn(&self) -> fn(&str) -> Option<String>;

    /// Run a pre/post-run hook script.
    ///
    /// Not overridable in practice: the policy lives in
    /// [`ai_process::run_hook_script`] so all four backends treat the same
    /// configuration identically.
    fn run_hook(
        &self,
        label: &str,
        script: &str,
        trust: HookScriptTrust,
        project_root: &Path,
        on_log: &SharedLog,
        fail_on_error: bool,
    ) -> Result<(), BackendError> {
        ai_process::run_hook_with_locked_log(
            on_log,
            label,
            script,
            trust,
            project_root,
            fail_on_error,
        )
        .map_err(|source| BackendError::SpawnFailed {
            backend: self.cli_name(),
            source,
        })
    }

    /// Backend-specific CLI invocation: spawn, stream, wait, interpret the
    /// exit status. No hook execution and no post-run cancel mapping — the
    /// driver owns those. Implementations may still return
    /// [`BackendError::Cancelled`] early to suppress noisy killed-child
    /// diagnostics; the driver's own cancel check makes the final call.
    fn invoke_cli(
        &self,
        req: &BackendRequest<'_>,
        on_log: &SharedLog,
        cancel: &Arc<AtomicBool>,
    ) -> Result<BackendResponse, BackendError>;

    /// The canonical run sequence shared by all backends — see module docs.
    fn invoke(
        &self,
        req: &BackendRequest<'_>,
        on_log: LogFn,
        cancel: Arc<AtomicBool>,
    ) -> Result<ProviderRunOutcome, BackendError> {
        let on_log: SharedLog = Arc::new(Mutex::new(on_log));

        self.run_hook(
            "pre-run",
            req.pre_run_script,
            req.hook_trust,
            req.project_root,
            &on_log,
            true,
        )?;

        let run_start = Instant::now();
        let result = self.invoke_cli(req, &on_log, &cancel);

        // The post-run hook always runs — even after a CLI error or a user
        // cancel — so cleanup (resetting env, releasing locks, …) is not
        // skipped. With fail_on_error=false a failing hook is logged, never
        // fatal.
        let _ = self.run_hook(
            "post-run",
            req.post_run_script,
            req.hook_trust,
            req.project_root,
            &on_log,
            false,
        );

        if cancel.load(Ordering::Relaxed) {
            return Err(BackendError::Cancelled);
        }
        let response = result?;

        // The single metrics mapping for all four backends. `duration_ms`
        // falls back to the measured wall clock, which is what fixed Claude
        // (and Codex without stream timings) reporting a zero duration.
        let elapsed_ms = run_start.elapsed().as_millis() as u64;
        Ok(ProviderRunOutcome {
            stdout: response.stdout,
            edited_files: response.edited_files,
            metrics: RunMetrics {
                cost_usd: response.cost_usd,
                duration_ms: response.duration_ms.unwrap_or(elapsed_ms),
                num_turns: response.num_turns,
                input_tokens: response.input_tokens,
                output_tokens: response.output_tokens,
            },
            metadata: response.metadata,
            parse_diff: self.parse_diff_fn(),
        })
    }
}

/// Resolve the backend implementation for a configured provider.
pub(crate) fn backend_for(provider: &CliProvider) -> &'static dyn AiBackend {
    match provider {
        CliProvider::Claude => &ClaudeBackend,
        CliProvider::Gemini => &GeminiBackend,
        CliProvider::OpenCode => &OpenCodeBackend,
        CliProvider::Codex => &CodexBackend,
        CliProvider::Acp => &AcpBackend,
    }
}

fn no_response_diff(_response: &str) -> Option<String> {
    None
}

pub(crate) struct ClaudeBackend;

impl AiBackend for ClaudeBackend {
    fn name(&self) -> &'static str {
        "Claude"
    }

    fn cli_name(&self) -> &'static str {
        "claude"
    }

    fn parse_diff_fn(&self) -> fn(&str) -> Option<String> {
        claude::parse_diff_from_response
    }

    fn invoke_cli(
        &self,
        req: &BackendRequest<'_>,
        on_log: &SharedLog,
        cancel: &Arc<AtomicBool>,
    ) -> Result<BackendResponse, BackendError> {
        // Claude logs only from the invoking thread (headless stderr is
        // collected silently), so holding the lock for the whole run is safe.
        ai_process::with_locked_log(on_log, "claude run", |log| {
            claude::run_claude_cli(req, &mut **log, Arc::clone(cancel))
        })
    }
}

pub(crate) struct GeminiBackend;

impl AiBackend for GeminiBackend {
    fn name(&self) -> &'static str {
        "Gemini"
    }

    fn cli_name(&self) -> &'static str {
        "gemini"
    }

    fn parse_diff_fn(&self) -> fn(&str) -> Option<String> {
        claude::parse_diff_from_response
    }

    fn invoke_cli(
        &self,
        req: &BackendRequest<'_>,
        on_log: &SharedLog,
        cancel: &Arc<AtomicBool>,
    ) -> Result<BackendResponse, BackendError> {
        gemini::run_gemini_cli(req, on_log, cancel)
    }
}

pub(crate) struct OpenCodeBackend;

impl AiBackend for OpenCodeBackend {
    fn name(&self) -> &'static str {
        "OpenCode"
    }

    fn cli_name(&self) -> &'static str {
        "opencode"
    }

    fn parse_diff_fn(&self) -> fn(&str) -> Option<String> {
        claude::parse_diff_from_response
    }

    fn invoke_cli(
        &self,
        req: &BackendRequest<'_>,
        on_log: &SharedLog,
        cancel: &Arc<AtomicBool>,
    ) -> Result<BackendResponse, BackendError> {
        opencode::run_opencode_cli(req, on_log, cancel)
    }
}

pub(crate) struct CodexBackend;

impl AiBackend for CodexBackend {
    fn name(&self) -> &'static str {
        "Codex"
    }

    fn cli_name(&self) -> &'static str {
        "codex"
    }

    fn parse_diff_fn(&self) -> fn(&str) -> Option<String> {
        no_response_diff
    }

    fn invoke_cli(
        &self,
        req: &BackendRequest<'_>,
        on_log: &SharedLog,
        cancel: &Arc<AtomicBool>,
    ) -> Result<BackendResponse, BackendError> {
        // Codex logs only from the invoking thread; its stderr collector
        // accumulates without logging, so locking for the run is safe.
        ai_process::with_locked_log(on_log, "codex run", |log| {
            codex::run_codex_cli(req, &mut **log, cancel)
        })
    }
}

pub(crate) struct AcpBackend;

impl AiBackend for AcpBackend {
    fn name(&self) -> &'static str {
        "ACP Agent"
    }

    fn cli_name(&self) -> &'static str {
        "ACP agent"
    }

    fn parse_diff_fn(&self) -> fn(&str) -> Option<String> {
        claude::parse_diff_from_response
    }

    fn invoke_cli(
        &self,
        req: &BackendRequest<'_>,
        on_log: &SharedLog,
        cancel: &Arc<AtomicBool>,
    ) -> Result<BackendResponse, BackendError> {
        // The ACP reader and stderr threads never log, so the invoking thread
        // can hold the lock for the whole turn.
        ai_process::with_locked_log(on_log, "acp run", |log| {
            acp::run_acp_cli(req, &mut **log, cancel)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stub backend that records the driver's call sequence.
    struct StubBackend {
        calls: Mutex<Vec<String>>,
        cli_result: fn() -> Result<BackendResponse, BackendError>,
        set_cancel: bool,
    }

    impl StubBackend {
        fn new(cli_result: fn() -> Result<BackendResponse, BackendError>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                cli_result,
                set_cancel: false,
            }
        }
    }

    impl AiBackend for StubBackend {
        fn name(&self) -> &'static str {
            "Stub"
        }

        fn cli_name(&self) -> &'static str {
            "stub"
        }

        fn parse_diff_fn(&self) -> fn(&str) -> Option<String> {
            no_response_diff
        }

        fn run_hook(
            &self,
            label: &str,
            _script: &str,
            _trust: HookScriptTrust,
            _project_root: &Path,
            _on_log: &SharedLog,
            _fail_on_error: bool,
        ) -> Result<(), BackendError> {
            self.calls.lock().unwrap().push(label.to_string());
            Ok(())
        }

        fn invoke_cli(
            &self,
            _req: &BackendRequest<'_>,
            _on_log: &SharedLog,
            cancel: &Arc<AtomicBool>,
        ) -> Result<BackendResponse, BackendError> {
            self.calls.lock().unwrap().push("cli".to_string());
            if self.set_cancel {
                cancel.store(true, Ordering::Relaxed);
            }
            (self.cli_result)()
        }
    }

    fn request<'a>(root: &'a Path) -> BackendRequest<'a> {
        BackendRequest {
            prompt: "p",
            project_root: root,
            model: "",
            cli_path: "",
            extra_args: "",
            extra_args_vec: &[],
            env_vars: "",
            pre_run_script: "pre",
            post_run_script: "post",
            hook_trust: HookScriptTrust::Trusted,
            skip_permissions: false,
            use_pty: false,
            read_only: false,
        }
    }

    #[test]
    fn post_run_hook_runs_even_when_cli_fails() {
        let backend = StubBackend::new(|| {
            Err(BackendError::NonZeroExit {
                backend: "stub",
                status: std::process::ExitStatus::default(),
                reason: None,
            })
        });
        let tmp = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));

        let result = backend.invoke(&request(tmp.path()), Box::new(|_| {}), cancel);

        assert!(matches!(result, Err(BackendError::NonZeroExit { .. })));
        assert_eq!(
            *backend.calls.lock().unwrap(),
            vec!["pre-run", "cli", "post-run"]
        );
    }

    #[test]
    fn cancel_wins_over_successful_cli_result_and_post_hook_still_runs() {
        let mut backend = StubBackend::new(|| Ok(BackendResponse::default()));
        backend.set_cancel = true;
        let tmp = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));

        let result = backend.invoke(&request(tmp.path()), Box::new(|_| {}), cancel);

        assert!(matches!(result, Err(BackendError::Cancelled)));
        assert_eq!(
            *backend.calls.lock().unwrap(),
            vec!["pre-run", "cli", "post-run"]
        );
    }

    #[test]
    fn metrics_fall_back_to_measured_duration_and_zero_cost() {
        let backend = StubBackend::new(|| {
            Ok(BackendResponse {
                stdout: "out".to_string(),
                cost_usd: None,
                duration_ms: None,
                num_turns: Some(3),
                ..BackendResponse::default()
            })
        });
        let tmp = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));

        let outcome = backend
            .invoke(&request(tmp.path()), Box::new(|_| {}), cancel)
            .expect("run should succeed");

        assert_eq!(outcome.stdout, "out");
        assert_eq!(
            outcome.metrics.cost_usd, None,
            "a backend that reports no cost must not record zero"
        );
        assert_eq!(outcome.metrics.num_turns, Some(3));
        assert_eq!(outcome.metrics.input_tokens, None);
        assert_eq!(outcome.metrics.output_tokens, None);
        // duration_ms is the measured wall clock — tiny but always stamped.
        assert!(outcome.metrics.duration_ms < 60_000);
    }

    #[test]
    fn reported_token_counts_reach_the_metrics() {
        let backend = StubBackend::new(|| {
            Ok(BackendResponse {
                input_tokens: Some(1200),
                output_tokens: Some(34),
                ..BackendResponse::default()
            })
        });
        let tmp = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));

        let outcome = backend
            .invoke(&request(tmp.path()), Box::new(|_| {}), cancel)
            .expect("run should succeed");

        assert_eq!(outcome.metrics.input_tokens, Some(1200));
        assert_eq!(outcome.metrics.output_tokens, Some(34));
    }

    #[test]
    fn stream_duration_wins_over_wall_clock() {
        let backend = StubBackend::new(|| {
            Ok(BackendResponse {
                duration_ms: Some(1234),
                ..BackendResponse::default()
            })
        });
        let tmp = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));

        let outcome = backend
            .invoke(&request(tmp.path()), Box::new(|_| {}), cancel)
            .expect("run should succeed");

        assert_eq!(outcome.metrics.duration_ms, 1234);
    }
}
