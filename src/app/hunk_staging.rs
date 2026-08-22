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

/// A hunk discard awaiting the user's confirmation.
///
/// Discard sits one small button away from Stage and permanently removes the
/// change from disk (`git apply --reverse`, or an outright file deletion for an
/// untracked file). A misclick used to be unrecoverable, so it now needs an
/// explicit confirmation *and* leaves a `git stash` undo point behind.
pub(crate) struct PendingHunkDiscard {
    pub file_idx: usize,
    pub hunk_idx: usize,
    /// Repository-relative path, shown in the confirmation.
    pub display_path: String,
    /// The hunk's `@@` header, shown so the user can tell which change it is.
    pub header: String,
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
    ///
    /// One `git status` call for the whole repository, intersected with the
    /// diff's files — not two `git diff` subprocesses per file, which turned a
    /// 50-file diff into ~100 synchronous process spawns per staging refresh.
    pub(in crate::app) fn compute_partial_staged(&self, parsed: &ParsedDiff) -> Vec<String> {
        let partial = git::partially_staged_files(&self.project_root);
        parsed
            .files
            .iter()
            .map(|f| f.display_path().to_string())
            .filter(|p| partial.contains(p))
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

    /// Ask before discarding: this is the one hunk action that destroys work.
    pub(in crate::app) fn request_discard_review_hunk(&mut self, file_idx: usize, hunk_idx: usize) {
        let Some(review) = self.diff_review.as_ref() else {
            return;
        };
        let Some(file) = review.parsed.files.get(file_idx) else {
            return;
        };
        let Some(hunk) = file.hunks.get(hunk_idx) else {
            return;
        };
        self.pending_hunk_discard = Some(PendingHunkDiscard {
            file_idx,
            hunk_idx,
            display_path: file.display_path().to_string(),
            header: hunk.header.clone(),
        });
    }

    /// Carry out a confirmed discard, taking a `git stash` undo point first.
    pub(in crate::app) fn confirm_discard_review_hunk(&mut self) {
        let Some(pending) = self.pending_hunk_discard.take() else {
            return;
        };
        let undo = git::snapshot_for_undo(
            &self.project_root,
            &format!(
                "Dirigent: before discarding a hunk in {}",
                pending.display_path
            ),
        );
        self.apply_review_hunk(pending.file_idx, pending.hunk_idx, HunkOp::Discard);
        if let Some(hash) = undo {
            // Appended after apply_review_hunk's own status message so the undo
            // instructions are what the user is left looking at.
            self.set_status_message(format!(
                "Discarded hunk in {} — undo with: git stash apply {}",
                pending.display_path,
                &hash[..hash.len().min(12)]
            ));
        }
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

/// Confirmation dialog for [`PendingHunkDiscard`].
impl DirigentApp {
    pub(in crate::app) fn render_discard_hunk_dialog(&mut self, ctx: &eframe::egui::Context) {
        use eframe::egui;

        let Some(pending) = self.pending_hunk_discard.as_ref() else {
            return;
        };
        let display_path = pending.display_path.clone();
        let header = pending.header.clone();

        let mut confirm = false;
        let mut cancel = false;

        egui::Window::new("Discard Hunk")
            .collapsible(false)
            .resizable(false)
            .default_size([420.0, 0.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.label("Discard this change from your working tree?");
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(&display_path)
                        .monospace()
                        .color(self.semantic.secondary_text),
                );
                ui.label(
                    egui::RichText::new(&header)
                        .monospace()
                        .small()
                        .color(self.semantic.tertiary_text),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(
                        "The edit is removed from disk. Dirigent saves a git stash \
                         entry first so it can be recovered.",
                    )
                    .small(),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(egui::RichText::new("Discard").color(self.semantic.danger))
                            .clicked()
                        {
                            confirm = true;
                        }
                    });
                });
            });

        if cancel {
            self.pending_hunk_discard = None;
        } else if confirm {
            self.confirm_discard_review_hunk();
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
