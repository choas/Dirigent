use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

use crate::db::CueStatus;
use crate::diff_view::{DiffViewMode, ParsedDiff};
use crate::git;
use crate::settings::PlayVariable;

use super::markdown_parser;
use super::search;
use super::symbols;

/// State for a play that has template variables requiring user input.
pub(super) struct PendingPlay {
    /// Original prompt template.
    pub prompt: String,
    /// Parsed template variables.
    pub variables: Vec<PlayVariable>,
    /// Current selected index per variable (into `options`, or options.len() for custom).
    pub selected: Vec<usize>,
    /// Custom text input per variable (used when "Other" is selected or no options).
    pub custom_text: Vec<String>,
    /// Variables that were auto-resolved (index -> resolved value).
    pub auto_resolved: HashMap<usize, String>,
}

/// State for reviewing a diff before accepting/rejecting.
pub(super) struct DiffReview {
    pub(super) cue_id: i64,
    pub(super) diff: String,
    pub(super) cue_text: String,
    pub(super) commit_hash: Option<String>,
    pub(super) parsed: ParsedDiff,
    pub(super) view_mode: DiffViewMode,
    pub(super) read_only: bool,
    pub(super) collapsed_files: HashSet<usize>,
    pub(super) prompt_expanded: bool,
    pub(super) reply_text: String,
    pub(super) search_active: bool,
    pub(super) search_query: String,
    /// Matches as (file_idx, hunk_idx, line_idx_in_hunk).
    pub(super) search_matches: Vec<(usize, usize, usize)>,
    pub(super) search_current: Option<usize>,
}

pub(super) enum CueAction {
    MoveTo(CueStatus),
    Delete,
    StartEdit(String),
    CancelEdit,
    SaveEdit(String),
    Navigate(String, usize, Option<usize>),
    ShowDiff(i64),
    CommitReview(i64),
    RevertReview(i64),
    ReplyReview(i64, String),
    ShowRunningLog(i64),
    ShowAgentRuns(i64),
    CommitAll,
    /// Queue this cue to run after all currently running cues finish.
    QueueNext,
    /// Schedule this cue to run after a delay (e.g. "5m", "2h").
    ScheduleRun(String),
    /// Cancel a queued or scheduled run.
    CancelQueue,
    /// Set (or clear) a tag on a single cue.
    SetTag(Option<String>),
    /// Set a tag on all Review cues at once.
    TagAllReview(String),
    /// Push current branch to remote.
    Push,
    /// Open the Create PR dialog.
    CreatePR,
    /// Notify the original PR comment that a finding was fixed.
    NotifyPR(i64),
    /// Push and notify all Done PR-sourced cues.
    PushAndNotifyPR,
    /// Refresh PR findings (re-import from the same PR).
    RefreshPR,
    /// Queue a follow-up prompt for a currently running cue.
    QueueFollowUp(i64, String),
    /// Open a Claude Code plan file in the code viewer.
    ViewPlan(i64),
    /// Execute a Claude Code plan by sending it back to Claude.
    RunPlan(i64),
    /// Mark a Notion-sourced cue as done in Notion.
    NotionDone(i64),
    /// Trigger LLM analysis of Inbox cues to create a workflow plan.
    CreateWorkflow,
    /// Cancel ongoing workflow generation.
    CancelWorkflow,
    /// Begin executing the workflow plan.
    StartWorkflow,
    /// Resume a paused workflow.
    ResumeWorkflow,
    /// Toggle pause_after on a specific step index.
    TogglePause(usize),
    /// Remove a cue from the workflow plan.
    RemoveFromWorkflow(i64),
    /// Move all Done cues to Archived.
    ArchiveAllDone,
    /// Permanently delete all Archived cues.
    DeleteAllArchived,
}

/// State for a single open file tab.
pub(super) struct TabState {
    pub(super) file_path: PathBuf,
    pub(super) content: Vec<String>,
    /// Start of the selected line range (1-based, always <= selection_end).
    pub(super) selection_start: Option<usize>,
    /// End of the selected line range (1-based, always >= selection_start).
    pub(super) selection_end: Option<usize>,
    pub(super) cue_input: String,
    pub(super) cue_images: Vec<PathBuf>,
    /// Cached parsed markdown blocks (set when a .md/.mdx file is loaded).
    pub(super) markdown_blocks: Option<Vec<markdown_parser::MarkdownBlock>>,
    /// Whether to show the rendered markdown view (true) or raw source (false).
    pub(super) markdown_rendered: bool,
    /// Saved scroll offset so switching tabs preserves position.
    pub(super) scroll_offset: f32,
    /// Parsed symbols for outline and breadcrumb.
    pub(super) symbols: Vec<symbols::FileSymbol>,
    /// Decoded image data for image files (lazily turned into a texture).
    pub(super) image_data: Option<eframe::egui::ColorImage>,
    /// Cached texture handle for the image (created on first render).
    pub(super) image_texture: Option<eframe::egui::TextureHandle>,
    /// Current zoom level for image viewer (1.0 = fit to area).
    pub(super) image_zoom: f32,
}

/// Check if a file extension corresponds to a supported image format.
fn is_image_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico"
    )
}

/// Read a file from disk and build a TabState with markdown parsing and symbol extraction.
pub(super) fn create_tab_state(path: &PathBuf) -> Option<TabState> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();

    // Handle image files: decode into ColorImage instead of reading as text
    if is_image_extension(&ext) {
        let bytes = std::fs::read(path).ok()?;
        let cursor = std::io::Cursor::new(&bytes);
        let mut reader = image::ImageReader::new(cursor).with_guessed_format().ok()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(256 * 1024 * 1024);
        reader.limits(limits);
        let img = reader.decode().ok()?.into_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        let pixels = img.into_raw();
        let color_image = eframe::egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
        return Some(TabState {
            file_path: path.clone(),
            content: Vec::new(),
            selection_start: None,
            selection_end: None,
            cue_input: String::new(),
            cue_images: Vec::new(),
            markdown_blocks: None,
            markdown_rendered: false,
            scroll_offset: 0.0,
            symbols: Vec::new(),
            image_data: Some(color_image),
            image_texture: None,
            image_zoom: 1.0,
        });
    }

    let content = std::fs::read_to_string(path).ok()?;
    let is_md = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("mdx"))
        .unwrap_or(false);
    let markdown_blocks = if is_md {
        Some(markdown_parser::parse_markdown(&content))
    } else {
        None
    };
    let lines: Vec<String> = content.lines().map(String::from).collect();
    let file_symbols = symbols::parse_symbols(&lines, &ext);
    Some(TabState {
        file_path: path.clone(),
        content: lines,
        selection_start: None,
        selection_end: None,
        cue_input: String::new(),
        cue_images: Vec::new(),
        markdown_blocks,
        markdown_rendered: true,
        scroll_offset: 0.0,
        symbols: file_symbols,
        image_data: None,
        image_texture: None,
        image_zoom: 1.0,
    })
}

/// Navigation history for back/forward.
pub(super) struct NavigationHistory {
    pub(super) entries: Vec<(PathBuf, usize)>, // (file, line)
    pub(super) position: usize,
}

impl NavigationHistory {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
            position: 0,
        }
    }

    pub(super) fn push(&mut self, file: PathBuf, line: usize) {
        // If we're not at the end, truncate forward history
        if self.position < self.entries.len() {
            self.entries.truncate(self.position);
        }
        // Don't push duplicate of current position
        if self.entries.last() == Some(&(file.clone(), line)) {
            return;
        }
        self.entries.push((file, line));
        if self.entries.len() > 50 {
            self.entries.remove(0);
        }
        self.position = self.entries.len();
    }

    pub(super) fn go_back(&mut self) -> Option<(PathBuf, usize)> {
        if self.position > 1 {
            self.position -= 1;
            Some(self.entries[self.position - 1].clone())
        } else {
            None
        }
    }

    pub(super) fn go_forward(&mut self) -> Option<(PathBuf, usize)> {
        if self.position < self.entries.len() {
            self.position += 1;
            Some(self.entries[self.position - 1].clone())
        } else {
            None
        }
    }
}

/// State for the code viewer panel (multi-tab).
pub(crate) struct CodeViewerState {
    pub(super) tabs: Vec<TabState>,
    pub(super) active_tab: Option<usize>,
    pub(super) scroll_to_line: Option<usize>,
    pub(super) syntax_theme: egui_extras::syntax_highlighting::CodeTheme,
    pub(super) nav_history: NavigationHistory,
    /// Whether the quick-open overlay (Cmd+P) is active.
    pub(super) quick_open_active: bool,
    pub(super) quick_open_query: String,
    pub(super) quick_open_selected: usize,
    /// Whether to show the symbol outline in the left panel.
    pub(super) show_outline: bool,
    /// Scroll to the Nth heading in rendered markdown view (0-based).
    pub(super) scroll_to_heading: Option<usize>,
}

impl CodeViewerState {
    /// Get a reference to the active tab, if any.
    pub(super) fn active(&self) -> Option<&TabState> {
        self.active_tab.and_then(|i| self.tabs.get(i))
    }

    /// Get a mutable reference to the active tab, if any.
    pub(super) fn active_mut(&mut self) -> Option<&mut TabState> {
        self.active_tab.and_then(|i| self.tabs.get_mut(i))
    }

    /// Get the current file path, if a tab is active.
    pub(super) fn current_file(&self) -> Option<&PathBuf> {
        self.active().map(|t| &t.file_path)
    }

    /// Find a tab index by file path.
    pub(super) fn find_tab(&self, path: &PathBuf) -> Option<usize> {
        self.tabs.iter().position(|t| &t.file_path == path)
    }

    /// Find an existing tab or load the file into a new tab (without touching nav history).
    /// Returns the tab index on success.
    pub(super) fn open_file_without_history(&mut self, path: PathBuf) -> Option<usize> {
        if let Some(idx) = self.find_tab(&path) {
            self.active_tab = Some(idx);
            return Some(idx);
        }
        let tab = create_tab_state(&path)?;
        // Soft cap at 20 tabs — close the oldest (first) non-active tab
        if self.tabs.len() >= 20 {
            let close_idx = self
                .tabs
                .iter()
                .enumerate()
                .position(|(i, _)| Some(i) != self.active_tab)
                .unwrap_or(0);
            self.close_tab(close_idx);
        }
        self.tabs.push(tab);
        let idx = self.tabs.len() - 1;
        self.active_tab = Some(idx);
        Some(idx)
    }

    /// Close the active tab and switch to the nearest remaining tab.
    pub(super) fn close_active_tab(&mut self) {
        if let Some(idx) = self.active_tab {
            self.tabs.remove(idx);
            if self.tabs.is_empty() {
                self.active_tab = None;
            } else if idx >= self.tabs.len() {
                self.active_tab = Some(self.tabs.len() - 1);
            } else {
                self.active_tab = Some(idx);
            }
        }
    }

    /// Close a specific tab by index.
    pub(super) fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        self.tabs.remove(idx);
        match self.active_tab {
            Some(active) if active == idx => {
                if self.tabs.is_empty() {
                    self.active_tab = None;
                } else if idx >= self.tabs.len() {
                    self.active_tab = Some(self.tabs.len() - 1);
                } else {
                    self.active_tab = Some(idx);
                }
            }
            Some(active) if active > idx => {
                self.active_tab = Some(active - 1);
            }
            _ => {}
        }
    }

    /// Close all tabs.
    pub(super) fn close_all_tabs(&mut self) {
        self.tabs.clear();
        self.active_tab = None;
    }

    /// Close all tabs except the one at `keep_idx`.
    pub(super) fn close_other_tabs(&mut self, keep_idx: usize) {
        if keep_idx >= self.tabs.len() {
            return;
        }
        let kept = self.tabs.remove(keep_idx);
        self.tabs.clear();
        self.tabs.push(kept);
        self.active_tab = Some(0);
    }

    /// Close all tabs to the right of `idx` (exclusive).
    pub(super) fn close_tabs_to_right(&mut self, idx: usize) {
        if idx + 1 < self.tabs.len() {
            self.tabs.truncate(idx + 1);
        }
        if let Some(active) = self.active_tab {
            if active > idx {
                self.active_tab = Some(idx);
            }
        }
    }
}

/// State for in-file and project-wide search.
pub(crate) struct SearchState {
    // Search in file (Cmd+F)
    pub(super) in_file_query: String,
    pub(super) in_file_active: bool,
    pub(super) in_file_matches: Vec<usize>,
    pub(super) in_file_current: Option<usize>,
    /// Flash timestamp for search navigation (briefly highlights current match)
    pub(super) in_file_nav_flash: Option<Instant>,

    // Search in files (Cmd+Shift+F)
    pub(super) in_files_query: String,
    pub(super) in_files_active: bool,
    #[allow(private_interfaces)]
    pub(super) in_files_results: Vec<search::SearchResult>,
    pub(super) in_files_searching: bool,
    #[allow(private_interfaces)]
    pub(super) search_result_tx: mpsc::Sender<Vec<search::SearchResult>>,
    #[allow(private_interfaces)]
    pub(super) search_result_rx: mpsc::Receiver<Vec<search::SearchResult>>,
}

/// Inline cue editing state (combined to prevent desync).
pub(crate) struct EditingCue {
    pub(super) id: i64,
    pub(super) text: String,
    pub(super) focus_requested: bool,
}

/// State for git information, dirty files, commit history, and worktrees.
pub(crate) struct GitState {
    pub(super) info: Option<git::GitInfo>,
    /// Relative paths of files with uncommitted changes, mapped to status letter.
    pub(super) dirty_files: HashMap<String, char>,
    /// Commits ahead of the remote tracking branch.
    pub(super) ahead_of_remote: usize,
    pub(super) commit_history: Vec<git::CommitInfo>,
    pub(super) commit_history_total: usize,
    pub(super) commit_history_limit: usize,
    pub(super) show_log: bool,
    /// Graph layout rows corresponding to `commit_history` (one per commit).
    pub(super) graph_rows: Vec<git::graph::GraphRow>,
    /// Maximum number of simultaneous lanes in the current graph.
    pub(super) graph_max_lanes: usize,
    /// Cache key for commit history: (HEAD hash, limit) — skip reload if unchanged.
    pub(super) history_cache_key: (String, usize),
    pub(super) worktrees: Vec<git::WorktreeInfo>,
    pub(super) new_worktree_name: String,
    pub(super) show_worktree_panel: bool,
    /// Branches available for worktree creation (local + remote, excluding checked-out).
    pub(super) available_branches: Vec<String>,
    /// Whether a git push is currently in progress.
    pub(super) pushing: bool,
    pub(super) push_rx: Option<mpsc::Receiver<Result<String, String>>>,
    /// Whether a git pull is currently in progress.
    pub(super) pulling: bool,
    pub(super) pull_rx: Option<mpsc::Receiver<Result<String, String>>>,
    /// Show dialog when pull fails due to diverged branches.
    pub(super) show_pull_diverged: bool,
    /// Show dialog when pull fails due to unmerged files.
    pub(super) show_pull_unmerged: bool,
    /// Show the merge conflict resolution dialog.
    pub(super) show_merge_conflicts: bool,
    /// The type of operation that caused conflicts (merge or rebase).
    pub(super) merge_operation: Option<git::MergeOperation>,
    /// List of files with conflicts (relative paths).
    pub(super) conflict_files: Vec<String>,
    /// Whether the Create PR dialog is open.
    pub(super) show_create_pr: bool,
    /// PR dialog fields.
    pub(super) pr_title: String,
    pub(super) pr_body: String,
    pub(super) pr_base: String,
    pub(super) pr_draft: bool,
    /// Whether a PR creation is in progress.
    pub(super) creating_pr: bool,
    pub(super) pr_rx: Option<mpsc::Receiver<Result<String, String>>>,
    /// Whether the Import PR Findings dialog is open.
    pub(super) show_import_pr: bool,
    /// PR number input for importing findings.
    pub(super) import_pr_number: String,
    /// Whether a PR findings import is in progress.
    pub(super) importing_pr: bool,
    pub(super) importing_pr_start: Option<Instant>,
    pub(super) import_pr_rx: Option<mpsc::Receiver<Result<Vec<crate::sources::PrFinding>, String>>>,
    /// Fetched PR findings awaiting user filtering before import.
    pub(super) pr_findings_pending: Vec<crate::sources::PrFinding>,
    /// Indices of findings the user has excluded from import.
    pub(super) pr_findings_excluded: std::collections::HashSet<usize>,
    /// Whether the PR findings filter dialog is open.
    pub(super) show_pr_filter: bool,
    /// Whether a PR notification (reply to PR comments) is in progress.
    pub(super) notifying_pr: bool,
    pub(super) pr_notify_rx: Option<mpsc::Receiver<Result<String, String>>>,
    /// Whether the "Switch Branch" dialog is open.
    pub(super) show_switch_branch: bool,
    /// Archived worktree DBs (cached list).
    pub(super) archived_dbs: Vec<git::ArchivedDb>,
    /// Whether the archived DBs section is expanded in the worktree panel.
    pub(super) show_archived_dbs: bool,
    /// Pending worktree removal that needs force confirmation (path, error message).
    pub(super) pending_force_remove: Option<(PathBuf, String)>,
    /// Archive message from the archive step (preserved for force-remove flow).
    pub(super) pending_archive_msg: Option<String>,
    /// Pending archived DB deletion that needs user confirmation.
    pub(super) pending_delete_archive: Option<PathBuf>,
    /// Whether the filter dialog is showing the "Patterns" page (true) or "Findings" page (false).
    pub(super) pr_filter_patterns_page: bool,
    /// Cached list of PR filter patterns loaded from the DB.
    pub(super) pr_filter_patterns: Vec<crate::db::PrFilterPattern>,
    /// Input field for adding a new pattern.
    pub(super) new_pattern_text: String,
    /// Match field for new pattern: "text" or "file_path".
    pub(super) new_pattern_field: String,
    /// Pattern id currently being edited (None = not editing).
    pub(super) editing_pattern: Option<(i64, String, String)>,
    /// Row index hovered in the git graph (previous frame), for branch lineage highlight.
    pub(super) hovered_graph_row: Option<usize>,
    /// Whether the Move to Branch dialog is open.
    pub(super) show_move_to_branch: bool,
    /// Whether the text field should receive initial focus on the next frame.
    pub(super) move_to_branch_needs_focus: bool,
    /// Branch name input for the Move to Branch dialog.
    pub(super) move_to_branch_name: String,
    /// Whether a move-to-branch operation is in progress.
    pub(super) moving_to_branch: bool,
    pub(super) move_to_branch_rx: Option<mpsc::Receiver<Result<String, String>>>,
}

impl GitState {
    /// Dismiss the topmost git-related modal dialog (priority order).
    /// Returns `true` if a modal was dismissed.
    pub(super) fn dismiss_topmost_modal(&mut self) -> bool {
        if self.pending_force_remove.is_some() {
            self.pending_force_remove = None;
            return true;
        }
        if self.pending_delete_archive.is_some() {
            self.pending_delete_archive = None;
            return true;
        }
        if self.show_merge_conflicts {
            self.show_merge_conflicts = false;
            return true;
        }
        if self.show_pull_diverged {
            self.show_pull_diverged = false;
            return true;
        }
        if self.show_pull_unmerged {
            self.show_pull_unmerged = false;
            return true;
        }
        if self.show_pr_filter {
            self.show_pr_filter = false;
            self.pr_findings_pending.clear();
            self.pr_findings_excluded.clear();
            return true;
        }
        if self.show_import_pr {
            self.show_import_pr = false;
            return true;
        }
        if self.show_move_to_branch {
            self.show_move_to_branch = false;
            return true;
        }
        if self.show_create_pr {
            self.show_create_pr = false;
            return true;
        }
        if self.show_switch_branch {
            self.show_switch_branch = false;
            return true;
        }
        if self.show_worktree_panel {
            self.show_worktree_panel = false;
            return true;
        }
        false
    }
}
