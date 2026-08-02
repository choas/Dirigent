//! App-side glue for hunk-level staging shown in the diff-review overlay.
//!
//! The staging UI lives in [`super::dialog`]'s diff review; these methods do the
//! git work: reload the staged/unstaged diff, and stage/unstage/discard a single
//! hunk. The UI hands over (file, hunk) indices into the *rendered* diff; they
//! are resolved to a stable (path, `@@` header) key before the patch is built
//! from a freshly re-read diff, so a file appearing, disappearing or shifting
//! between refresh and click can never retarget the operation to another file.

use crate::diff_view::{parse_unified_diff, ParsedDiff};
use crate::git;

use super::DirigentApp;

enum HunkOp {
    Stage,
    Unstage,
    Discard,
}

/// Build a single-hunk patch from a raw multi-file diff, addressing the hunk by
/// (path, `@@` header). Exact header match first, then unambiguous new-side
/// range overlap (via [`git::match_hunk_header`]); `None` when the file or hunk
/// can no longer be found — the caller reports failure instead of guessing.
fn hunk_patch_by_path_and_header(diff: &str, path: &str, header: &str) -> Option<String> {
    let file_diffs = git::split_into_file_diffs(diff);
    let file = file_diffs.iter().find(|f| f.path == path)?;
    let idx = git::match_hunk_header(file, header)?;
    git::build_hunk_patch(file, idx)
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

    /// Build the patch for the hunk shown at (file_idx, hunk_idx) from a freshly
    /// re-read diff of the current view — done just before applying to reduce
    /// context drift.
    ///
    /// The indices address the rendered [`ParsedDiff`], whose file list omits
    /// entries without textual hunks (binary files, mode-only changes, pure
    /// renames), while [`git::split_into_file_diffs`] lists every `diff --git`
    /// section. Indexing the raw list positionally would therefore stage or
    /// discard a hunk of the *wrong file*, so the click is resolved to the
    /// rendered file's path and `@@` header first and re-found by those.
    fn staging_hunk_patch(
        &self,
        file_idx: usize,
        hunk_idx: usize,
        staged_view: bool,
        files: &[String],
    ) -> Option<String> {
        let rendered = &self.diff_review.as_ref()?.parsed;
        let file = rendered.files.get(file_idx)?;
        let hunk = file.hunks.get(hunk_idx)?;
        let diff = if staged_view {
            git::get_staged_diff(&self.project_root, files)?
        } else {
            git::get_working_diff(&self.project_root, files)?
        };
        hunk_patch_by_path_and_header(&diff, file.display_path(), &hunk.header)
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

#[cfg(test)]
mod tests {
    use super::hunk_patch_by_path_and_header;
    use crate::diff_view::parse_unified_diff;

    /// A binary file precedes two text files: `parse_unified_diff` (which drives
    /// the rendered hunk list) drops the binary entry, `split_into_file_diffs`
    /// keeps it, so rendered file indices are shifted by one against the raw list.
    const BINARY_FIRST_DIFF: &str = "diff --git a/img.png b/img.png\n\
        index 1111111..2222222 100644\n\
        Binary files a/img.png and b/img.png differ\n\
        diff --git a/a.rs b/a.rs\n\
        --- a/a.rs\n\
        +++ b/a.rs\n\
        @@ -1,1 +1,1 @@\n\
        -old_a\n\
        +new_a\n\
        diff --git a/z.rs b/z.rs\n\
        --- a/z.rs\n\
        +++ b/z.rs\n\
        @@ -5,1 +5,1 @@\n\
        -old_z\n\
        +new_z\n";

    #[test]
    fn resolves_correct_file_when_binary_file_precedes_text_files() {
        let rendered = parse_unified_diff(BINARY_FIRST_DIFF);
        let paths: Vec<&str> = rendered.files.iter().map(|f| f.display_path()).collect();
        // The rendered list has no img.png entry — positional indexing into the
        // raw split would map rendered z.rs (idx 1) onto raw a.rs (idx 1).
        assert_eq!(paths, ["a.rs", "z.rs"]);

        let z = &rendered.files[1];
        let patch =
            hunk_patch_by_path_and_header(BINARY_FIRST_DIFF, z.display_path(), &z.hunks[0].header)
                .expect("patch for z.rs");
        assert!(patch.contains("+++ b/z.rs") && patch.contains("+new_z"));
        assert!(
            !patch.contains("new_a"),
            "patch must not contain a.rs's hunk"
        );

        let a = &rendered.files[0];
        let patch =
            hunk_patch_by_path_and_header(BINARY_FIRST_DIFF, a.display_path(), &a.hunks[0].header)
                .expect("patch for a.rs");
        assert!(patch.contains("+++ b/a.rs") && patch.contains("+new_a"));
    }

    #[test]
    fn vanished_file_or_hunk_yields_none_instead_of_mistargeting() {
        let rendered = parse_unified_diff(BINARY_FIRST_DIFF);
        let z = &rendered.files[1];
        // File disappeared from the re-read diff (e.g. an agent reverted it).
        let without_z = &BINARY_FIRST_DIFF[..BINARY_FIRST_DIFF.find("diff --git a/z.rs").unwrap()];
        assert_eq!(
            hunk_patch_by_path_and_header(without_z, z.display_path(), &z.hunks[0].header),
            None
        );
        // Hunk no longer present in the file.
        assert_eq!(
            hunk_patch_by_path_and_header(BINARY_FIRST_DIFF, "a.rs", "@@ -99,1 +99,1 @@"),
            None
        );
    }
}
