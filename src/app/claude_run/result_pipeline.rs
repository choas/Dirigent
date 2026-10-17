//! The run itself: snapshot the working tree, drive the backend through the
//! [`crate::ai_backend::AiBackend`] seam, then map its outcome (or error) onto
//! a single [`super::ClaudeResult`].

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::run_diff::{resolve_run_diff, snapshot_dirty_state};
use super::{ClaudeResult, RunRequest};
use crate::ai_backend::{self, ProviderRunOutcome};
use crate::claude;

/// Run the CLI provider on a background thread and produce a `ClaudeResult`.
///
/// All four providers go through the [`ai_backend::AiBackend`] trait, which
/// owns the canonical hook/cancel ordering and the single metrics mapping;
/// only the pre-run dirty-file baseline and the result mapping live here.
pub(super) fn run_provider(
    req: &RunRequest,
    on_log: impl FnMut(&str) + Send + 'static,
    cancel: Arc<AtomicBool>,
) -> ClaudeResult {
    // Snapshot dirty files before the run (and before the pre-run hook, which
    // may itself modify files) so the fallback diff can detect which files
    // were actually modified — including ones that were already dirty at
    // baseline — by comparing post-run content to pre-run hashes.
    let baseline = snapshot_dirty_state(req);

    let backend = ai_backend::backend_for(&req.provider);
    let invocation = ai_backend::BackendRequest {
        prompt: req.prompt,
        project_root: req.project_root,
        model: &req.config.model,
        cli_path: &req.config.cli_path,
        extra_args: &req.config.extra_args,
        extra_args_vec: &req.config.extra_args_vec,
        env_vars: &req.config.env_vars,
        pre_run_script: &req.config.pre_run_script,
        post_run_script: &req.config.post_run_script,
        hook_trust: req.config.hook_trust,
        skip_permissions: req.config.skip_permissions,
        use_pty: req.config.use_pty,
        read_only: false,
    };
    let res = backend.invoke(&invocation, Box::new(on_log), cancel);
    finalize_run(req, &baseline, res)
}

/// Build a `ClaudeResult` from a provider's outcome (or error). Centralizing
/// this here prevents the three providers from drifting in how they map
/// success/failure into the result type.
pub(super) fn finalize_run<E: std::fmt::Display>(
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
                metadata: outcome.metadata,
            }
        }
        Err(e) => ClaudeResult {
            cue_id: req.cue_id,
            exec_id: req.exec_id,
            diff: None,
            response: String::new(),
            error: Some(e.to_string()),
            metrics: claude::RunMetrics::default(),
            metadata: None,
        },
    }
}
