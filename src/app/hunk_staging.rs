//! App-side glue for hunk-level staging shown in the diff-review overlay.
//!
//! The staging UI lives in [`super::dialog`]'s diff review; these methods do the
//! git work: reload the staged/unstaged diff, and stage/unstage/discard a single
//! hunk by (file, hunk) index. Each op re-reads the diff immediately before
//! building the patch to minimise context drift.

use crate::diff_view::{parse_unified_diff, ParsedDiff};
use crate::git;

use super::DirigentApp;

enum HunkOp {
    Stage,
    Unstage,
    Discard,
}

impl DirigentApp {
    /// Which of a parsed diff's files are partially staged (drives the `[-]` mark).
    pub(in crate::app) fn compute_partial_staged(&self, parsed: &ParsedDiff) -> Vec<String> {
        parsed
            .files
            .iter()
            .map(|f| f.display_path().to_string())
            .filter(|p| git::is_partially_staged(&self.project_root, p))
            .collect()
    }

    /// Reload the diff for the current staging view (staged-vs-unstaged or
    /// previous-vs-staged) and refresh the partial-staged set.
    pub(in crate::app) fn refresh_staging_diff(&mut self) {
        let (files, staged_view) = match self.diff_review.as_ref().and_then(|r| r.staging.as_ref())
        {
            Some(s) => (s.files.clone(), s.staged_view),
            None => return,
        };
        let diff = if staged_view {
            git::get_staged_diff(&self.project_root, &files)
        } else {
            git::get_working_diff(&self.project_root, &files)
        };
        let text = diff.unwrap_or_default();
        let parsed = parse_unified_diff(&text);
        let partial = self.compute_partial_staged(&parsed);
        if let Some(review) = self.diff_review.as_mut() {
            review.diff = text;
            review.parsed = parsed;
            review.collapsed_files.clear();
            if let Some(s) = review.staging.as_mut() {
                s.partial = partial;
            }
        }
    }

    /// Flip between the staged-vs-unstaged and previous-vs-staged views.
    pub(in crate::app) fn toggle_staging_view(&mut self) {
        if let Some(s) = self.diff_review.as_mut().and_then(|r| r.staging.as_mut()) {
            s.staged_view = !s.staged_view;
        }
        self.refresh_staging_diff();
    }

    pub(in crate::app) fn stage_review_hunk(&mut self, file_idx: usize, hunk_idx: usize) {
        self.apply_review_hunk(file_idx, hunk_idx, HunkOp::Stage);
    }

    pub(in crate::app) fn discard_review_hunk(&mut self, file_idx: usize, hunk_idx: usize) {
        self.apply_review_hunk(file_idx, hunk_idx, HunkOp::Discard);
    }

    pub(in crate::app) fn unstage_review_hunk(&mut self, file_idx: usize, hunk_idx: usize) {
        self.apply_review_hunk(file_idx, hunk_idx, HunkOp::Unstage);
    }

    /// Build the patch for (file_idx, hunk_idx) from a freshly re-read diff of the
    /// current view — done just before applying to reduce context drift.
    fn staging_hunk_patch(
        &self,
        file_idx: usize,
        hunk_idx: usize,
        staged_view: bool,
        files: &[String],
    ) -> Option<String> {
        let diff = if staged_view {
            git::get_staged_diff(&self.project_root, files)?
        } else {
            git::get_working_diff(&self.project_root, files)?
        };
        let file_diffs = git::split_into_file_diffs(&diff);
        let file = file_diffs.get(file_idx)?;
        git::build_hunk_patch(file, hunk_idx)
    }

    fn apply_review_hunk(&mut self, file_idx: usize, hunk_idx: usize, op: HunkOp) {
        let (files, staged_view) = match self.diff_review.as_ref().and_then(|r| r.staging.as_ref())
        {
            Some(s) => (s.files.clone(), s.staged_view),
            None => return,
        };
        let patch = match self.staging_hunk_patch(file_idx, hunk_idx, staged_view, &files) {
            Some(p) => p,
            None => {
                self.set_status_message("Could not build hunk patch".into());
                return;
            }
        };
        let result = match op {
            HunkOp::Stage => git::stage_hunk(&self.project_root, &patch),
            HunkOp::Unstage => git::unstage_hunk(&self.project_root, &patch),
            HunkOp::Discard => git::discard_hunk(&self.project_root, &patch),
        };
        match result {
            Ok(()) => {
                self.set_status_message(
                    match op {
                        HunkOp::Stage => "Staged hunk",
                        HunkOp::Unstage => "Unstaged hunk",
                        HunkOp::Discard => "Discarded hunk",
                    }
                    .into(),
                );
                self.refresh_staging_diff();
                self.reload_git_info();
            }
            // git apply is atomic: a rejected patch leaves index and tree unchanged.
            Err(e) => self.set_status_message(format!("{e}")),
        }
    }
}
