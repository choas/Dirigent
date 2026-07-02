//! Change Set grouping for split commits (whole-file v1).
//!
//! Backs the "Commit Changes" button: sends the working diff to the user's
//! selected coding-agent CLI, which groups the changed files into logical
//! feature sets. Routing through the full CLI (rather than the Fast LLM) is
//! slower but more precise. Each group is normalized to a file-disjoint
//! partition and queued in the commit dialog ([`GitState::commit_queue`]), where
//! the human reviews each group's message and commits it. Nothing is committed
//! without explicit confirmation.
//!
//! [`GitState::commit_queue`]: super::types::GitState::commit_queue
//!
//! Modeled on [`super::split_cue`]: the CLI call runs on a background thread, the
//! result comes back over an `mpsc` channel, and the UI is repainted when it
//! lands.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};

use crate::fast_llm::ChangeSetGroupRaw;
use crate::git::FileRawDiff;
use crate::settings::{CliProvider, Settings, VcsBackend};

use super::types::CommitGroup;
use super::DirigentApp;

/// Diffs larger than this (in characters) skip the model and collapse to a
/// single "All changes" group, guarding against poor or expensive groupings.
const OVERSIZED_DIFF_CHARS: usize = 60_000;

/// A normalized change-set group ready to queue in the commit dialog.
///
/// `files` lists every owned path (whole-file and partial). `hunk_selection`
/// maps a path to the `@@` headers of the hunks this group owns; a path in
/// `files` but absent from `hunk_selection` is committed whole (v1 behavior).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChangeSetGroup {
    pub title: String,
    pub description: String,
    pub files: Vec<String>,
    pub hunk_selection: HashMap<String, Vec<String>>,
}

/// Outcome of a change-set analysis run.
pub(super) enum ChangeSetOutcome {
    /// One or more groups ready to queue in the commit dialog.
    Groups(Vec<ChangeSetGroup>),
    /// The working tree was clean / produced no groups.
    Nothing,
}

pub(super) type ChangeSetResult = Result<ChangeSetOutcome, String>;

/// Read-only result of analyzing the diff between two refs: the grouped change
/// sets plus the range that produced them.
pub(super) struct AnalyzeOver {
    pub base: String,
    pub head: String,
    pub diff: String,
    pub groups: Vec<ChangeSetGroup>,
}

/// `Ok(None)` means there was nothing to analyze (identical refs / empty range).
pub(super) type AnalyzeOverResult = Result<Option<AnalyzeOver>, String>;

/// Group the diff between two refs off the UI thread, reusing the working-tree
/// grouping helpers. Whole-file grouping only — per-hunk staging is meaningless
/// across branches, so no hunk selection is produced.
fn run_analyze_over(
    settings: &Settings,
    project_root: &Path,
    provider: &CliProvider,
    base: String,
    head: String,
    cancel: Arc<AtomicBool>,
) -> AnalyzeOverResult {
    let diff = match crate::git::get_range_diff(project_root, &base, &head) {
        Some(d) => d,
        None => return Ok(None),
    };
    let file_diffs = crate::git::split_into_file_diffs(&diff);
    let changed = changed_files_from_diff(&diff);
    if changed.is_empty() {
        return Ok(None);
    }
    let prompt = build_change_set_prompt(&file_diffs);
    let response =
        super::split_cue::run_cli_prompt(&prompt, provider, project_root, settings, cancel)?;
    let raw = crate::fast_llm::parse_change_sets(&response)?;
    // Whole-file grouping (empty file_diffs => no hunk selection).
    let groups = normalize_change_sets(raw, &changed, &[]);
    if groups.is_empty() {
        return Ok(None);
    }
    Ok(Some(AnalyzeOver {
        base,
        head,
        diff,
        groups,
    }))
}

/// Cap on the diff embedded in the CLI prompt. Larger working trees collapse to
/// a single "All changes" group via [`OVERSIZED_DIFF_CHARS`] before we get here,
/// so this only guards against the prompt itself growing unwieldy.
const PROMPT_DIFF_CHARS: usize = 40_000;

/// Build the one-shot, read-only prompt asking the coding-agent CLI to group the
/// working diff into logical change sets, assigning hunks per file, and return
/// JSON only. Hunks are presented numbered per file so the model can reference
/// them by their 1-based index.
fn build_change_set_prompt(file_diffs: &[FileRawDiff]) -> String {
    let mut body = String::new();
    let mut budget = PROMPT_DIFF_CHARS;
    for f in file_diffs {
        if f.binary {
            body.push_str(&format!("File: {} (binary — whole file only)\n", f.path));
            continue;
        }
        body.push_str(&format!("File: {}\n", f.path));
        for i in 0..f.hunk_count() {
            if let Some(lines) = f.hunk_lines(i) {
                body.push_str(&format!("  Hunk {}: {}\n", i + 1, lines.first().map(|s| s.as_str()).unwrap_or("")));
                for line in lines.iter().skip(1) {
                    if budget == 0 {
                        break;
                    }
                    body.push_str("    ");
                    body.push_str(line);
                    body.push('\n');
                    budget = budget.saturating_sub(line.len());
                }
            }
        }
    }
    format!(
        "You are organizing a messy git working tree. Below is the working diff, \
         with each file's hunks numbered.\n\n\
         Group the changes into logical, self-describing feature sets. Assign each \
         hunk to exactly one group. When a group owns an entire file, you may list \
         all its hunk numbers or leave \"hunks\" empty.\n\n\
         Output ONLY a JSON array, with no markdown fences and no commentary, shaped like:\n\
         [{{\"title\": \"...\", \"description\": \"...\", \"files\": [{{\"path\": \"...\", \"hunks\": [1, 2]}}]}}]\n\n\
         Rules:\n\
         - \"hunks\" holds the 1-based hunk numbers this group owns for that file; \
         omit it or use [] to take the whole file.\n\
         - Each hunk belongs to exactly one group; cover every hunk.\n\
         - A file may appear in several groups with different hunks.\n\
         - Short imperative title (max ~50 chars) and one-line description per group.\n\
         - Use file paths exactly as shown.\n\
         - Do NOT modify any files. Only analyze and return JSON.\n\n\
         Changes:\n{body}"
    )
}

/// Pure short-circuit decision shared by the run path and tests: return
/// `Nothing` when the working diff is absent or empty, otherwise `None` to
/// proceed to the model.
fn precheck(diff: Option<&str>) -> Option<ChangeSetOutcome> {
    match diff {
        None => Some(ChangeSetOutcome::Nothing),
        Some(d) if d.trim().is_empty() => Some(ChangeSetOutcome::Nothing),
        Some(_) => None,
    }
}

/// Run the analysis off the UI thread: compute the working diff, ask the selected
/// CLI to group it, and normalize to a file-disjoint partition. Returns a pure
/// outcome; no side effects to the working tree are ever made here.
fn run_change_set_analysis(
    settings: &Settings,
    backend: &VcsBackend,
    jj_path: &str,
    project_root: &Path,
    provider: &CliProvider,
    cancel: Arc<AtomicBool>,
) -> ChangeSetResult {
    let diff = super::vcs_dispatch::get_working_diff(backend, jj_path, project_root, &[]);
    if let Some(outcome) = precheck(diff.as_deref()) {
        return Ok(outcome);
    }
    // precheck guarantees the diff is present and non-empty here.
    let diff = diff.expect("diff present after precheck");
    let file_diffs = crate::git::split_into_file_diffs(&diff);
    let changed = changed_files_from_diff(&diff);
    if changed.is_empty() {
        return Ok(ChangeSetOutcome::Nothing);
    }
    // Degrade an oversized diff to a single whole-file group rather than failing.
    if diff.chars().count() > OVERSIZED_DIFF_CHARS {
        return Ok(ChangeSetOutcome::Groups(vec![ChangeSetGroup {
            title: "All changes".to_string(),
            description: "Working tree too large to group automatically".to_string(),
            files: changed,
            hunk_selection: HashMap::new(),
        }]));
    }
    let prompt = build_change_set_prompt(&file_diffs);
    let response =
        super::split_cue::run_cli_prompt(&prompt, provider, project_root, settings, cancel)?;
    let raw = crate::fast_llm::parse_change_sets(&response)?;
    // Partial (per-hunk) commits require the Git index; jj has no staging area,
    // so fall back to whole-file grouping there by hiding the real hunks.
    let fds: &[FileRawDiff] = if matches!(backend, VcsBackend::Git) {
        &file_diffs
    } else {
        &[]
    };
    let groups = normalize_change_sets(raw, &changed, fds);
    if groups.is_empty() {
        return Ok(ChangeSetOutcome::Nothing);
    }
    Ok(ChangeSetOutcome::Groups(groups))
}

impl DirigentApp {
    /// Start a split commit: group the dirty working tree into logical change
    /// sets via the selected coding-agent CLI, then feed them to the commit
    /// dialog's queue so each can be reviewed and committed in turn. Guards on a
    /// clean tree up front so the run never has side effects. The grouping is
    /// slower than the Fast LLM but more precise; while it runs the dialog is not
    /// yet shown — [`process_change_set_result`](Self::process_change_set_result)
    /// opens it once the groups land.
    pub(super) fn start_split_commit(&mut self) {
        if self.change_set_generating {
            self.set_status_message("Already analyzing changes…".into());
            return;
        }
        if self.git.dirty_files.is_empty() {
            self.set_status_message("Nothing to commit — the working tree is clean".into());
            return;
        }

        self.change_set_generating = true;
        self.change_set_cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        self.change_set_rx = Some(rx);

        let settings = self.settings.clone();
        let backend = self.settings.vcs_backend.clone();
        let jj_path = self.settings.jj_cli_path.clone();
        let provider = self.settings.cli_provider.clone();
        let project_root = self.project_root.clone();
        let cancel = Arc::clone(&self.change_set_cancel);
        let ctx = self.egui_ctx.clone();

        std::thread::spawn(move || {
            let result = run_change_set_analysis(
                &settings,
                &backend,
                &jj_path,
                &project_root,
                &provider,
                cancel,
            );
            let _ = tx.send(result);
            if let Some(c) = ctx.get() {
                c.request_repaint();
            }
        });

        self.set_status_message("Analyzing changes with the selected CLI…".into());
    }

    /// Poll for a completed grouping run and open the commit dialog with the
    /// groups queued. A single group still goes through the queue (so the user
    /// gets the same review-then-commit flow); a clean/empty result just reports
    /// status without opening the dialog.
    pub(super) fn process_change_set_result(&mut self) {
        let rx = match self.change_set_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.change_set_generating = false;
                self.change_set_rx = None;
                self.set_status_message("Change analysis failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.change_set_generating = false;
        self.change_set_rx = None;

        match result {
            Ok(ChangeSetOutcome::Nothing) => {
                self.set_status_message("Nothing to commit — no changes found".into());
            }
            Ok(ChangeSetOutcome::Groups(groups)) => {
                self.git.commit_queue = groups
                    .into_iter()
                    .map(|g| CommitGroup {
                        title: g.title,
                        files: g.files,
                        message: String::new(),
                        hunk_selection: g.hunk_selection,
                    })
                    .collect();
                self.git.commit_queue_pos = 0;
                self.open_commit_queue();
            }
            Err(e) => {
                self.set_status_message(format!("Change analysis failed: {e}"));
            }
        }
    }

    /// Open the commit dialog on the first queued group. Clears any single-commit
    /// context (review/selection/background) so the dialog renders in queue mode.
    fn open_commit_queue(&mut self) {
        if self.git.commit_queue.is_empty() {
            return;
        }
        self.git.commit_review_cue_id = None;
        self.git.commit_in_background = false;
        self.git.commit_queue_pos = 0;
        self.git.show_commit_dialog = true;
        self.load_commit_group(0);
        let n = self.git.commit_queue.len();
        self.set_status_message(format!(
            "Split into {n} commit{} — review and commit each",
            if n == 1 { "" } else { "s" }
        ));
    }

    /// Load queued group `pos` into the dialog's editor: scope the commit to its
    /// files and show its message, drafting one from the diff when the group has
    /// not been visited yet.
    pub(in crate::app) fn load_commit_group(&mut self, pos: usize) {
        let group = match self.git.commit_queue.get(pos) {
            Some(g) => g,
            None => return,
        };
        let message = group.message.clone();
        let files = group.files.clone();
        // Cancel any suggestion still in flight for the previously shown group so
        // its late result can't overwrite this group's message.
        self.git
            .commit_suggest_cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.git.commit_suggesting = false;
        self.git.commit_suggest_rx = None;
        self.git.commit_queue_pos = pos;
        self.git.commit_files = files;
        self.git.commit_message_input = message;
        self.git.commit_needs_focus = true;
        // First visit with no message yet: draft one from this group's diff.
        if self.git.commit_message_input.trim().is_empty() {
            self.spawn_commit_message_suggestion();
        }
    }

    /// Persist the edited message back into the current queued group, so it
    /// survives navigating away and back.
    pub(in crate::app) fn save_current_commit_group_message(&mut self) {
        let pos = self.git.commit_queue_pos;
        let msg = self.git.commit_message_input.clone();
        if let Some(group) = self.git.commit_queue.get_mut(pos) {
            group.message = msg;
        }
    }

    /// Stage just the current commit target's files (`git add`), without
    /// committing. Git-only; jj has no separate staging area.
    pub(in crate::app) fn stage_commit_files(&mut self) {
        if self.settings.vcs_backend != VcsBackend::Git {
            self.set_status_message("Staging is only available with Git".into());
            return;
        }
        let files = self.git.commit_files.clone();
        if files.is_empty() {
            self.set_status_message("No files to stage".into());
            return;
        }
        match crate::git::stage_files(&self.project_root, &files) {
            Ok(()) => self.set_status_message(format!("Staged {} file(s)", files.len())),
            Err(e) => self.set_status_message(format!("Stage failed: {e}")),
        }
    }

    /// Start analyzing the diff between two refs (read-only cross-branch review).
    pub(in crate::app) fn start_analyze_over(&mut self, base: String, head: String) {
        if base.is_empty() || head.is_empty() {
            self.set_status_message("Pick a base and a head ref".into());
            return;
        }
        if base == head {
            self.set_status_message("Pick two different refs to compare".into());
            return;
        }
        if self.analyze_over_generating {
            return;
        }
        self.analyze_over_generating = true;
        self.analyze_over_show_picker = false;
        self.analyze_over_result = None;
        self.analyze_over_cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        self.analyze_over_rx = Some(rx);

        let settings = self.settings.clone();
        let provider = self.settings.cli_provider.clone();
        let project_root = self.project_root.clone();
        let cancel = Arc::clone(&self.analyze_over_cancel);
        let ctx = self.egui_ctx.clone();
        let (b, h) = (base.clone(), head.clone());

        std::thread::spawn(move || {
            let result = run_analyze_over(&settings, &project_root, &provider, b, h, cancel);
            let _ = tx.send(result);
            if let Some(c) = ctx.get() {
                c.request_repaint();
            }
        });
        self.set_status_message(format!("Analyzing {base}\u{2026}{head}\u{2026}"));
    }

    /// Poll for a completed range analysis and open the read-only results.
    pub(in crate::app) fn process_analyze_over_result(&mut self) {
        let rx = match self.analyze_over_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.analyze_over_generating = false;
                self.analyze_over_rx = None;
                self.set_status_message("Range analysis failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.analyze_over_generating = false;
        self.analyze_over_rx = None;
        match result {
            Ok(None) => self.set_status_message("Nothing to analyze between those refs".into()),
            Ok(Some(a)) => {
                let n = a.groups.len();
                self.set_status_message(format!(
                    "Grouped {}\u{2026}{} into {n} change set(s)",
                    a.base, a.head
                ));
                self.analyze_over_result = Some(a);
            }
            Err(e) => self.set_status_message(format!("Range analysis failed: {e}")),
        }
    }

    /// Commit the current queued group's owned hunks (v2 partial commit). Whole
    /// files are staged wholly; hunk-subset files have exactly their owned hunks
    /// applied to the index. A hunk whose stored header no longer maps cleanly to
    /// the live diff falls back to committing that file whole, with a warning —
    /// never a guessed range. On success the queue advances.
    pub(in crate::app) fn commit_current_group_partial(&mut self) {
        let msg = self.git.commit_message_input.trim().to_string();
        if msg.is_empty() {
            self.set_status_message("Commit message cannot be empty".into());
            return;
        }
        let pos = self.git.commit_queue_pos;
        let group = match self.git.commit_queue.get(pos) {
            Some(g) => g.clone(),
            None => return,
        };

        // Re-read the live diff so the hunk mapping reflects the current tree.
        let diff = crate::git::get_working_diff(&self.project_root, &[]).unwrap_or_default();
        let file_diffs = crate::git::split_into_file_diffs(&diff);

        let mut whole_files: Vec<String> = Vec::new();
        let mut patches: Vec<String> = Vec::new();
        let mut warned: Vec<String> = Vec::new();

        for path in &group.files {
            match group.hunk_selection.get(path) {
                None => whole_files.push(path.clone()),
                Some(headers) => {
                    let fd = file_diffs.iter().find(|f| &f.path == path);
                    let mut file_patches: Vec<String> = Vec::new();
                    let mut ok = fd.is_some();
                    if let Some(fd) = fd {
                        for h in headers {
                            match crate::git::match_hunk_header(fd, h)
                                .and_then(|idx| crate::git::build_hunk_patch(fd, idx))
                            {
                                Some(p) => file_patches.push(p),
                                None => {
                                    ok = false;
                                    break;
                                }
                            }
                        }
                    }
                    if ok && !file_patches.is_empty() {
                        patches.extend(file_patches);
                    } else {
                        // Mapping failed: commit the whole file rather than guess.
                        whole_files.push(path.clone());
                        warned.push(path.clone());
                    }
                }
            }
        }

        match crate::git::commit_partial(&self.project_root, &whole_files, &patches, &msg) {
            Ok(_) => {
                if warned.is_empty() {
                    self.set_status_message("Committed change set".into());
                } else {
                    self.set_status_message(format!(
                        "Committed; hunk mapping failed for {} — committed whole file",
                        warned.join(", ")
                    ));
                }
                self.advance_commit_queue();
                self.reload_git_info();
                self.reload_commit_history();
            }
            Err(e) => self.set_status_message(format!("Commit failed: {e}")),
        }
    }

    /// After a queued group commits successfully, drop it and advance: reopen the
    /// dialog on the next group, or finish when the queue is empty.
    pub(in crate::app) fn advance_commit_queue(&mut self) {
        let pos = self.git.commit_queue_pos;
        if pos < self.git.commit_queue.len() {
            self.git.commit_queue.remove(pos);
        }
        if self.git.commit_queue.is_empty() {
            self.git.commit_queue_pos = 0;
            self.set_status_message("All change sets committed".into());
            return;
        }
        // `pos` now points at the next group (indices shifted down by the removal).
        let next = pos.min(self.git.commit_queue.len() - 1);
        self.git.show_commit_dialog = true;
        self.load_commit_group(next);
    }
}

/// Extract the set of changed file paths from a unified working diff, preserving
/// first-seen order. For deletions (`new_path == "/dev/null"`) the old path is used.
pub(super) fn changed_files_from_diff(diff: &str) -> Vec<String> {
    let parsed = crate::diff_view::parse_unified_diff(diff);
    let mut files: Vec<String> = Vec::new();
    for f in &parsed.files {
        let path = if f.new_path.is_empty() || f.new_path == "/dev/null" {
            f.old_path.clone()
        } else {
            f.new_path.clone()
        };
        if !path.is_empty() && path != "/dev/null" && !files.contains(&path) {
            files.push(path);
        }
    }
    files
}

/// Intermediate group during normalization: whole-file paths plus per-file
/// partial hunk-header selections.
struct GroupBuild {
    title: String,
    description: String,
    whole: Vec<String>,
    partial: HashMap<String, Vec<String>>,
}

/// Normalize raw model groups into commit-ready groups, resolving per-file hunk
/// assignments (v2) and falling back to whole-file (v1) when needed.
///
/// Guarantees enforced in code, not trusted from the model:
/// - files the model invented (not in `changed_files`) are dropped;
/// - a file wholly claimed anywhere is whole everywhere (partial claims dropped);
/// - groups sharing a whole file are merged;
/// - partial hunk ownership is made disjoint across groups (first-wins);
/// - a changed file owned by no group is collected into a trailing "Other
///   changes" whole-file group. Unassigned hunks of a partially-owned file are
///   simply left unowned (they stay dirty after commits).
pub(super) fn normalize_change_sets(
    raw: Vec<ChangeSetGroupRaw>,
    changed_files: &[String],
    file_diffs: &[FileRawDiff],
) -> Vec<ChangeSetGroup> {
    let changed: HashSet<&str> = changed_files.iter().map(String::as_str).collect();
    let find_file = |path: &str| file_diffs.iter().find(|f| f.path == path);

    // 1. Resolve each raw group into whole/partial ownership.
    let mut builds: Vec<GroupBuild> = Vec::new();
    for g in raw {
        let mut whole: Vec<String> = Vec::new();
        let mut partial: HashMap<String, Vec<String>> = HashMap::new();
        for f in &g.files {
            let path = f.path.clone();
            if !changed.contains(path.as_str()) {
                continue; // hallucinated file
            }
            let ordinals = f.hunk_indices();
            let resolved = if ordinals.is_empty() {
                None // whole file
            } else {
                resolve_hunks(find_file(&path), &ordinals)
            };
            match resolved {
                Some(headers) if !headers.is_empty() => {
                    let e = partial.entry(path).or_default();
                    for h in headers {
                        if !e.contains(&h) {
                            e.push(h);
                        }
                    }
                }
                // Whole file, or hunks that couldn't be resolved -> fall back whole.
                _ => {
                    if !whole.contains(&path) {
                        whole.push(path);
                    }
                }
            }
        }
        if whole.is_empty() && partial.is_empty() {
            continue;
        }
        builds.push(GroupBuild {
            title: g.title.trim().to_string(),
            description: g.description.trim().to_string(),
            whole,
            partial,
        });
    }

    // 2. A path claimed whole anywhere is whole everywhere: drop partial claims.
    let whole_paths: HashSet<String> = builds.iter().flat_map(|b| b.whole.iter().cloned()).collect();
    for b in builds.iter_mut() {
        b.partial.retain(|p, _| !whole_paths.contains(p));
    }

    // 3. Merge groups sharing a whole file (union-find), preserving order.
    let n = builds.len();
    let mut parent: Vec<usize> = (0..n).collect();
    let mut owner: HashMap<&str, usize> = HashMap::new();
    for (i, b) in builds.iter().enumerate() {
        for p in &b.whole {
            match owner.get(p.as_str()) {
                Some(&j) => union(&mut parent, i, j),
                None => {
                    owner.insert(p.as_str(), i);
                }
            }
        }
    }
    let mut order: Vec<usize> = Vec::new();
    let mut by_root: HashMap<usize, GroupBuild> = HashMap::new();
    for (i, b) in builds.into_iter().enumerate() {
        let root = find(&mut parent, i);
        match by_root.get_mut(&root) {
            Some(dst) => merge_build(dst, b),
            None => {
                order.push(root);
                by_root.insert(root, b);
            }
        }
    }
    let mut merged: Vec<GroupBuild> =
        order.into_iter().filter_map(|r| by_root.remove(&r)).collect();

    // 4. Enforce partial-hunk disjointness across groups (first-wins per header).
    let mut claimed: HashMap<String, HashSet<String>> = HashMap::new();
    for b in merged.iter_mut() {
        let mut drop_paths: Vec<String> = Vec::new();
        for (path, headers) in b.partial.iter_mut() {
            let seen = claimed.entry(path.clone()).or_default();
            headers.retain(|h| seen.insert(h.clone()));
            if headers.is_empty() {
                drop_paths.push(path.clone());
            }
        }
        for p in drop_paths {
            b.partial.remove(&p);
        }
    }

    // 5. Convert to ChangeSetGroup; append "Other changes" for omitted files.
    let mut result: Vec<ChangeSetGroup> = merged
        .into_iter()
        .filter_map(|b| {
            let mut files = b.whole.clone();
            for p in b.partial.keys() {
                if !files.contains(p) {
                    files.push(p.clone());
                }
            }
            if files.is_empty() {
                return None;
            }
            Some(ChangeSetGroup {
                title: b.title,
                description: b.description,
                files,
                hunk_selection: b.partial,
            })
        })
        .collect();

    let owned: HashSet<&str> = result
        .iter()
        .flat_map(|g| g.files.iter().map(String::as_str))
        .collect();
    let leftover: Vec<String> = changed_files
        .iter()
        .filter(|f| !owned.contains(f.as_str()))
        .cloned()
        .collect();
    if !leftover.is_empty() {
        result.push(ChangeSetGroup {
            title: "Other changes".to_string(),
            description: "Files not assigned to a specific group".to_string(),
            files: leftover,
            hunk_selection: HashMap::new(),
        });
    }

    result
}

/// Resolve 1-based hunk ordinals to their `@@` headers in `file`. Returns `None`
/// (triggering whole-file fallback) if the file is missing or any ordinal is out
/// of range — never a partial/guessed selection.
fn resolve_hunks(file: Option<&FileRawDiff>, ordinals: &[usize]) -> Option<Vec<String>> {
    let file = file?;
    let mut headers = Vec::new();
    for &ord in ordinals {
        if ord == 0 {
            return None;
        }
        let header = file.hunk_header(ord - 1)?.to_string();
        if !headers.contains(&header) {
            headers.push(header);
        }
    }
    Some(headers)
}

/// Merge `src` into `dst`: union whole files and partial selections, join titles.
fn merge_build(dst: &mut GroupBuild, src: GroupBuild) {
    if !src.title.is_empty() {
        if dst.title.is_empty() {
            dst.title = src.title;
        } else {
            dst.title = format!("{} + {}", dst.title, src.title);
        }
    }
    if !src.description.is_empty() {
        if dst.description.is_empty() {
            dst.description = src.description;
        } else {
            dst.description = format!("{}; {}", dst.description, src.description);
        }
    }
    for f in src.whole {
        if !dst.whole.contains(&f) {
            dst.whole.push(f);
        }
    }
    for (path, headers) in src.partial {
        let e = dst.partial.entry(path).or_default();
        for h in headers {
            if !e.contains(&h) {
                e.push(h);
            }
        }
    }
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let ra = find(parent, a);
    let rb = find(parent, b);
    if ra != rb {
        // Attach the higher index under the lower so the lower (earlier) root
        // wins, keeping first-appearance order stable.
        let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
        parent[hi] = lo;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fast_llm::{ChangeSetFileRaw, ChangeSetGroupRaw};

    fn raw(title: &str, files: &[&str]) -> ChangeSetGroupRaw {
        ChangeSetGroupRaw {
            title: title.to_string(),
            description: format!("desc {title}"),
            files: files
                .iter()
                .map(|p| ChangeSetFileRaw {
                    path: p.to_string(),
                    hunks: vec![],
                })
                .collect(),
        }
    }

    /// A raw group whose single file carries specific 1-based hunk numbers.
    fn raw_hunks(title: &str, path: &str, hunks: &[u64]) -> ChangeSetGroupRaw {
        ChangeSetGroupRaw {
            title: title.to_string(),
            description: format!("desc {title}"),
            files: vec![ChangeSetFileRaw {
                path: path.to_string(),
                hunks: hunks.iter().map(|n| serde_json::json!(n)).collect(),
            }],
        }
    }

    /// A two-hunk file diff for `path`, so ordinals 1/2 resolve to real headers.
    fn two_hunk_file(path: &str) -> Vec<FileRawDiff> {
        let diff = format!(
            "diff --git a/{p} b/{p}\n--- a/{p}\n+++ b/{p}\n@@ -1,1 +1,1 @@\n-a\n+A\n@@ -20,1 +20,1 @@\n-b\n+B\n",
            p = path
        );
        crate::git::split_into_file_diffs(&diff)
    }

    #[test]
    fn disjoint_groups_pass_through() {
        let changed = vec!["a.rs".to_string(), "b.rs".to_string()];
        let groups =
            normalize_change_sets(vec![raw("A", &["a.rs"]), raw("B", &["b.rs"])], &changed, &[]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].title, "A");
        assert_eq!(groups[0].files, vec!["a.rs"]);
        assert_eq!(groups[1].title, "B");
        assert_eq!(groups[1].files, vec!["b.rs"]);
    }

    #[test]
    fn overlapping_groups_merge() {
        let changed = vec!["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()];
        // A and B share a.rs, so they must collapse into one group.
        let groups = normalize_change_sets(
            vec![raw("A", &["a.rs", "b.rs"]), raw("B", &["a.rs", "c.rs"])],
            &changed,
            &[],
        );
        assert_eq!(groups.len(), 1, "overlapping groups should merge");
        assert_eq!(groups[0].title, "A + B");
        assert_eq!(groups[0].files, vec!["a.rs", "b.rs", "c.rs"]);
    }

    #[test]
    fn omitted_files_go_to_fallback() {
        let changed = vec!["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()];
        // Model only mentions a.rs; b.rs and c.rs must land in the fallback group.
        let groups = normalize_change_sets(vec![raw("A", &["a.rs"])], &changed, &[]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].files, vec!["a.rs"]);
        assert_eq!(groups[1].title, "Other changes");
        assert_eq!(groups[1].files, vec!["b.rs", "c.rs"]);
    }

    #[test]
    fn hallucinated_files_are_dropped() {
        let changed = vec!["a.rs".to_string()];
        // Model references a file that isn't actually changed; it must be ignored,
        // and the group keeps only the real file.
        let groups = normalize_change_sets(vec![raw("A", &["a.rs", "ghost.rs"])], &changed, &[]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].files, vec!["a.rs"]);
    }

    #[test]
    fn split_file_across_two_groups_by_hunk() {
        let changed = vec!["a.rs".to_string()];
        let fds = two_hunk_file("a.rs");
        // Group A owns hunk 1, group B owns hunk 2 of the same file.
        let groups = normalize_change_sets(
            vec![raw_hunks("A", "a.rs", &[1]), raw_hunks("B", "a.rs", &[2])],
            &changed,
            &fds,
        );
        assert_eq!(groups.len(), 2, "a split file must not merge its groups");
        // Each group owns the file with a disjoint single-hunk selection.
        assert_eq!(groups[0].hunk_selection["a.rs"], vec!["@@ -1,1 +1,1 @@"]);
        assert_eq!(groups[1].hunk_selection["a.rs"], vec!["@@ -20,1 +20,1 @@"]);
    }

    #[test]
    fn colliding_hunk_is_first_wins() {
        let changed = vec!["a.rs".to_string()];
        let fds = two_hunk_file("a.rs");
        // Both groups claim hunk 1; the second must lose it and be dropped.
        let groups = normalize_change_sets(
            vec![raw_hunks("A", "a.rs", &[1]), raw_hunks("B", "a.rs", &[1])],
            &changed,
            &fds,
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].hunk_selection["a.rs"], vec!["@@ -1,1 +1,1 @@"]);
    }

    #[test]
    fn unresolvable_hunk_falls_back_to_whole_file() {
        let changed = vec!["a.rs".to_string()];
        let fds = two_hunk_file("a.rs");
        // Ordinal 9 is out of range -> whole-file fallback (no hunk_selection).
        let groups = normalize_change_sets(vec![raw_hunks("A", "a.rs", &[9])], &changed, &fds);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].files, vec!["a.rs"]);
        assert!(groups[0].hunk_selection.is_empty(), "must not stage a guessed range");
    }

    #[test]
    fn whole_claim_wins_over_partial_on_same_file() {
        let changed = vec!["a.rs".to_string()];
        let fds = two_hunk_file("a.rs");
        // Group A wants whole a.rs; group B wants just hunk 1 of a.rs.
        // A whole claim makes the file whole everywhere -> groups merge, no partial.
        let groups = normalize_change_sets(
            vec![raw("A", &["a.rs"]), raw_hunks("B", "a.rs", &[1])],
            &changed,
            &fds,
        );
        assert_eq!(groups.len(), 1);
        assert!(groups[0].hunk_selection.is_empty());
        assert_eq!(groups[0].files, vec!["a.rs"]);
    }

    #[test]
    fn precheck_short_circuits() {
        // Nothing on a clean tree (no diff, or empty/whitespace diff).
        assert!(matches!(precheck(None), Some(ChangeSetOutcome::Nothing)));
        assert!(matches!(
            precheck(Some("   \n ")),
            Some(ChangeSetOutcome::Nothing)
        ));
        // Otherwise proceed to the model.
        assert!(precheck(Some("diff --git a/x b/x")).is_none());
    }

    #[test]
    fn multi_group_yields_one_queued_commit_per_group_with_disjoint_files() {
        let changed = vec!["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()];
        let groups = normalize_change_sets(
            vec![raw("A", &["a.rs"]), raw("B", &["b.rs", "c.rs"])],
            &changed,
            &[],
        );
        // One queued commit per group, each scoped to exactly its own files.
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].files, vec!["a.rs"]);
        assert_eq!(groups[1].files, vec!["b.rs", "c.rs"]);
    }

    #[test]
    fn every_changed_file_belongs_to_exactly_one_group() {
        let changed = vec!["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()];
        let groups = normalize_change_sets(
            vec![raw("A", &["a.rs", "b.rs"]), raw("B", &["b.rs", "c.rs"])],
            &changed,
            &[],
        );
        let mut seen: Vec<&str> = groups
            .iter()
            .flat_map(|g| g.files.iter().map(String::as_str))
            .collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len(), "no file may appear in two groups");
        assert_eq!(seen, vec!["a.rs", "b.rs", "c.rs"]);
    }
}
