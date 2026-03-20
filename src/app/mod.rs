mod agents_poll;
mod claude_run;
mod code_viewer;
mod cue_pool;
mod dialog;
mod lava_lamp;
mod markdown_parser;
mod markdown_viewer;
mod notifications;
mod panels;
mod search;
mod sources_poll;
pub(super) mod symbols;
mod tasks;
mod theme;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::{Duration, Instant};

// -- Timing constants --
const FS_RESCAN_DEBOUNCE: Duration = Duration::from_secs(2);
const LOG_SYNC_INTERVAL: Duration = Duration::from_secs(3);
const REPAINT_FAST: Duration = Duration::from_millis(100);
const REPAINT_SLOW: Duration = Duration::from_millis(500);
const SOURCE_POLL_REPAINT: Duration = Duration::from_secs(30);
const ELAPSED_REPAINT: Duration = Duration::from_secs(1);

// -- Spacing scale (4/8/16/24 point grid) --
pub(crate) const SPACE_XS: f32 = 4.0;
pub(crate) const SPACE_SM: f32 = 8.0;
pub(crate) const SPACE_MD: f32 = 16.0;
pub(crate) const SPACE_LG: f32 = 24.0;

// -- UI dimension constants --
const FONT_SCALE_SMALL: f32 = 0.75;
const FONT_SCALE_LINE_NUM: f32 = 0.85;
const FONT_SCALE_SUBHEADING: f32 = 1.15;
const FONT_SCALE_HEADING: f32 = 1.4;
const SEARCH_PANEL_DEFAULT_WIDTH: f32 = 220.0;
const SEARCH_PANEL_MIN_WIDTH: f32 = 150.0;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use eframe::egui;

/// Truncate a string to at most `max_bytes` without panicking on UTF-8 boundaries.
/// Returns a slice that ends at or before `max_bytes` on a valid char boundary.
pub(crate) fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

use crate::agents::AgentRunState;
use crate::db::{Cue, CueStatus, Database};
use crate::diff_view::{DiffViewMode, ParsedDiff};
use crate::file_tree::FileTree;
use crate::git;
use crate::settings::{self, PlayVariable, SemanticColors, Settings};

// Re-export items from submodules so existing sibling modules can use `super::icon` etc.
use claude_run::ClaudeRunState;
use sources_poll::SourceState;
use tasks::TaskHandle;
use theme::{icon, icon_small};

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
struct DiffReview {
    cue_id: i64,
    diff: String,
    cue_text: String,
    parsed: ParsedDiff,
    view_mode: DiffViewMode,
    read_only: bool,
    collapsed_files: HashSet<usize>,
    prompt_expanded: bool,
    reply_text: String,
    search_active: bool,
    search_query: String,
    /// Matches as (file_idx, hunk_idx, line_idx_in_hunk).
    search_matches: Vec<(usize, usize, usize)>,
    search_current: Option<usize>,
}

enum CueAction {
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
    pub(super) _scroll_offset: f32,
    /// Parsed symbols for outline and breadcrumb.
    pub(super) symbols: Vec<symbols::FileSymbol>,
}

/// Navigation history for back/forward.
pub(super) struct NavigationHistory {
    pub(super) entries: Vec<(PathBuf, usize)>, // (file, line)
    pub(super) position: usize,
}

impl NavigationHistory {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            position: 0,
        }
    }

    fn push(&mut self, file: PathBuf, line: usize) {
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

    fn go_back(&mut self) -> Option<(PathBuf, usize)> {
        if self.position > 1 {
            self.position -= 1;
            Some(self.entries[self.position - 1].clone())
        } else {
            None
        }
    }

    fn go_forward(&mut self) -> Option<(PathBuf, usize)> {
        if self.position < self.entries.len() {
            self.position += 1;
            Some(self.entries[self.position - 1].clone())
        } else {
            None
        }
    }
}

/// State for the code viewer panel (multi-tab).
pub(super) struct CodeViewerState {
    pub(super) tabs: Vec<TabState>,
    pub(super) active_tab: Option<usize>,
    pub(super) scroll_to_line: Option<usize>,
    pub(super) syntax_theme: egui_extras::syntax_highlighting::CodeTheme,
    pub(super) nav_history: NavigationHistory,
    /// Whether the quick-open overlay (Cmd+P) is active.
    pub(super) quick_open_active: bool,
    pub(super) quick_open_query: String,
    /// Whether to show the symbol outline in the left panel.
    pub(super) show_outline: bool,
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
}

/// State for in-file and project-wide search.
pub(super) struct SearchState {
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
pub(super) struct EditingCue {
    pub(super) id: i64,
    pub(super) text: String,
    pub(super) focus_requested: bool,
}

/// State for git information, dirty files, commit history, and worktrees.
pub(super) struct GitState {
    pub(super) info: Option<git::GitInfo>,
    /// Relative paths of files with uncommitted changes, mapped to status letter.
    pub(super) dirty_files: HashMap<String, char>,
    /// Commits ahead of the remote tracking branch.
    pub(super) ahead_of_remote: usize,
    pub(super) commit_history: Vec<git::CommitInfo>,
    pub(super) commit_history_total: usize,
    pub(super) commit_history_limit: usize,
    pub(super) show_log: bool,
    pub(super) worktrees: Vec<git::WorktreeInfo>,
    pub(super) new_worktree_name: String,
    pub(super) show_worktree_panel: bool,
    /// Whether a git push is currently in progress.
    pub(super) pushing: bool,
    pub(super) push_rx: Option<mpsc::Receiver<Result<String, String>>>,
    /// Whether a git pull is currently in progress.
    pub(super) pulling: bool,
    pub(super) pull_rx: Option<mpsc::Receiver<Result<String, String>>>,
}

pub struct DirigentApp {
    project_root: PathBuf,
    db: Database,

    // File tree
    file_tree: Option<FileTree>,
    expanded_dirs: HashSet<PathBuf>,
    file_tree_tx: mpsc::Sender<FileTree>,
    file_tree_rx: mpsc::Receiver<FileTree>,
    file_tree_scanning: bool,

    // Code viewer
    pub(super) viewer: CodeViewerState,

    // Cue pool
    cues: Vec<Cue>,
    archived_cue_count: usize,

    // Claude execution & running logs
    pub(super) claude: ClaudeRunState,

    // Diff review modal
    diff_review: Option<DiffReview>,

    // Git state
    pub(super) git: GitState,

    // Settings & theme
    settings: Settings,
    pub(super) semantic: SemanticColors,
    show_settings: bool,
    needs_theme_apply: bool,
    playbook_expanded: bool,
    agents_expanded: bool,
    commands_expanded: bool,
    agents_init_language: crate::agents::AgentLanguage,

    // Global prompt
    global_prompt_input: String,
    global_prompt_images: Vec<PathBuf>,

    // Repo picker
    pub show_repo_picker: bool,
    repo_path_input: String,

    // Inline cue editing
    pub(super) editing_cue: Option<EditingCue>,

    // Reply inputs for Review cues (cue_id -> text)
    pub(super) reply_inputs: HashMap<i64, String>,

    // Reply input for the conversation log view
    pub(super) conversation_reply: String,
    pub(super) conversation_reply_images: Vec<PathBuf>,

    // About dialog
    show_about: bool,
    logo_texture: Option<egui::TextureHandle>,

    // File-system watcher
    _fs_watcher: Option<RecommendedWatcher>,
    fs_changed: Arc<AtomicBool>,
    last_fs_rescan: Instant,
    egui_ctx: Arc<OnceLock<egui::Context>>,

    // Status bar message (auto-dismisses after a few seconds)
    status_message: Option<(String, Instant)>,

    // Source integration
    pub(super) sources: SourceState,

    // Search
    pub(super) search: SearchState,

    // Task lifecycle management
    task_handles: Vec<TaskHandle>,

    // Agent system (format, lint, build, test)
    pub(super) agent_state: AgentRunState,

    // Animation: highlight flash when cue moves between kanban columns
    cue_move_flash: HashMap<i64, Instant>,

    // Cue cards with fully expanded text (for long cues)
    cue_text_expanded: HashSet<i64>,

    // Expanded activity logbooks (cue IDs with open logbook)
    logbook_expanded: HashSet<i64>,

    // Expanded agent output entries in activity logbook (agent_run IDs)
    agent_output_expanded: HashSet<(i64, String)>,

    // Per-cue agent runs viewer (cue ID whose agent runs to show in central panel)
    show_agent_runs_for_cue: Option<i64>,

    // OpenCode models (cached from CLI)
    pub(super) opencode_models: Vec<String>,

    // Agent run history cleanup tracking
    last_agent_cleanup: Instant,

    // Run queue: cues waiting to run after all running cues finish (FIFO order)
    run_queue: Vec<i64>,

    // Scheduled runs: cue_id -> when to trigger
    scheduled_runs: HashMap<i64, Instant>,

    // Schedule input text per cue (visible when toggled)
    schedule_inputs: HashMap<i64, String>,

    // Tag input per cue (visible when toggled via overflow menu)
    tag_inputs: HashMap<i64, String>,

    // Tag input for "Tag All Review" (visible when toggled)
    tag_all_review_input: Option<String>,

    // Lava lamp enlarged toggle
    lava_lamp_big: bool,

    // Pending play with template variables awaiting user input
    pending_play: Option<PendingPlay>,

    // "git init" confirmation: path that is a directory but not a git repo
    git_init_confirm: Option<PathBuf>,
}

fn start_fs_watcher(
    root: &PathBuf,
    changed: &Arc<AtomicBool>,
    egui_ctx: &Arc<OnceLock<egui::Context>>,
) -> Option<RecommendedWatcher> {
    let flag = Arc::clone(changed);
    let ctx = Arc::clone(egui_ctx);
    let mut watcher =
        notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                use notify::EventKind;
                match event.kind {
                    EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(_) => {
                        flag.store(true, Ordering::Relaxed);
                        if let Some(ctx) = ctx.get() {
                            ctx.request_repaint();
                        }
                    }
                    _ => {}
                }
            }
        })
        .ok()?;
    watcher
        .watch(root.as_path(), RecursiveMode::Recursive)
        .ok()?;
    Some(watcher)
}

impl DirigentApp {
    pub fn new(project_root: PathBuf, skip_scan: bool) -> Self {
        let db = Database::open(&project_root).expect("failed to open database");
        let mut settings = settings::load_settings(&project_root);
        // Apply one-time settings migrations (e.g. updated default plays).
        if db.migrate_settings(&mut settings) {
            settings::save_settings(&project_root, &settings);
        }
        // Seed the in-session recent_repos from the global list so the repo
        // picker always shows previously opened projects, even on first launch.
        settings.recent_repos = settings::load_global_recent_projects();

        // When launched from Finder without a project (skip_scan=true), the
        // project_root is $HOME.  Scanning $HOME recursively touches ~/Music,
        // ~/Movies, and ~/Library which triggers the macOS TCC "would like to
        // access Apple Music" permission dialog.  Skip everything that walks
        // the file system until the user picks a real repo.
        let (
            file_tree,
            cues,
            archived_cue_count,
            git_info,
            dirty_files,
            ahead_of_remote,
            commit_history,
            commit_history_total,
            worktrees,
            mut _fs_watcher,
        ) = if skip_scan {
            let _fs_changed_dummy = Arc::new(AtomicBool::new(false));
            (
                None,
                Vec::new(),
                0_usize,
                None,
                HashMap::new(),
                0_usize,
                Vec::new(),
                0_usize,
                Vec::new(),
                None,
            )
        } else {
            let file_tree = FileTree::scan(&project_root).ok();
            let cues = db.all_cues_limited_archived(200).unwrap_or_default();
            let archived_cue_count = db.archived_cue_count().unwrap_or(0);
            let git_info = git::read_git_info(&project_root);
            let dirty_files = git::get_dirty_files(&project_root);
            let ahead_of_remote = git::get_ahead_of_remote(&project_root);
            let commit_history = git::read_commit_history(&project_root, 10);
            let commit_history_total = git::count_commits(&project_root);
            let worktrees = git::list_worktrees(&project_root).unwrap_or_default();
            (
                file_tree,
                cues,
                archived_cue_count,
                git_info,
                dirty_files,
                ahead_of_remote,
                commit_history,
                commit_history_total,
                worktrees,
                None,
            )
        };

        let fs_changed = Arc::new(AtomicBool::new(false));
        let egui_ctx = Arc::new(OnceLock::new());

        // Start the watcher using the same Arcs the app will store,
        // so the watcher can actually signal changes to the app.
        if !skip_scan {
            _fs_watcher = start_fs_watcher(&project_root, &fs_changed, &egui_ctx);
        }

        let (file_tree_tx, file_tree_rx) = mpsc::channel();
        let (search_result_tx, search_result_rx) = mpsc::channel();

        let syntax_theme = if settings.theme.is_dark() {
            egui_extras::syntax_highlighting::CodeTheme::dark(12.0)
        } else {
            egui_extras::syntax_highlighting::CodeTheme::light(12.0)
        };

        let semantic = settings.theme.semantic_colors();

        DirigentApp {
            project_root,
            db,
            file_tree,
            expanded_dirs: HashSet::new(),
            file_tree_tx,
            file_tree_rx,
            file_tree_scanning: false,
            viewer: CodeViewerState {
                tabs: Vec::new(),
                active_tab: None,
                scroll_to_line: None,
                syntax_theme,
                nav_history: NavigationHistory::new(),
                quick_open_active: false,
                quick_open_query: String::new(),
                show_outline: true,
            },
            cues,
            archived_cue_count,
            claude: ClaudeRunState::new(),
            diff_review: None,
            git: GitState {
                info: git_info,
                dirty_files,
                ahead_of_remote,
                commit_history,
                commit_history_total,
                commit_history_limit: 10,
                show_log: false,
                worktrees,
                new_worktree_name: String::new(),
                show_worktree_panel: false,
                pushing: false,
                push_rx: None,
                pulling: false,
                pull_rx: None,
            },
            settings,
            semantic,
            show_settings: false,
            needs_theme_apply: true,
            playbook_expanded: false,
            agents_expanded: false,
            commands_expanded: false,
            agents_init_language: crate::agents::AgentLanguage::Rust,
            global_prompt_input: String::new(),
            global_prompt_images: Vec::new(),
            show_repo_picker: false,
            repo_path_input: String::new(),
            editing_cue: None,
            reply_inputs: HashMap::new(),
            conversation_reply: String::new(),
            conversation_reply_images: Vec::new(),
            show_about: false,
            logo_texture: None,
            _fs_watcher,
            fs_changed,
            last_fs_rescan: Instant::now(),
            egui_ctx,
            status_message: None,
            sources: SourceState::new(),
            search: SearchState {
                in_file_query: String::new(),
                in_file_active: false,
                in_file_matches: Vec::new(),
                in_file_current: None,
                in_file_nav_flash: None,
                in_files_query: String::new(),
                in_files_active: false,
                in_files_results: Vec::new(),
                in_files_searching: false,
                search_result_tx,
                search_result_rx,
            },
            task_handles: Vec::new(),
            agent_state: AgentRunState::new(),
            cue_move_flash: HashMap::new(),
            cue_text_expanded: HashSet::new(),
            logbook_expanded: HashSet::new(),
            agent_output_expanded: HashSet::new(),
            show_agent_runs_for_cue: None,
            opencode_models: Vec::new(),
            last_agent_cleanup: Instant::now(),
            run_queue: Vec::new(),
            scheduled_runs: HashMap::new(),
            schedule_inputs: HashMap::new(),
            tag_inputs: HashMap::new(),
            tag_all_review_input: None,
            lava_lamp_big: false,
            pending_play: None,
            git_init_confirm: None,
        }
    }

    /// Return a short preview of a cue's text (first few words).
    fn cue_preview(&self, cue_id: i64) -> String {
        self.cues
            .iter()
            .find(|c| c.id == cue_id)
            .map(|c| {
                let words: Vec<&str> = c.text.split_whitespace().take(6).collect();
                let mut preview = words.join(" ");
                if c.text.split_whitespace().count() > 6 {
                    preview.push('\u{2026}');
                }
                preview
            })
            .unwrap_or_else(|| format!("Cue #{}", cue_id))
    }

    /// Ensure the logo texture is loaded (lazy, called once).
    fn ensure_logo_texture(&mut self, ctx: &egui::Context) {
        if self.logo_texture.is_none() {
            let png_bytes = include_bytes!("../../assets/logo.png");
            let img = image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png)
                .expect("failed to decode logo.png")
                .into_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let pixels = img.into_raw();
            let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
            self.logo_texture =
                Some(ctx.load_texture("dirigent_logo", color_image, egui::TextureOptions::LINEAR));
        }
    }

    fn set_status_message(&mut self, msg: String) {
        self.status_message = Some((msg, Instant::now()));
    }

    fn format_elapsed(&self, cue_id: i64) -> String {
        if let Some(start) = self.claude.start_times.get(&cue_id) {
            let secs = start.elapsed().as_secs();
            if secs < 60 {
                format!("{}s", secs)
            } else {
                format!("{}:{:02}", secs / 60, secs % 60)
            }
        } else {
            String::new()
        }
    }

    fn reload_file_tree(&mut self) {
        if self.file_tree_scanning {
            return;
        }
        self.file_tree_scanning = true;
        let root = self.project_root.clone();
        let tx = self.file_tree_tx.clone();
        std::thread::spawn(move || {
            if let Ok(tree) = FileTree::scan(&root) {
                let _ = tx.send(tree);
            }
        });
    }

    fn reload_cues(&mut self) {
        self.cues = self.db.all_cues_limited_archived(200).unwrap_or_default();
        self.archived_cue_count = self.db.archived_cue_count().unwrap_or(0);
    }

    /// Start an async git push operation.
    fn start_git_push(&mut self) {
        if self.git.pushing {
            return;
        }
        self.git.pushing = true;
        let (tx, rx) = mpsc::channel();
        self.git.push_rx = Some(rx);
        let root = self.project_root.clone();
        std::thread::spawn(move || {
            let result = git::git_push(&root).map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.set_status_message("Pushing...".to_string());
    }

    /// Check for completed git push.
    fn process_push_result(&mut self) {
        if let Some(ref rx) = self.git.push_rx {
            if let Ok(result) = rx.try_recv() {
                self.git.pushing = false;
                self.git.push_rx = None;
                match result {
                    Ok(msg) => {
                        self.set_status_message(msg);
                        self.reload_git_info();
                        self.reload_commit_history();
                    }
                    Err(e) => {
                        self.set_status_message(format!("Push failed: {}", e));
                    }
                }
            }
        }
    }

    /// Start an async git pull operation.
    fn start_git_pull(&mut self) {
        if self.git.pulling {
            return;
        }
        self.git.pulling = true;
        let (tx, rx) = mpsc::channel();
        self.git.pull_rx = Some(rx);
        let root = self.project_root.clone();
        std::thread::spawn(move || {
            let result = git::git_pull(&root).map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        self.set_status_message("Pulling...".to_string());
    }

    /// Check for completed git pull.
    fn process_pull_result(&mut self) {
        if let Some(ref rx) = self.git.pull_rx {
            if let Ok(result) = rx.try_recv() {
                self.git.pulling = false;
                self.git.pull_rx = None;
                match result {
                    Ok(msg) => {
                        self.set_status_message(msg);
                        self.reload_git_info();
                        self.reload_commit_history();
                    }
                    Err(e) => {
                        self.set_status_message(format!("Pull failed: {}", e));
                    }
                }
            }
        }
    }

    fn reload_git_info(&mut self) {
        self.git.info = git::read_git_info(&self.project_root);
        self.git.dirty_files = git::get_dirty_files(&self.project_root);
        self.git.ahead_of_remote = git::get_ahead_of_remote(&self.project_root);
    }

    fn reload_commit_history(&mut self) {
        self.git.commit_history =
            git::read_commit_history(&self.project_root, self.git.commit_history_limit);
        self.git.commit_history_total = git::count_commits(&self.project_root);
    }

    /// Process scheduled runs: trigger any cue whose scheduled time has arrived.
    fn process_scheduled_runs(&mut self) {
        let now = Instant::now();
        let ready: Vec<i64> = self
            .scheduled_runs
            .iter()
            .filter(|(_, &when)| now >= when)
            .map(|(&id, _)| id)
            .collect();
        for id in ready {
            self.scheduled_runs.remove(&id);
            // Verify cue is still in Inbox before triggering
            if self
                .cues
                .iter()
                .any(|c| c.id == id && c.status == CueStatus::Inbox)
            {
                let _ = self.db.update_cue_status(id, CueStatus::Ready);
                let _ = self.db.log_activity(id, "Scheduled run started");
                self.cue_move_flash.insert(id, Instant::now());
                self.claude.expand_running = true;
                self.reload_cues();
                self.trigger_claude(id);
            }
        }
    }

    /// Process the run queue: start the next queued cue when no cues are currently running.
    fn process_run_queue(&mut self) {
        if self.run_queue.is_empty() {
            return;
        }
        // Check if any cues are currently running
        let any_running = self.cues.iter().any(|c| c.status == CueStatus::Ready);
        if !any_running {
            let id = self.run_queue.remove(0);
            // Verify cue is still in Inbox before triggering
            if self
                .cues
                .iter()
                .any(|c| c.id == id && c.status == CueStatus::Inbox)
            {
                let _ = self.db.update_cue_status(id, CueStatus::Ready);
                let _ = self.db.log_activity(id, "Queued run started");
                self.cue_move_flash.insert(id, Instant::now());
                self.claude.expand_running = true;
                self.reload_cues();
                self.trigger_claude(id);
            }
        }
    }

    /// Dismiss any overlay that occupies the central panel (settings, diff review, running log)
    /// so the code viewer becomes visible.
    fn dismiss_central_overlays(&mut self) {
        self.show_settings = false;
        self.diff_review = None;
        self.claude.show_log = None;
        self.agent_state.show_output = None;
        self.show_agent_runs_for_cue = None;
    }

    fn load_file(&mut self, path: PathBuf) {
        self.dismiss_central_overlays();

        // If this file is already open in a tab, switch to it
        if let Some(idx) = self.viewer.find_tab(&path) {
            self.viewer.active_tab = Some(idx);
            // Reset in-file search state when switching
            self.search.in_file_active = false;
            self.search.in_file_query.clear();
            self.search.in_file_matches.clear();
            self.search.in_file_current = None;
            return;
        }

        // Read file content and create new tab
        if let Ok(content) = std::fs::read_to_string(&path) {
            let is_markdown = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("mdx"))
                .unwrap_or(false);
            let markdown_blocks = if is_markdown {
                Some(markdown_parser::parse_markdown(&content))
            } else {
                None
            };
            let lines: Vec<String> = content.lines().map(String::from).collect();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_string();
            let file_symbols = symbols::parse_symbols(&lines, &ext);

            let tab = TabState {
                file_path: path,
                content: lines,
                selection_start: None,
                selection_end: None,
                cue_input: String::new(),
                cue_images: Vec::new(),
                markdown_blocks,
                markdown_rendered: true,
                _scroll_offset: 0.0,
                symbols: file_symbols,
            };

            // Soft cap at 20 tabs — close the oldest (first) non-active tab
            if self.viewer.tabs.len() >= 20 {
                let close_idx = self
                    .viewer
                    .tabs
                    .iter()
                    .position(|_| true) // first tab
                    .filter(|&i| Some(i) != self.viewer.active_tab)
                    .unwrap_or(0);
                self.viewer.close_tab(close_idx);
            }

            self.viewer.tabs.push(tab);
            self.viewer.active_tab = Some(self.viewer.tabs.len() - 1);

            // Reset in-file search state for the new file
            self.search.in_file_active = false;
            self.search.in_file_query.clear();
            self.search.in_file_matches.clear();
            self.search.in_file_current = None;
        }
    }

    /// Push the current position onto the navigation history.
    fn push_nav_history(&mut self) {
        if let Some(tab) = self.viewer.active() {
            let line = tab.selection_start.unwrap_or(1);
            self.viewer.nav_history.push(tab.file_path.clone(), line);
        }
    }

    /// Navigate back in history.
    fn nav_back(&mut self) {
        if let Some((path, line)) = self.viewer.nav_history.go_back() {
            // Find or open the file without pushing to history
            if let Some(idx) = self.viewer.find_tab(&path) {
                self.viewer.active_tab = Some(idx);
            } else {
                // Need to load the file
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let is_md = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("mdx"))
                        .unwrap_or(false);
                    let md_blocks = if is_md {
                        Some(markdown_parser::parse_markdown(&content))
                    } else {
                        None
                    };
                    let lines: Vec<String> = content.lines().map(String::from).collect();
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_string();
                    let syms = symbols::parse_symbols(&lines, &ext);
                    self.viewer.tabs.push(TabState {
                        file_path: path,
                        content: lines,
                        selection_start: None,
                        selection_end: None,
                        cue_input: String::new(),
                        cue_images: Vec::new(),
                        markdown_blocks: md_blocks,
                        markdown_rendered: true,
                        _scroll_offset: 0.0,
                        symbols: syms,
                    });
                    self.viewer.active_tab = Some(self.viewer.tabs.len() - 1);
                }
            }
            self.viewer.scroll_to_line = Some(line);
            self.dismiss_central_overlays();
        }
    }

    /// Navigate forward in history.
    fn nav_forward(&mut self) {
        if let Some((path, line)) = self.viewer.nav_history.go_forward() {
            if let Some(idx) = self.viewer.find_tab(&path) {
                self.viewer.active_tab = Some(idx);
            } else {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let is_md = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("mdx"))
                        .unwrap_or(false);
                    let md_blocks = if is_md {
                        Some(markdown_parser::parse_markdown(&content))
                    } else {
                        None
                    };
                    let lines: Vec<String> = content.lines().map(String::from).collect();
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_string();
                    let syms = symbols::parse_symbols(&lines, &ext);
                    self.viewer.tabs.push(TabState {
                        file_path: path,
                        content: lines,
                        selection_start: None,
                        selection_end: None,
                        cue_input: String::new(),
                        cue_images: Vec::new(),
                        markdown_blocks: md_blocks,
                        markdown_rendered: true,
                        _scroll_offset: 0.0,
                        symbols: syms,
                    });
                    self.viewer.active_tab = Some(self.viewer.tabs.len() - 1);
                }
            }
            self.viewer.scroll_to_line = Some(line);
            self.dismiss_central_overlays();
        }
    }

    fn relative_path(&self, path: &PathBuf) -> String {
        path.strip_prefix(&self.project_root)
            .unwrap_or(path.as_path())
            .to_string_lossy()
            .to_string()
    }

    fn file_cues(&self) -> Vec<&Cue> {
        if let Some(current) = self.viewer.current_file() {
            let rel = self.relative_path(current);
            self.cues.iter().filter(|c| c.file_path == rel).collect()
        } else {
            Vec::new()
        }
    }

    /// Returns a map from line number to whether the cue is archived.
    /// `false` = active (yellow dot), `true` = archived (grey dot).
    /// If a line has both active and archived cues, active wins.
    fn lines_with_cues(&self) -> HashMap<usize, bool> {
        let mut map = HashMap::new();
        for c in self.file_cues() {
            let start = c.line_number;
            let end = c.line_number_end.unwrap_or(start);
            let is_archived = c.status == crate::db::CueStatus::Archived;
            for line in start..=end {
                let entry = map.entry(line).or_insert(is_archived);
                // Active cue wins over archived on the same line
                if !is_archived {
                    *entry = false;
                }
            }
        }
        map
    }

    // -- Repo switching --

    fn switch_repo(&mut self, new_root: PathBuf) {
        // Cancel all running tasks — they belong to the old repo.
        self.cancel_all_tasks();
        self.run_queue.clear();
        self.scheduled_runs.clear();
        self.schedule_inputs.clear();

        // Validate that the path is an existing directory
        if !new_root.is_dir() {
            self.set_status_message(format!(
                "Cannot switch repo: not a directory: {}",
                new_root.display()
            ));
            return;
        }
        // Offer to initialize git if not a repository
        if git2::Repository::discover(&new_root).is_err() {
            self.git_init_confirm = Some(new_root);
            return;
        }

        self.db = match Database::open(&new_root) {
            Ok(db) => db,
            Err(e) => {
                self.set_status_message(format!("Failed to open database: {}", e));
                return;
            }
        };
        self.project_root = new_root.clone();
        self.file_tree = FileTree::scan(&self.project_root).ok();
        self.fs_changed.store(false, Ordering::Relaxed);
        self._fs_watcher = start_fs_watcher(&self.project_root, &self.fs_changed, &self.egui_ctx);
        self.cues = self.db.all_cues_limited_archived(200).unwrap_or_default();
        self.archived_cue_count = self.db.archived_cue_count().unwrap_or(0);
        self.git.info = git::read_git_info(&self.project_root);
        self.git.dirty_files = git::get_dirty_files(&self.project_root);
        self.git.ahead_of_remote = git::get_ahead_of_remote(&self.project_root);
        self.viewer.tabs.clear();
        self.viewer.active_tab = None;
        self.viewer.nav_history = NavigationHistory::new();
        self.viewer.quick_open_active = false;
        self.viewer.quick_open_query.clear();
        self.git.commit_history_limit = 10;
        self.git.commit_history = git::read_commit_history(&self.project_root, 10);
        self.git.commit_history_total = git::count_commits(&self.project_root);
        self.expanded_dirs.clear();
        self.diff_review = None;
        self.git.worktrees = git::list_worktrees(&self.project_root).unwrap_or_default();

        // Load project-specific settings if the new repo has them,
        // carrying over recent_repos from the current session.
        let recent_repos = self.settings.recent_repos.clone();
        self.settings = settings::load_settings(&self.project_root);
        self.settings.recent_repos = recent_repos;
        let path_str = new_root.to_string_lossy().to_string();
        settings::add_recent_repo(&mut self.settings, &path_str);
        settings::save_settings(&self.project_root, &self.settings);
        // Persist to global list so every app launch remembers this project.
        settings::add_global_recent_project(&path_str);
        self.needs_theme_apply = true;

        // Update window title to show the new folder name
        if let Some(ctx) = self.egui_ctx.get() {
            let folder = self
                .project_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| self.project_root.to_string_lossy().to_string());
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                "Dirigent - {}",
                folder
            )));
        }
    }

    // -- Worktrees --

    fn reload_worktrees(&mut self) {
        self.git.worktrees = git::list_worktrees(&self.project_root).unwrap_or_default();
    }

    /// Re-read settings from disk (the file may have been changed externally by Claude Code).
    fn reload_settings_from_disk(&mut self) {
        let recent_repos = self.settings.recent_repos.clone();
        self.settings = settings::load_settings(&self.project_root);
        self.settings.recent_repos = recent_repos;
        self.needs_theme_apply = true;
    }

    /// Render project-wide search panel as a left side panel (replaces file tree).
    fn render_search_in_files_panel_wrapper(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("search_files_panel")
            .default_width(SEARCH_PANEL_DEFAULT_WIDTH)
            .min_width(SEARCH_PANEL_MIN_WIDTH)
            .show(ctx, |ui| {
                self.render_search_in_files_panel(ui);
            });
    }
}

impl eframe::App for DirigentApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Store egui context so the file watcher can request repaints
        let _ = self.egui_ctx.set(ctx.clone());

        // Apply theme if needed
        self.apply_theme(ctx);

        // Check for filesystem changes and rescan file tree (debounced)
        if self.fs_changed.load(Ordering::Relaxed)
            && self.last_fs_rescan.elapsed() >= FS_RESCAN_DEBOUNCE
        {
            self.fs_changed.store(false, Ordering::Relaxed);
            self.last_fs_rescan = Instant::now();
            self.reload_file_tree();
            self.git.dirty_files = git::get_dirty_files(&self.project_root);
            self.git.ahead_of_remote = git::get_ahead_of_remote(&self.project_root);
            // Reload all open tabs so the code viewer shows fresh content
            for tab in &mut self.viewer.tabs {
                if let Ok(content) = std::fs::read_to_string(&tab.file_path) {
                    if tab.markdown_blocks.is_some() {
                        tab.markdown_blocks = Some(markdown_parser::parse_markdown(&content));
                    }
                    tab.content = content.lines().map(String::from).collect();
                    let ext = tab
                        .file_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    tab.symbols = symbols::parse_symbols(&tab.content, ext);
                }
            }
            // Trigger agents configured with OnFileChange
            self.trigger_agents_for(&crate::agents::AgentTrigger::OnFileChange, None, "");
        }

        // Reap finished/panicked worker threads
        self.reap_tasks();

        // Check for completed background file tree scan
        if let Ok(tree) = self.file_tree_rx.try_recv() {
            self.file_tree = Some(tree);
            self.file_tree_scanning = false;
        }

        // Check for completed background search
        if let Ok(results) = self.search.search_result_rx.try_recv() {
            self.search.in_files_results = results;
            self.search.in_files_searching = false;
        }

        // Poll for Claude results
        self.process_claude_results();

        // Process scheduled runs (trigger when their time arrives)
        self.process_scheduled_runs();

        // Process run queue (start next queued cue when no cues are running)
        self.process_run_queue();

        // Poll for git push/pull results
        self.process_push_result();
        self.process_pull_result();

        // Poll for agent results (format, lint, build, test)
        self.process_agent_results();

        // Periodic agent run history cleanup (every hour, keep 200 runs per kind, 64KB output max)
        if self.last_agent_cleanup.elapsed() >= Duration::from_secs(3600) {
            self.last_agent_cleanup = Instant::now();
            let _ = self.db.cleanup_agent_runs(200, 65536);
        }

        // Poll external sources for new cues
        self.poll_sources();
        self.process_source_results();

        // Periodically sync running logs to/from DB (every 3s)
        if !self.claude.exec_ids.is_empty() || self.claude.show_log.is_some() {
            if self.claude.last_log_flush.elapsed() >= LOG_SYNC_INTERVAL {
                self.sync_running_logs();
            }
        }

        // Request repaint while Claude tasks are running
        if self.cues.iter().any(|c| c.status == CueStatus::Ready) {
            // Repaint faster when log window is open for live streaming
            let interval = if self.claude.show_log.is_some() {
                REPAINT_FAST
            } else {
                REPAINT_SLOW
            };
            ctx.request_repaint_after(interval);
        } else if !self.run_queue.is_empty() {
            // Repaint to check if queued cues can start (no more running)
            ctx.request_repaint_after(REPAINT_SLOW);
        } else if self.fs_changed.load(Ordering::Relaxed) {
            // Ensure we repaint to pick up filesystem changes after debounce
            ctx.request_repaint_after(FS_RESCAN_DEBOUNCE);
        }
        // Repaint for scheduled runs (countdown display + trigger check)
        if !self.scheduled_runs.is_empty() {
            ctx.request_repaint_after(ELAPSED_REPAINT);
        }
        // Ensure periodic repaint for source polling
        if self
            .settings
            .sources
            .iter()
            .any(|s| s.enabled && s.poll_interval_secs > 0)
        {
            ctx.request_repaint_after(SOURCE_POLL_REPAINT);
        }

        // Handle drag & drop of files onto the window
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            if self.claude.show_log.is_some() {
                self.conversation_reply_images.extend(dropped);
            } else {
                self.global_prompt_images.extend(dropped);
            }
        }

        // Show overlay when files are being dragged over the window
        if !ctx.input(|i| i.raw.hovered_files.is_empty()) {
            let screen = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop_overlay"),
            ));
            painter.rect_filled(screen, 0, egui::Color32::from_black_alpha(160));
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                "Drop files to attach",
                egui::FontId::proportional(24.0),
                egui::Color32::WHITE,
            );
        }

        // Handle keyboard shortcuts for search (Cmd+F, Cmd+Shift+F)
        self.handle_search_shortcuts(ctx);

        // Cmd+N = open a new Dirigent window
        if ctx.input(|i| i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::N)) {
            crate::spawn_new_instance();
        }

        // Cmd+W = close active tab
        if ctx.input(|i| i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::W)) {
            self.viewer.close_active_tab();
        }

        // Cmd+P = quick file open
        if ctx.input(|i| i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::P)) {
            self.viewer.quick_open_active = !self.viewer.quick_open_active;
            self.viewer.quick_open_query.clear();
        }

        // Cmd+[ = navigate back
        if ctx.input(|i| {
            i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::OpenBracket)
        }) {
            self.push_nav_history();
            self.nav_back();
        }

        // Cmd+] = navigate forward
        if ctx.input(|i| {
            i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::CloseBracket)
        }) {
            self.nav_forward();
        }

        // Render all panels (order matters for layout)
        self.render_menu_bar(ctx); // macOS-style menu bar
        self.render_repo_bar(ctx); // top
        self.render_status_bar(ctx); // bottom-most
        self.render_prompt_field(ctx); // above status bar
        if self.search.in_files_active {
            self.render_search_in_files_panel_wrapper(ctx); // replaces file tree
        } else {
            self.render_file_tree_panel(ctx); // left side
        }
        self.render_cue_pool(ctx); // right side
        self.render_code_viewer(ctx); // center (code / diff review / claude progress / settings)

        // Modal overlay dimming behind floating windows — blocks interaction
        if self.show_repo_picker
            || self.git.show_worktree_panel
            || self.show_about
            || self.pending_play.is_some()
        {
            let screen = ctx.content_rect();
            egui::Area::new(egui::Id::new("modal_dim"))
                .order(egui::Order::Middle)
                .fixed_pos(screen.min)
                .show(ctx, |ui| {
                    let (rect, resp) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                    ui.painter()
                        .rect_filled(rect, 0.0, self.semantic.modal_overlay());
                    // Click on overlay dismisses the topmost modal
                    if resp.clicked() {
                        if self.pending_play.is_some() {
                            self.pending_play = None;
                        } else if self.show_about {
                            self.show_about = false;
                        } else if self.git.show_worktree_panel {
                            self.git.show_worktree_panel = false;
                        } else if self.show_repo_picker {
                            self.show_repo_picker = false;
                        }
                    }
                });
        }

        self.render_repo_picker(ctx); // floating
        self.render_worktree_panel(ctx); // floating
        self.render_about_dialog(ctx); // floating
        self.render_play_variables_dialog(ctx); // floating
        self.render_git_init_dialog(ctx); // floating
    }
}

impl Drop for DirigentApp {
    fn drop(&mut self) {
        self.shutdown_tasks();
    }
}
