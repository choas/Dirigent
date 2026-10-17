//! Working out what a run actually changed.
//!
//! A pre-run content hash of every dirty file is the baseline; after the run
//! the diff is scoped to the files whose bytes moved. See [`super::resolve_run_diff`]
//! for the source-precedence order.

use std::collections::HashMap;
use std::path::Path;

use super::{ProviderRunOutcome, RunRequest};
use crate::git;
use crate::jj;
use crate::settings::VcsBackend;

pub(super) fn vcs_get_working_diff(req: &RunRequest, files: &[String]) -> Option<String> {
    match req.vcs_backend {
        VcsBackend::Jj => jj::jj_get_working_diff(req.project_root, files, &req.jj_cli_path),
        VcsBackend::Git => git::get_working_diff(req.project_root, files),
    }
}

pub(super) fn vcs_get_dirty_files(req: &RunRequest) -> HashMap<String, char> {
    match req.vcs_backend {
        VcsBackend::Jj => jj::jj_get_dirty_files(req.project_root, &req.jj_cli_path),
        VcsBackend::Git => git::get_dirty_files(req.project_root),
    }
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
pub(super) fn resolve_run_diff(
    req: &RunRequest,
    baseline: &HashMap<String, Option<u64>>,
    outcome: &ProviderRunOutcome,
) -> Option<String> {
    if !outcome.edited_files.is_empty()
        && let Some(d) = vcs_get_working_diff(req, &outcome.edited_files)
    {
        return Some(d);
    }
    scoped_working_diff(req, baseline).or_else(|| (outcome.parse_diff)(&outcome.stdout))
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
        if let Ok(mt) = meta.modified()
            && let Ok(d) = mt.duration_since(std::time::UNIX_EPOCH)
        {
            d.as_nanos().hash(&mut hasher);
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
pub(super) fn snapshot_dirty_state(req: &RunRequest) -> HashMap<String, Option<u64>> {
    vcs_get_dirty_files(req)
        .into_keys()
        .map(|f| {
            let hash = hash_file_bytes(&req.project_root.join(&f));
            (f, hash)
        })
        .collect()
}

pub(super) fn scoped_working_diff(
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
