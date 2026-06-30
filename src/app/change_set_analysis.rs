//! Change Set Analysis (whole-file v1).
//!
//! Sends the working diff to the user's selected coding-agent CLI, which groups
//! the changed files into logical feature sets. Routing through the full CLI
//! (rather than the Fast LLM) is slower but more precise. Each group is
//! normalized to a file-disjoint partition and surfaced as an ephemeral cue card
//! in the Review column, where the human can stage or commit it. Nothing is
//! committed without explicit confirmation.
//!
//! Modeled on [`super::split_cue`]: the CLI call runs on a background thread, the
//! result comes back over an `mpsc` channel, and the UI is repainted when it
//! lands.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};

use crate::db::CueStatus;
use crate::fast_llm::ChangeSetGroupRaw;
use crate::settings::{CliProvider, Settings, VcsBackend};

use super::DirigentApp;

/// Tag applied to ephemeral change-set Review cues so they can be recognized.
const CHANGE_SET_TAG: &str = "changeset";

/// Diffs larger than this (in characters) skip the model and collapse to a
/// single "All changes" group, guarding against poor or expensive groupings.
const OVERSIZED_DIFF_CHARS: usize = 60_000;

/// A normalized, file-disjoint change-set group ready to surface as a Review cue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChangeSetGroup {
    pub title: String,
    pub description: String,
    pub files: Vec<String>,
}

/// Outcome of a change-set analysis run.
pub(super) enum ChangeSetOutcome {
    /// One or more groups ready to surface as Review cards.
    Groups(Vec<ChangeSetGroup>),
    /// The working tree was clean / produced no groups.
    Nothing,
}

pub(super) type ChangeSetResult = Result<ChangeSetOutcome, String>;

/// Format the cue text for a change-set Review card: title, description, and the
/// member files, so the human can see exactly what the group covers.
fn change_set_cue_text(group: &ChangeSetGroup) -> String {
    let mut s = if group.title.is_empty() {
        "Change set".to_string()
    } else {
        group.title.clone()
    };
    if !group.description.is_empty() {
        s.push_str("\n\n");
        s.push_str(&group.description);
    }
    s.push_str("\n\nFiles:");
    for f in &group.files {
        s.push_str("\n- ");
        s.push_str(f);
    }
    s
}

/// Cap on the diff embedded in the CLI prompt. Larger working trees collapse to
/// a single "All changes" group via [`OVERSIZED_DIFF_CHARS`] before we get here,
/// so this only guards against the prompt itself growing unwieldy.
const PROMPT_DIFF_CHARS: usize = 40_000;

/// Build the one-shot, read-only prompt asking the coding-agent CLI to group the
/// working diff into logical, file-disjoint change sets and return JSON only.
fn build_change_set_prompt(diff: &str) -> String {
    let trimmed: String = diff.chars().take(PROMPT_DIFF_CHARS).collect();
    format!(
        "You are organizing a messy git working tree. Given the unified diff below, \
         group the changed files into logical, self-describing feature sets.\n\n\
         Output ONLY a JSON array, with no markdown fences and no commentary, shaped like:\n\
         [{{\"title\": \"...\", \"description\": \"...\", \"files\": [{{\"path\": \"...\", \"hunks\": []}}]}}]\n\n\
         Rules:\n\
         - Put each changed file in exactly one group; cover every file in the diff.\n\
         - Write a short imperative title (max ~50 chars) and a one-line description per group.\n\
         - Use the file paths exactly as they appear in the diff.\n\
         - Do NOT modify any files. Only analyze the diff and return JSON.\n\n\
         Diff:\n{trimmed}"
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
    let changed = changed_files_from_diff(&diff);
    if changed.is_empty() {
        return Ok(ChangeSetOutcome::Nothing);
    }
    // Degrade an oversized diff to a single group rather than failing.
    if diff.chars().count() > OVERSIZED_DIFF_CHARS {
        return Ok(ChangeSetOutcome::Groups(vec![ChangeSetGroup {
            title: "All changes".to_string(),
            description: "Working tree too large to group automatically".to_string(),
            files: changed,
        }]));
    }
    let prompt = build_change_set_prompt(&diff);
    let response = super::split_cue::run_cli_prompt(&prompt, provider, project_root, settings, cancel)?;
    let raw = crate::fast_llm::parse_change_sets(&response)?;
    let groups = normalize_change_sets(raw, &changed);
    if groups.is_empty() {
        return Ok(ChangeSetOutcome::Nothing);
    }
    Ok(ChangeSetOutcome::Groups(groups))
}

impl DirigentApp {
    /// Start a Change Set Analysis run: group the dirty working tree into
    /// logical feature sets surfaced as Review cards. Guards on a clean tree up
    /// front so the run never has side effects. The grouping runs through the
    /// user's selected coding-agent CLI — slower than the Fast LLM but more
    /// precise.
    pub(super) fn start_change_set_analysis(&mut self) {
        if self.change_set_generating {
            self.set_status_message("Change-set analysis already in progress".into());
            return;
        }
        if self.git.dirty_files.is_empty() {
            self.set_status_message("Nothing to analyze — the working tree is clean".into());
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

    /// Poll for a completed analysis run and surface the groups as Review cards.
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
                self.set_status_message("Change-set analysis failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.change_set_generating = false;
        self.change_set_rx = None;

        match result {
            Ok(ChangeSetOutcome::Nothing) => {
                self.set_status_message("Nothing to analyze — no changes found".into());
            }
            Ok(ChangeSetOutcome::Groups(groups)) => {
                let mut created = 0usize;
                for group in &groups {
                    let text = change_set_cue_text(group);
                    match self.db.insert_cue(&text, "", 0, None, &[]) {
                        Ok(id) => {
                            let _ = self.db.update_cue_status(id, CueStatus::Review);
                            let _ = self.db.update_cue_tag(id, Some(CHANGE_SET_TAG));
                            self.change_set_files.insert(id, group.files.clone());
                            created += 1;
                        }
                        Err(e) => log::error!("insert change-set cue failed: {e}"),
                    }
                }
                self.reload_cues();
                self.set_status_message(format!(
                    "Analyzed into {created} change set(s) — review in the Review column"
                ));
            }
            Err(e) => {
                self.set_status_message(format!("Change-set analysis failed: {e}"));
            }
        }
    }

    /// Stage exactly a change-set group's files (`git add`), without committing.
    pub(super) fn process_stage_change_set(&mut self, id: i64) {
        let files = match self.change_set_files.get(&id) {
            Some(f) => f.clone(),
            None => return,
        };
        if self.settings.vcs_backend != VcsBackend::Git {
            self.set_status_message("Staging a change set is only available with Git".into());
            return;
        }
        match crate::git::stage_files(&self.project_root, &files) {
            Ok(()) => self.set_status_message(format!("Staged {} file(s)", files.len())),
            Err(e) => self.set_status_message(format!("Stage failed: {e}")),
        }
    }

    /// Open the commit dialog scoped to a change-set group's files, pre-filling a
    /// generated message. The commit only runs when the human confirms; on
    /// success [`process_commit_result`](Self::process_commit_result) marks the
    /// card done and clears it.
    pub(super) fn process_commit_change_set(&mut self, id: i64) {
        let files = match self.change_set_files.get(&id) {
            Some(f) => f.clone(),
            None => return,
        };
        let title = self
            .cues
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.text.lines().next())
            .unwrap_or("Change set")
            .to_string();

        self.git.commit_files = files;
        self.git.commit_review_cue_id = None;
        self.git.commit_change_set_cue_id = Some(id);
        self.git.commit_in_background = false;
        self.git.commit_message_input = crate::git::generate_commit_message(&title, None);
        self.git.commit_needs_focus = true;
        self.git.show_commit_dialog = true;
    }

    /// True when a cue is an ephemeral change-set card (drives card buttons).
    pub(in crate::app) fn is_change_set_cue(&self, id: i64) -> bool {
        self.change_set_files.contains_key(&id)
    }

    /// Drop any change-set bookkeeping for a cue that is being removed.
    pub(in crate::app) fn forget_change_set(&mut self, id: i64) {
        self.change_set_files.remove(&id);
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

/// Normalize raw model groups into a file-disjoint partition of `changed_files`.
///
/// Enforces the v1 invariant in code rather than trusting the model:
/// - files the model invented (not in `changed_files`) are dropped;
/// - groups that share any file are merged (union of files, joined title/description);
/// - any changed file the model omitted is collected into a trailing
///   "Other changes" fallback group, so every changed file stays reviewable.
///
/// Output order is stable: groups appear in order of first appearance in `raw`,
/// with the fallback group (if any) last.
pub(super) fn normalize_change_sets(
    raw: Vec<ChangeSetGroupRaw>,
    changed_files: &[String],
) -> Vec<ChangeSetGroup> {
    let changed: std::collections::HashSet<&str> =
        changed_files.iter().map(String::as_str).collect();

    // Build initial groups, keeping only real changed files (deduped, ordered).
    let mut groups: Vec<ChangeSetGroup> = Vec::new();
    for g in raw {
        let mut files: Vec<String> = Vec::new();
        for f in g.files {
            if changed.contains(f.path.as_str()) && !files.contains(&f.path) {
                files.push(f.path);
            }
        }
        if files.is_empty() {
            continue;
        }
        groups.push(ChangeSetGroup {
            title: g.title.trim().to_string(),
            description: g.description.trim().to_string(),
            files,
        });
    }

    // Union-find over group indices: any two groups sharing a file are merged.
    let n = groups.len();
    let mut parent: Vec<usize> = (0..n).collect();
    let mut file_owner: HashMap<&str, usize> = HashMap::new();
    for (i, g) in groups.iter().enumerate() {
        for f in &g.files {
            match file_owner.get(f.as_str()) {
                Some(&j) => union(&mut parent, i, j),
                None => {
                    file_owner.insert(f.as_str(), i);
                }
            }
        }
    }

    // Collapse each group into its root, preserving first-appearance order.
    let mut order: Vec<usize> = Vec::new();
    let mut by_root: HashMap<usize, ChangeSetGroup> = HashMap::new();
    for (i, g) in groups.into_iter().enumerate() {
        let root = find(&mut parent, i);
        match by_root.get_mut(&root) {
            Some(existing) => merge_group(existing, g),
            None => {
                order.push(root);
                by_root.insert(root, g);
            }
        }
    }
    let mut result: Vec<ChangeSetGroup> = order
        .into_iter()
        .filter_map(|root| by_root.remove(&root))
        .collect();

    // Collect any changed file not claimed by a group into a fallback group.
    let claimed: std::collections::HashSet<&str> = result
        .iter()
        .flat_map(|g| g.files.iter().map(String::as_str))
        .collect();
    let leftover: Vec<String> = changed_files
        .iter()
        .filter(|f| !claimed.contains(f.as_str()))
        .cloned()
        .collect();
    if !leftover.is_empty() {
        result.push(ChangeSetGroup {
            title: "Other changes".to_string(),
            description: "Files not assigned to a specific group".to_string(),
            files: leftover,
        });
    }

    result
}

/// Merge `src` into `dst`: union files (order-stable, deduped) and join the
/// title/description so no information is lost when overlapping groups collapse.
fn merge_group(dst: &mut ChangeSetGroup, src: ChangeSetGroup) {
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
    for f in src.files {
        if !dst.files.contains(&f) {
            dst.files.push(f);
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

    #[test]
    fn disjoint_groups_pass_through() {
        let changed = vec!["a.rs".to_string(), "b.rs".to_string()];
        let groups = normalize_change_sets(vec![raw("A", &["a.rs"]), raw("B", &["b.rs"])], &changed);
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
        );
        assert_eq!(groups.len(), 1, "overlapping groups should merge");
        assert_eq!(groups[0].title, "A + B");
        assert_eq!(groups[0].files, vec!["a.rs", "b.rs", "c.rs"]);
    }

    #[test]
    fn omitted_files_go_to_fallback() {
        let changed = vec!["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()];
        // Model only mentions a.rs; b.rs and c.rs must land in the fallback group.
        let groups = normalize_change_sets(vec![raw("A", &["a.rs"])], &changed);
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
        let groups = normalize_change_sets(vec![raw("A", &["a.rs", "ghost.rs"])], &changed);
        assert_eq!(groups.len(), 1);
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
    fn multi_group_yields_one_card_per_group_with_disjoint_files() {
        let changed = vec!["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()];
        let groups =
            normalize_change_sets(vec![raw("A", &["a.rs"]), raw("B", &["b.rs", "c.rs"])], &changed);
        // One Review card per group...
        assert_eq!(groups.len(), 2);
        // ...each cue text lists exactly that group's files and no other's.
        let text_a = change_set_cue_text(&groups[0]);
        let text_b = change_set_cue_text(&groups[1]);
        assert!(text_a.contains("a.rs"));
        assert!(!text_a.contains("b.rs") && !text_a.contains("c.rs"));
        assert!(text_b.contains("b.rs") && text_b.contains("c.rs"));
        assert!(!text_b.contains("a.rs"));
    }

    #[test]
    fn every_changed_file_belongs_to_exactly_one_group() {
        let changed = vec!["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()];
        let groups = normalize_change_sets(
            vec![raw("A", &["a.rs", "b.rs"]), raw("B", &["b.rs", "c.rs"])],
            &changed,
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
