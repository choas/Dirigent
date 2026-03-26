mod agents_poll;
mod background;
mod claude_run;
mod code_viewer;
mod cue_pool;
mod dialog;
mod file_navigation;
mod git_operations;
mod lava_lamp;
mod markdown_parser;
mod markdown_viewer;
mod notifications;
mod panels;
mod rendering;
mod repo_management;
mod search;
mod sources_poll;
pub(super) mod symbols;
mod tasks;
mod theme;
mod types;
pub(crate) mod util;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::{Duration, Instant};

// -- Timing constants --
const FS_RESCAN_DEBOUNCE: Duration = Duration::from_secs(2);
const LOG_SYNC_INTERVAL: Duration = Duration::from_secs(3);
const REPAINT_FAST: Duration = Duration::from_millis(100);
const REPAINT_SLOW: Duration = Duration::from_millis(500);
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
use crate::file_tree::FileTree;
use crate::git;
use crate::settings::{self, SemanticColors, Settings};

// Re-export items from submodules so existing sibling modules can use `super::icon` etc.
use claude_run::ClaudeRunState;
use sources_poll::SourceState;
use tasks::TaskHandle;
use theme::{icon, icon_small};
use types::{
    create_tab_state, CodeViewerState, CueAction, DiffReview, EditingCue, GitState,
    NavigationHistory, PendingPlay, SearchState,
};

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
    archived_cue_limit: usize,

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
    sources_expanded: bool,
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
    opencode_models_loading: bool,
    opencode_models_rx: mpsc::Receiver<Vec<String>>,

    // Agent run history cleanup tracking
    last_agent_cleanup: Instant,

    // Run queue: cues waiting to run after all running cues finish (FIFO order)
    run_queue: Vec<i64>,

    // Follow-up prompts queued for currently running cues (cue_id -> FIFO list of prompts)
    follow_up_queue: HashMap<i64, Vec<String>>,

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

    // Go-to-definition background search
    goto_def_tx: mpsc::Sender<(u64, PathBuf, usize, String)>,
    goto_def_rx: mpsc::Receiver<(u64, PathBuf, usize, String)>,
    goto_def_gen: u64,
    goto_def_cancel: Arc<AtomicBool>,

    // Prompt history search
    prompt_history_query: String,
    prompt_history_results: Vec<crate::db::CueHistoryRow>,
    prompt_history_active: bool,

    // Cached total cost (refreshed when executions complete, avoids SQL aggregate per frame)
    cached_total_cost: f64,

    // Cached latest execution metrics per cue (avoids DB reads during repaint)
    latest_exec_cache: HashMap<i64, crate::db::ExecutionMetrics>,

    // Pending file/directory delete confirmation (path, is_dir)
    pending_file_delete: Option<(PathBuf, bool)>,

    // Inline rename in file tree
    rename_target: Option<PathBuf>,
    rename_buffer: String,
    rename_focus_requested: bool,
}

/// Try to detect a PR number for the current branch using `gh pr view`.
fn detect_pr_number_from_branch(project_root: &std::path::Path, _branch: &str) -> Option<u32> {
    let output = std::process::Command::new("gh")
        .args(["pr", "view", "--json", "number", "-q", ".number"])
        .current_dir(project_root)
        .output()
        .ok()?;
    if output.status.success() {
        let s = String::from_utf8_lossy(&output.stdout);
        s.trim().parse().ok()
    } else {
        None
    }
}

fn start_fs_watcher(
    root: &Path,
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
    watcher.watch(root, RecursiveMode::Recursive).ok()?;
    Some(watcher)
}

impl DirigentApp {
    pub fn new(project_root: PathBuf, skip_scan: bool) -> Self {
        let db = Database::open(&project_root).expect("failed to open database");
        let mut settings = settings::load_settings(&project_root);
        // Apply one-time settings migrations (e.g. updated default plays).
        if db.migrate_settings(&mut settings).unwrap_or_else(|e| {
            eprintln!("settings migration error: {e:#}");
            false
        }) {
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
            let cues = db.all_cues_limited_archived(50).unwrap_or_default();
            let archived_cue_count = db.archived_cue_count().unwrap_or(0);
            let git_info = git::read_git_info(&project_root);
            let dirty_files = git::get_dirty_files(&project_root);
            let ahead_of_remote = git::get_ahead_of_remote(&project_root);
            let commit_history =
                git::read_commit_history(&project_root, 10_usize.max(ahead_of_remote));
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
        let (goto_def_tx, goto_def_rx) = mpsc::channel();

        let syntax_theme = if settings.theme.is_dark() {
            egui_extras::syntax_highlighting::CodeTheme::dark(12.0)
        } else {
            egui_extras::syntax_highlighting::CodeTheme::light(12.0)
        };

        let semantic = settings.theme.semantic_colors();
        let initial_total_cost = db.total_cost().unwrap_or(0.0);
        let initial_exec_cache = db.get_all_latest_execution_metrics().unwrap_or_default();

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
                quick_open_selected: 0,
                show_outline: true,
                scroll_to_heading: None,
            },
            cues,
            archived_cue_count,
            archived_cue_limit: 50,
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
                show_pull_diverged: false,
                show_pull_unmerged: false,
                show_merge_conflicts: false,
                merge_operation: None,
                conflict_files: Vec::new(),
                show_create_pr: false,
                pr_title: String::new(),
                pr_body: String::new(),
                pr_base: String::new(),
                pr_draft: false,
                creating_pr: false,
                pr_rx: None,
                show_import_pr: false,
                import_pr_number: String::new(),
                importing_pr: false,
                importing_pr_start: None,
                import_pr_rx: None,
                notifying_pr: false,
                pr_notify_rx: None,
                archived_dbs: Vec::new(),
                show_archived_dbs: false,
                pending_force_remove: None,
                pending_archive_msg: None,
                pending_delete_archive: None,
            },
            settings,
            semantic,
            show_settings: false,
            needs_theme_apply: true,
            playbook_expanded: false,
            sources_expanded: false,
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
            opencode_models_loading: false,
            opencode_models_rx: mpsc::channel().1,
            last_agent_cleanup: Instant::now(),
            run_queue: Vec::new(),
            follow_up_queue: HashMap::new(),
            scheduled_runs: HashMap::new(),
            schedule_inputs: HashMap::new(),
            tag_inputs: HashMap::new(),
            tag_all_review_input: None,
            lava_lamp_big: false,
            pending_play: None,
            git_init_confirm: None,
            goto_def_tx,
            goto_def_rx,
            goto_def_gen: 0,
            goto_def_cancel: Arc::new(AtomicBool::new(false)),

            prompt_history_query: String::new(),
            prompt_history_results: Vec::new(),
            prompt_history_active: false,

            cached_total_cost: initial_total_cost,
            latest_exec_cache: initial_exec_cache,

            pending_file_delete: None,

            rename_target: None,
            rename_buffer: String::new(),
            rename_focus_requested: false,
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
        self.cues = self
            .db
            .all_cues_limited_archived(self.archived_cue_limit)
            .unwrap_or_default();
        self.archived_cue_count = self.db.archived_cue_count().unwrap_or(0);
        self.latest_exec_cache = self
            .db
            .get_all_latest_execution_metrics()
            .unwrap_or_default();
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
}

impl eframe::App for DirigentApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx();
        // Store egui context so the file watcher can request repaints
        let _ = self.egui_ctx.set(ctx.clone());

        // Apply theme if needed
        self.apply_theme(ctx);

        // Check for filesystem changes and rescan file tree (debounced)
        self.handle_fs_changes();

        // Reap finished/panicked worker threads
        self.reap_tasks();

        // Poll background results (file tree, search, go-to-definition)
        self.poll_background_results();

        // Poll for Claude results
        self.process_claude_results();

        // Process scheduled runs (trigger when their time arrives)
        self.process_scheduled_runs();

        // Process run queue (start next queued cue when no cues are running)
        self.process_run_queue();

        // Poll for git push/pull/PR results
        self.process_push_result();
        self.process_pull_result();
        self.process_pr_result();
        self.process_import_pr_result();
        self.process_pr_notify_result();

        // Poll for agent results (format, lint, build, test)
        self.process_agent_results();

        // Poll external sources for new cues
        self.poll_sources();
        self.process_source_results();

        // Sync logs and periodic cleanup
        self.sync_logs_and_cleanup();

        // Schedule repaint intervals
        self.schedule_repaints(ctx);

        // Handle drag & drop of files onto the window
        self.handle_drag_and_drop(ctx);

        // Handle keyboard shortcuts
        self.handle_global_shortcuts(ctx);

        // Render all panels and dialogs
        self.render_panels_and_dialogs(ui);
    }
}

impl Drop for DirigentApp {
    fn drop(&mut self) {
        self.shutdown_tasks();
    }
}
