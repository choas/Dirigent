mod code_viewer;
mod cue_pool;
mod dialogs;
mod panels;
mod search;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

// -- Timing constants --
const FS_RESCAN_DEBOUNCE: Duration = Duration::from_secs(2);
const LOG_SYNC_INTERVAL: Duration = Duration::from_secs(3);
const REPAINT_FAST: Duration = Duration::from_millis(100);
const REPAINT_SLOW: Duration = Duration::from_millis(500);
const SOURCE_POLL_REPAINT: Duration = Duration::from_secs(30);
const ELAPSED_REPAINT: Duration = Duration::from_secs(1);

// -- UI dimension constants --
const FONT_SCALE_SMALL: f32 = 0.75;
const FONT_SCALE_HEADING: f32 = 1.4;
const SEARCH_PANEL_DEFAULT_WIDTH: f32 = 220.0;
const SEARCH_PANEL_MIN_WIDTH: f32 = 150.0;
const COMMIT_MSG_TRUNCATE_LEN: usize = 27;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use eframe::egui;

use crate::claude;
use crate::db::{Cue, CueStatus, Database};
use crate::diff_view::{DiffViewMode, ParsedDiff};
use crate::file_tree::FileTree;
use crate::git;
use crate::settings::{self, Settings, SourceKind};
use crate::sources::{self, SourceItem};

/// Try to load a font from the system by name. Returns (bytes, ttc_index).
fn load_system_font(name: &str) -> Option<(Vec<u8>, u32)> {
    // Known font paths on macOS
    let candidates: &[(&str, u32)] = match name {
        "Menlo" => &[("/System/Library/Fonts/Menlo.ttc", 0)],
        "Monaco" => &[("/System/Library/Fonts/Monaco.ttf", 0)],
        "SF Mono" => &[("/System/Library/Fonts/SFNSMono.ttf", 0)],
        "Courier New" => &[
            ("/System/Library/Fonts/Supplemental/Courier New.ttf", 0),
            ("/Library/Fonts/Courier New.ttf", 0),
        ],
        _ => &[],
    };

    for &(path, index) in candidates {
        if let Ok(data) = std::fs::read(path) {
            return Some((data, index));
        }
    }

    // Try common font directories with various extensions
    let home = std::env::var("HOME").unwrap_or_default();
    let dirs = [
        "/System/Library/Fonts".to_string(),
        "/System/Library/Fonts/Supplemental".to_string(),
        "/Library/Fonts".to_string(),
        format!("{}/Library/Fonts", home),
    ];
    let exts = ["ttf", "ttc", "otf"];

    for dir in &dirs {
        for ext in &exts {
            let path = format!("{}/{}.{}", dir, name, ext);
            if let Ok(data) = std::fs::read(&path) {
                return Some((data, 0));
            }
        }
    }

    None
}

/// Returns a `RichText` using the dedicated icon font (SF Mono) at the given size.
fn icon(text: &str, size: f32) -> egui::RichText {
    egui::RichText::new(text).font(egui::FontId::new(
        size,
        egui::FontFamily::Name("Icons".into()),
    ))
}

/// Returns a `RichText` using the dedicated icon font (SF Mono) at 75% of the given size.
fn icon_small(text: &str, size: f32) -> egui::RichText {
    egui::RichText::new(text).font(egui::FontId::new(
        size * 0.75,
        egui::FontFamily::Name("Icons".into()),
    ))
}

/// Result of a background Claude invocation.
struct ClaudeResult {
    cue_id: i64,
    exec_id: i64,
    diff: Option<String>,
    response: String,
    error: Option<String>,
}

/// A log message from a running Claude worker thread.
struct LogUpdate {
    cue_id: i64,
    text: String,
}

/// Handle to a background task with cancellation support.
struct TaskHandle {
    join_handle: JoinHandle<()>,
    cancel: Arc<AtomicBool>,
    /// Cue ID if this is a Claude execution task (None for source fetches).
    cue_id: Option<i64>,
    /// Execution DB row ID for cleanup on panic.
    exec_id: Option<i64>,
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
    ShowRunningLog(i64),
}

/// State for the code viewer panel.
pub(super) struct CodeViewerState {
    pub(super) current_file: Option<PathBuf>,
    pub(super) content: Vec<String>,
    /// Start of the selected line range (1-based, always <= selection_end).
    pub(super) selection_start: Option<usize>,
    /// End of the selected line range (1-based, always >= selection_start).
    pub(super) selection_end: Option<usize>,
    pub(super) cue_input: String,
    pub(super) scroll_to_line: Option<usize>,
    pub(super) syntax_theme: egui_extras::syntax_highlighting::CodeTheme,
}

/// State for in-file and project-wide search.
pub(super) struct SearchState {
    // Search in file (Cmd+F)
    pub(super) in_file_query: String,
    pub(super) in_file_active: bool,
    pub(super) in_file_matches: Vec<usize>,
    pub(super) in_file_current: Option<usize>,

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

/// State for Claude execution and live log streaming.
pub(super) struct ClaudeRunState {
    tx: mpsc::Sender<ClaudeResult>,
    rx: mpsc::Receiver<ClaudeResult>,
    log_tx: mpsc::Sender<LogUpdate>,
    log_rx: mpsc::Receiver<LogUpdate>,
    pub(super) running_logs: HashMap<i64, String>,
    pub(super) start_times: HashMap<i64, Instant>,
    pub(super) exec_ids: HashMap<i64, i64>,
    pub(super) show_log: Option<i64>,
    pub(super) last_log_flush: Instant,
    /// Expand the "Running" section on next frame (after user clicks Run).
    pub(super) expand_running: bool,
}

/// State for git information, dirty files, commit history, and worktrees.
pub(super) struct GitState {
    pub(super) info: Option<git::GitInfo>,
    /// Relative paths of files with uncommitted changes.
    pub(super) dirty_files: HashSet<String>,
    pub(super) commit_history: Vec<git::CommitInfo>,
    pub(super) show_log: bool,
    pub(super) worktrees: Vec<git::WorktreeInfo>,
    pub(super) new_worktree_name: String,
    pub(super) show_worktree_panel: bool,
}

/// State for external cue source polling.
pub(super) struct SourceState {
    tx: mpsc::Sender<SourceItem>,
    rx: mpsc::Receiver<SourceItem>,
    error_tx: mpsc::Sender<String>,
    error_rx: mpsc::Receiver<String>,
    pub(super) last_poll: HashMap<String, Instant>,
    pub(super) filter: Option<String>,
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

    // Settings
    settings: Settings,
    show_settings: bool,
    needs_theme_apply: bool,

    // Global prompt
    global_prompt_input: String,

    // Repo picker
    show_repo_picker: bool,
    repo_path_input: String,

    // Inline cue editing
    pub(super) editing_cue: Option<EditingCue>,

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
}

fn start_fs_watcher(
    root: &PathBuf,
    changed: &Arc<AtomicBool>,
    egui_ctx: &Arc<OnceLock<egui::Context>>,
) -> Option<RecommendedWatcher> {
    let flag = Arc::clone(changed);
    let ctx = Arc::clone(egui_ctx);
    let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        if let Ok(event) = res {
            use notify::EventKind;
            match event.kind {
                EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
                    flag.store(true, Ordering::Relaxed);
                    if let Some(ctx) = ctx.get() {
                        ctx.request_repaint();
                    }
                }
                _ => {}
            }
        }
    }).ok()?;
    watcher.watch(root.as_path(), RecursiveMode::Recursive).ok()?;
    Some(watcher)
}

impl DirigentApp {
    pub fn new(project_root: PathBuf) -> Self {
        let db = Database::open(&project_root).expect("failed to open database");
        let file_tree = FileTree::scan(&project_root).ok();
        let cues = db.all_cues_limited_archived(200).unwrap_or_default();
        let archived_cue_count = db.archived_cue_count().unwrap_or(0);
        let git_info = git::read_git_info(&project_root);
        let dirty_files = git::get_dirty_files(&project_root);
        let settings = settings::load_settings(&project_root);
        let commit_history = git::read_commit_history(&project_root, 50);
        let worktrees = git::list_worktrees(&project_root).unwrap_or_default();

        let fs_changed = Arc::new(AtomicBool::new(false));
        let egui_ctx = Arc::new(OnceLock::new());
        let _fs_watcher = start_fs_watcher(&project_root, &fs_changed, &egui_ctx);

        let (claude_tx, claude_rx) = mpsc::channel();
        let (source_tx, source_rx) = mpsc::channel();
        let (source_error_tx, source_error_rx) = mpsc::channel();
        let (log_tx, log_rx) = mpsc::channel();
        let (file_tree_tx, file_tree_rx) = mpsc::channel();
        let (search_result_tx, search_result_rx) = mpsc::channel();

        let syntax_theme = if settings.theme.is_dark() {
            egui_extras::syntax_highlighting::CodeTheme::dark(12.0)
        } else {
            egui_extras::syntax_highlighting::CodeTheme::light(12.0)
        };

        DirigentApp {
            project_root,
            db,
            file_tree,
            expanded_dirs: HashSet::new(),
            file_tree_tx,
            file_tree_rx,
            file_tree_scanning: false,
            viewer: CodeViewerState {
                current_file: None,
                content: Vec::new(),
                selection_start: None,
                selection_end: None,
                cue_input: String::new(),
                scroll_to_line: None,
                syntax_theme,
            },
            cues,
            archived_cue_count,
            claude: ClaudeRunState {
                tx: claude_tx,
                rx: claude_rx,
                log_tx,
                log_rx,
                running_logs: HashMap::new(),
                start_times: HashMap::new(),
                exec_ids: HashMap::new(),
                show_log: None,
                last_log_flush: Instant::now(),
                expand_running: false,
            },
            diff_review: None,
            git: GitState {
                info: git_info,
                dirty_files,
                commit_history,
                show_log: false,
                worktrees,
                new_worktree_name: String::new(),
                show_worktree_panel: false,
            },
            settings,
            show_settings: false,
            needs_theme_apply: true,
            global_prompt_input: String::new(),
            show_repo_picker: false,
            repo_path_input: String::new(),
            editing_cue: None,
            show_about: false,
            logo_texture: None,
            _fs_watcher,
            fs_changed,
            last_fs_rescan: Instant::now(),
            egui_ctx,
            status_message: None,
            sources: SourceState {
                tx: source_tx,
                rx: source_rx,
                error_tx: source_error_tx,
                error_rx: source_error_rx,
                last_poll: HashMap::new(),
                filter: None,
            },
            search: SearchState {
                in_file_query: String::new(),
                in_file_active: false,
                in_file_matches: Vec::new(),
                in_file_current: None,
                in_files_query: String::new(),
                in_files_active: false,
                in_files_results: Vec::new(),
                in_files_searching: false,
                search_result_tx,
                search_result_rx,
            },
            task_handles: Vec::new(),
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
            self.logo_texture = Some(ctx.load_texture(
                "dirigent_logo",
                color_image,
                egui::TextureOptions::LINEAR,
            ));
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

    fn reload_git_info(&mut self) {
        self.git.info = git::read_git_info(&self.project_root);
        self.git.dirty_files = git::get_dirty_files(&self.project_root);
    }

    fn reload_commit_history(&mut self) {
        self.git.commit_history = git::read_commit_history(&self.project_root, 50);
    }

    /// Dismiss any overlay that occupies the central panel (settings, diff review, running log)
    /// so the code viewer becomes visible.
    fn dismiss_central_overlays(&mut self) {
        self.show_settings = false;
        self.diff_review = None;
        self.claude.show_log = None;
    }

    fn load_file(&mut self, path: PathBuf) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            self.dismiss_central_overlays();
            self.viewer.content = content.lines().map(String::from).collect();
            self.viewer.current_file = Some(path);
            self.viewer.selection_start = None;
            self.viewer.selection_end = None;
            self.viewer.cue_input.clear();
            // Reset in-file search state for the new file
            self.search.in_file_active = false;
            self.search.in_file_query.clear();
            self.search.in_file_matches.clear();
            self.search.in_file_current = None;
        }
    }

    fn relative_path(&self, path: &PathBuf) -> String {
        path.strip_prefix(&self.project_root)
            .unwrap_or(path.as_path())
            .to_string_lossy()
            .to_string()
    }

    fn file_cues(&self) -> Vec<&Cue> {
        if let Some(ref current) = self.viewer.current_file {
            let rel = self.relative_path(current);
            self.cues
                .iter()
                .filter(|c| c.file_path == rel)
                .collect()
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

    // -- Feature 4: Repo switching --

    fn switch_repo(&mut self, new_root: PathBuf) {
        // Cancel all running tasks — they belong to the old repo.
        self.cancel_all_tasks();

        // Validate that the path is an existing directory
        if !new_root.is_dir() {
            self.set_status_message(format!("Cannot switch repo: not a directory: {}", new_root.display()));
            return;
        }
        // Validate that it's inside a git repository
        if git2::Repository::discover(&new_root).is_err() {
            self.set_status_message(format!("Cannot switch repo: not a git repository: {}", new_root.display()));
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
        self.viewer.current_file = None;
        self.git.commit_history = git::read_commit_history(&self.project_root, 50);
        self.viewer.content.clear();
        self.viewer.selection_start = None;
        self.viewer.selection_end = None;
        self.expanded_dirs.clear();
        self.diff_review = None;
        self.git.worktrees = git::list_worktrees(&self.project_root).unwrap_or_default();

        // Update recent repos
        let path_str = new_root.to_string_lossy().to_string();
        settings::add_recent_repo(&mut self.settings, &path_str);
        settings::save_settings(&self.project_root, &self.settings);
    }

    // -- Feature 5: Worktrees --

    fn reload_worktrees(&mut self) {
        self.git.worktrees = git::list_worktrees(&self.project_root).unwrap_or_default();
    }

    // -- Claude integration --

    fn trigger_claude(&mut self, cue_id: i64) {
        let cue = match self.cues.iter().find(|c| c.id == cue_id) {
            Some(c) => c.clone(),
            None => return,
        };

        let prompt = claude::build_prompt(
            &cue.text,
            &cue.file_path,
            cue.line_number,
            cue.line_number_end,
        );

        // Insert execution record
        let exec_id = self.db.insert_execution(cue_id, &prompt).unwrap_or(0);

        // Initialize log buffer for this cue
        self.claude.running_logs.insert(cue_id, String::new());
        self.claude.start_times.insert(cue_id, Instant::now());
        self.claude.exec_ids.insert(cue_id, exec_id);

        let project_root = self.project_root.clone();
        let claude_tx = self.claude.tx.clone();
        let log_tx = self.claude.log_tx.clone();
        let model = self.settings.claude_model.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_thread = Arc::clone(&cancel);

        let join_handle = std::thread::spawn(move || {
            let on_log = |text: &str| {
                let _ = log_tx.send(LogUpdate {
                    cue_id,
                    text: text.to_string(),
                });
            };
            let result =
                match claude::invoke_claude_streaming(
                    &prompt, &project_root, &model, on_log, cancel_thread,
                ) {
                    Ok(response) => {
                        // Claude Code edits files directly via tools.
                        // Capture the actual changes via git diff on edited files.
                        let diff = if response.edited_files.is_empty() {
                            // Fallback: try parsing response text for a diff
                            claude::parse_diff_from_response(&response.stdout)
                        } else {
                            git::get_working_diff(&project_root, &response.edited_files)
                        };
                        ClaudeResult {
                            cue_id,
                            exec_id,
                            diff,
                            response: response.stdout,
                            error: None,
                        }
                    }
                    Err(e) => ClaudeResult {
                        cue_id,
                        exec_id,
                        diff: None,
                        response: String::new(),
                        error: Some(e.to_string()),
                    },
                };
            let _ = claude_tx.send(result);
        });

        self.task_handles.push(TaskHandle {
            join_handle,
            cancel,
            cue_id: Some(cue_id),
            exec_id: Some(exec_id),
        });
    }

    fn process_claude_results(&mut self) {
        // Drain log channel into local buffers first
        self.drain_log_channel();

        let results: Vec<ClaudeResult> = self.claude.rx.try_iter().collect();

        for result in results {
            // Save the running log to DB before processing
            if let Some(log_text) = self.claude.running_logs.get(&result.cue_id) {
                let _ = self.db.update_execution_log(result.exec_id, log_text);
            }
            // Clean up runtime tracking (keep running_logs for viewing)
            self.claude.exec_ids.remove(&result.cue_id);
            self.claude.start_times.remove(&result.cue_id);

            if let Some(ref error) = result.error {
                let preview = self.cue_preview(result.cue_id);
                self.set_status_message(format!("Claude error for \"{}\": {}", preview, error));
                let _ = self.db.fail_execution(result.exec_id, error);
                let _ = self
                    .db
                    .update_cue_status(result.cue_id, CueStatus::Inbox);
            } else if let Some(ref diff) = result.diff {
                // Claude already edited files directly. Store the diff for review.
                let _ =
                    self.db
                        .complete_execution(result.exec_id, &result.response, Some(diff));
                let _ = self.db.update_cue_status(
                    result.cue_id,
                    CueStatus::Review,
                );
                self.notify_review_ready(result.cue_id);
                // Reload current file so user sees changes
                if let Some(ref path) = self.viewer.current_file {
                    let p = path.clone();
                    self.load_file(p);
                }
                self.reload_git_info();
            } else {
                // Claude ran but no files were changed
                let _ =
                    self.db
                        .complete_execution(result.exec_id, &result.response, None);
                let preview = self.cue_preview(result.cue_id);
                self.set_status_message(format!(
                    "Claude completed but no file changes detected for \"{}\"",
                    preview
                ));
                let _ = self
                    .db
                    .update_cue_status(result.cue_id, CueStatus::Done);
            }
            self.reload_cues();
        }
    }

    /// Drain the log channel, appending text to the per-cue log buffers.
    fn drain_log_channel(&mut self) {
        for update in self.claude.log_rx.try_iter() {
            self.claude.running_logs
                .entry(update.cue_id)
                .or_default()
                .push_str(&update.text);
        }
    }

    /// Periodically flush local running logs to DB (for cross-instance visibility)
    /// and reload remote running logs from DB (for viewing another instance's run).
    fn sync_running_logs(&mut self) {
        self.drain_log_channel();

        // Flush local running logs to DB
        for (&cue_id, log_text) in &self.claude.running_logs {
            if let Some(&exec_id) = self.claude.exec_ids.get(&cue_id) {
                let _ = self.db.update_execution_log(exec_id, log_text);
            }
        }

        // Reload log from DB for the currently viewed cue if it's a remote run
        if let Some(cue_id) = self.claude.show_log {
            if !self.claude.exec_ids.contains_key(&cue_id) {
                let is_running = self
                    .cues
                    .iter()
                    .any(|c| c.id == cue_id && c.status == CueStatus::Ready);
                if is_running {
                    if let Ok(Some(exec)) = self.db.get_latest_execution(cue_id) {
                        if let Some(log_text) = exec.log {
                            self.claude.running_logs.insert(cue_id, log_text);
                        }
                    }
                }
            }
        }

        self.claude.last_log_flush = Instant::now();
    }

    fn notify_review_ready(&self, cue_id: i64) {
        if self.settings.notify_sound {
            std::thread::spawn(|| {
                let _ = Command::new("afplay")
                    .arg("/System/Library/Sounds/Glass.aiff")
                    .output();
            });
        }
        if self.settings.notify_popup {
            let preview = self.cue_preview(cue_id);
            let project_name = self
                .project_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            send_macos_notification("Dirigent", &project_name, &preview);
        }
    }

    // -- Source polling --

    fn poll_sources(&mut self) {
        // Collect sources to poll first to avoid borrow conflict with &mut self.
        let to_poll: Vec<settings::SourceConfig> = self
            .settings
            .sources
            .iter()
            .filter(|s| {
                s.enabled
                    && s.poll_interval_secs > 0
                    && self
                        .sources.last_poll
                        .get(&s.name)
                        .map_or(true, |last| {
                            last.elapsed()
                                >= std::time::Duration::from_secs(s.poll_interval_secs)
                        })
            })
            .cloned()
            .collect();

        for source in to_poll {
            self.sources.last_poll
                .insert(source.name.clone(), Instant::now());
            self.trigger_source_fetch_config(source);
        }
    }

    fn trigger_source_fetch_config(&mut self, source: settings::SourceConfig) {
        let project_root = self.project_root.clone();
        let source_tx = self.sources.tx.clone();
        let error_tx = self.sources.error_tx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_thread = Arc::clone(&cancel);

        let join_handle = std::thread::spawn(move || {
            if cancel_thread.load(Ordering::Relaxed) {
                return;
            }
            let items = match source.kind {
                SourceKind::GitHubIssues => {
                    let label_filter = if source.filter.is_empty() {
                        None
                    } else {
                        Some(source.filter.as_str())
                    };
                    sources::fetch_github_issues(
                        &project_root,
                        label_filter,
                        None,
                        &source.label,
                    )
                    .unwrap_or_else(|e| {
                        let _ = error_tx.send(format!("Source '{}': {}", source.name, e));
                        Vec::new()
                    })
                }
                SourceKind::Custom | SourceKind::Notion | SourceKind::Mcp => {
                    if source.command.is_empty() {
                        Vec::new()
                    } else {
                        sources::fetch_custom_command(
                            &project_root,
                            &source.command,
                            &source.label,
                        )
                        .unwrap_or_else(|e| {
                            let _ = error_tx.send(format!("Source '{}': {}", source.name, e));
                            Vec::new()
                        })
                    }
                }
            };
            if cancel_thread.load(Ordering::Relaxed) {
                return;
            }
            for item in items {
                let _ = source_tx.send(item);
            }
        });

        self.task_handles.push(TaskHandle {
            join_handle,
            cancel,
            cue_id: None,
            exec_id: None,
        });
    }

    /// Trigger a manual fetch for a source by its index in settings.
    pub(super) fn trigger_source_fetch(&mut self, idx: usize) {
        if let Some(source) = self.settings.sources.get(idx).cloned() {
            self.sources.last_poll
                .insert(source.name.clone(), Instant::now());
            let msg = format!("Fetching from \"{}\"...", source.name);
            self.trigger_source_fetch_config(source);
            self.set_status_message(msg);
        }
    }

    fn process_source_results(&mut self) {
        // Surface any source fetch errors to the UI
        if let Ok(err_msg) = self.sources.error_rx.try_recv() {
            self.set_status_message(err_msg);
        }

        let items: Vec<SourceItem> = self.sources.rx.try_iter().collect();

        if items.is_empty() {
            return;
        }

        let mut new_count = 0;
        for item in items {
            match self.db.cue_exists_by_source_ref(&item.external_id) {
                Ok(true) => continue,
                Ok(false) => {}
                Err(_) => continue,
            }
            if self
                .db
                .insert_cue_from_source(&item.text, &item.source_label, &item.external_id)
                .is_ok()
            {
                new_count += 1;
            }
        }
        if new_count > 0 {
            self.reload_cues();
            self.set_status_message(format!("{} new cue(s) from sources", new_count));
        }
    }

    // -- Task lifecycle --

    /// Reap finished tasks: join completed threads, surface panics, clean up
    /// orphaned execution records.
    fn reap_tasks(&mut self) {
        let mut i = 0;
        while i < self.task_handles.len() {
            if self.task_handles[i].join_handle.is_finished() {
                let handle = self.task_handles.swap_remove(i);
                match handle.join_handle.join() {
                    Ok(()) => {
                        // Normal completion — result already sent via channel.
                    }
                    Err(_panic) => {
                        // Thread panicked — mark orphaned execution as failed.
                        if let Some(exec_id) = handle.exec_id {
                            let _ = self.db.fail_execution(exec_id, "worker thread panicked");
                        }
                        if let Some(cue_id) = handle.cue_id {
                            let _ = self.db.update_cue_status(cue_id, CueStatus::Inbox);
                            self.claude.running_logs.remove(&cue_id);
                            self.claude.start_times.remove(&cue_id);
                            self.claude.exec_ids.remove(&cue_id);
                            let preview = self.cue_preview(cue_id);
                            self.set_status_message(format!(
                                "Worker thread panicked for \"{}\"",
                                preview
                            ));
                        }
                        self.reload_cues();
                    }
                }
            } else {
                i += 1;
            }
        }
    }

    /// Signal cancellation for a specific cue's running task.
    fn cancel_cue_task(&mut self, cue_id: i64) {
        for handle in &self.task_handles {
            if handle.cue_id == Some(cue_id) {
                handle.cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Signal cancellation for all running tasks.
    fn cancel_all_tasks(&mut self) {
        for handle in &self.task_handles {
            handle.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Cancel all tasks and block until every worker thread has exited.
    fn shutdown_tasks(&mut self) {
        self.cancel_all_tasks();
        for handle in self.task_handles.drain(..) {
            let _ = handle.join_handle.join();
        }
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

    // -- Theme --

    fn apply_theme(&mut self, ctx: &egui::Context) {
        if !self.needs_theme_apply {
            return;
        }
        self.needs_theme_apply = false;
        ctx.set_visuals(self.settings.theme.visuals());
        self.viewer.syntax_theme = if self.settings.theme.is_dark() {
            egui_extras::syntax_highlighting::CodeTheme::dark(self.settings.font_size)
        } else {
            egui_extras::syntax_highlighting::CodeTheme::light(self.settings.font_size)
        };

        let mut style = (*ctx.style()).clone();
        let font_family = &self.settings.font_family;
        let size = self.settings.font_size;

        // Load the user's chosen font from the system and register it with egui
        let mut font_def = egui::FontDefinitions::default();
        if let Some((font_bytes, index)) = load_system_font(font_family) {
            let mut font_data = egui::FontData::from_owned(font_bytes);
            font_data.index = index;
            font_def.font_data.insert(font_family.clone(), font_data.into());
            font_def.families.entry(egui::FontFamily::Monospace).or_default()
                .insert(0, font_family.clone());
            font_def.families.entry(egui::FontFamily::Proportional).or_default()
                .insert(0, font_family.clone());
        }
        // Add symbol fallback fonts so icons render even when the chosen
        // code font lacks glyphs like ⚙, ❯, ↺, etc.
        // SF Mono has the best coverage for our icon characters, so it comes first.
        let symbol_fonts: &[(&str, &str, u32)] = &[
            ("DiriSymFallback_SFMono", "/System/Library/Fonts/SFNSMono.ttf", 0),
            ("DiriSymFallback_Symbols", "/System/Library/Fonts/Apple Symbols.ttf", 0),
            ("DiriSymFallback_Menlo", "/System/Library/Fonts/Menlo.ttc", 0),
        ];
        for &(name, path, index) in symbol_fonts {
            if let Ok(data) = std::fs::read(path) {
                let mut fd = egui::FontData::from_owned(data);
                fd.index = index;
                font_def.font_data.insert(name.to_string(), fd.into());
                font_def.families.entry(egui::FontFamily::Monospace).or_default()
                    .push(name.to_string());
                font_def.families.entry(egui::FontFamily::Proportional).or_default()
                    .push(name.to_string());
                font_def.families.entry(egui::FontFamily::Name("Icons".into())).or_default()
                    .push(name.to_string());
            }
        }
        // Ensure the "Icons" family always exists so icon() / icon_small() never
        // panic.  When no symbol font was loaded, fall back to Monospace fonts.
        {
            let needs_fallback = font_def.families
                .get(&egui::FontFamily::Name("Icons".into()))
                .map_or(true, |v| v.is_empty());
            if needs_fallback {
                let mono = font_def.families
                    .get(&egui::FontFamily::Monospace)
                    .cloned()
                    .unwrap_or_default();
                font_def.families.insert(egui::FontFamily::Name("Icons".into()), mono);
            }
        }
        ctx.set_fonts(font_def);

        // Scale all text styles based on the chosen font size
        style.text_styles.insert(egui::TextStyle::Small, egui::FontId::new(size * FONT_SCALE_SMALL, egui::FontFamily::Proportional));
        style.text_styles.insert(egui::TextStyle::Body, egui::FontId::new(size, egui::FontFamily::Proportional));
        style.text_styles.insert(egui::TextStyle::Monospace, egui::FontId::new(size, egui::FontFamily::Monospace));
        style.text_styles.insert(egui::TextStyle::Button, egui::FontId::new(size, egui::FontFamily::Proportional));
        style.text_styles.insert(egui::TextStyle::Heading, egui::FontId::new(size * FONT_SCALE_HEADING, egui::FontFamily::Proportional));
        ctx.set_style(style);
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
        if self
            .cues
            .iter()
            .any(|c| c.status == CueStatus::Ready)
        {
            // Repaint faster when log window is open for live streaming
            let interval = if self.claude.show_log.is_some() {
                REPAINT_FAST
            } else {
                REPAINT_SLOW
            };
            ctx.request_repaint_after(interval);
        } else if self.fs_changed.load(Ordering::Relaxed) {
            // Ensure we repaint to pick up filesystem changes after debounce
            ctx.request_repaint_after(FS_RESCAN_DEBOUNCE);
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

        // Handle keyboard shortcuts for search (Cmd+F, Cmd+Shift+F)
        self.handle_search_shortcuts(ctx);

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
        self.render_repo_picker(ctx); // floating
        self.render_worktree_panel(ctx); // floating
        self.render_about_dialog(ctx); // floating
    }
}

impl Drop for DirigentApp {
    fn drop(&mut self) {
        self.shutdown_tasks();
    }
}

/// Send a macOS notification via `UNUserNotificationCenter` (modern API).
/// Clicking the notification activates the Dirigent process that sent it.
/// Falls back to the deprecated `NSUserNotificationCenter`, then to `osascript`.
#[cfg(target_os = "macos")]
fn send_macos_notification(title: &str, subtitle: &str, body: &str) {
    use objc::runtime::{Class, Object};
    use objc::{msg_send, sel, sel_impl};
    use std::ffi::CString;

    // Objective-C block layout (no captures) for completion handlers.
    #[repr(C)]
    struct ObjcBlock {
        isa: *const std::ffi::c_void,
        flags: i32,
        reserved: i32,
        invoke: unsafe extern "C" fn(*mut ObjcBlock, u8, *mut Object),
        descriptor: *const BlockDesc,
    }

    #[repr(C)]
    struct BlockDesc {
        reserved: usize,
        size: usize,
    }

    extern "C" {
        static _NSConcreteStackBlock: std::ffi::c_void;
    }

    unsafe extern "C" fn noop_auth(
        _block: *mut ObjcBlock,
        _granted: u8,
        _error: *mut Object,
    ) {
    }

    static BLOCK_DESC: BlockDesc = BlockDesc {
        reserved: 0,
        size: std::mem::size_of::<ObjcBlock>(),
    };

    unsafe {
        let pool_cls = Class::get("NSAutoreleasePool").unwrap();
        let pool: *mut Object = msg_send![pool_cls, new];

        let nsstring = Class::get("NSString").unwrap();

        // Strip null bytes to prevent CString::new panics
        let title_safe = title.replace('\0', "");
        let subtitle_safe = subtitle.replace('\0', "");
        let body_safe = body.replace('\0', "");

        let title_c = CString::new(title_safe).unwrap();
        let sub_c = CString::new(subtitle_safe).unwrap();
        let body_c = CString::new(body_safe).unwrap();

        let title_ns: *mut Object =
            msg_send![nsstring, stringWithUTF8String: title_c.as_ptr()];
        let sub_ns: *mut Object =
            msg_send![nsstring, stringWithUTF8String: sub_c.as_ptr()];
        let body_ns: *mut Object =
            msg_send![nsstring, stringWithUTF8String: body_c.as_ptr()];

        let mut delivered = false;

        // ── Modern API: UNUserNotificationCenter (macOS 10.14+) ──
        // Load the UserNotifications framework at runtime.
        let bundle_cls = Class::get("NSBundle").unwrap();
        let fw_path_c = CString::new(
            "/System/Library/Frameworks/UserNotifications.framework",
        )
        .unwrap();
        let fw_path_ns: *mut Object =
            msg_send![nsstring, stringWithUTF8String: fw_path_c.as_ptr()];
        let fw_bundle: *mut Object = msg_send![bundle_cls, bundleWithPath: fw_path_ns];

        if !fw_bundle.is_null() {
            let loaded: bool = msg_send![fw_bundle, load];
            if loaded {
                if let Some(center_cls) = Class::get("UNUserNotificationCenter") {
                    let center: *mut Object =
                        msg_send![center_cls, currentNotificationCenter];
                    if !center.is_null() {
                        // Request authorization (idempotent once granted).
                        let auth_block = ObjcBlock {
                            isa: &_NSConcreteStackBlock as *const _
                                as *const std::ffi::c_void,
                            flags: 0,
                            reserved: 0,
                            invoke: noop_auth,
                            descriptor: &BLOCK_DESC,
                        };
                        // UNAuthorizationOptionAlert (1<<2) | UNAuthorizationOptionSound (1<<1)
                        let options: usize = 4 | 2;
                        let _: () = msg_send![center,
                            requestAuthorizationWithOptions:options
                            completionHandler:&auth_block as *const _ as *const std::ffi::c_void];

                        // Build notification content.
                        if let Some(content_cls) =
                            Class::get("UNMutableNotificationContent")
                        {
                            let content: *mut Object = msg_send![content_cls, new];
                            let _: () = msg_send![content, setTitle: title_ns];
                            let _: () = msg_send![content, setSubtitle: sub_ns];
                            let _: () = msg_send![content, setBody: body_ns];

                            // Build and deliver the request.
                            if let Some(request_cls) =
                                Class::get("UNNotificationRequest")
                            {
                                let nsuuid_cls = Class::get("NSUUID").unwrap();
                                let uuid: *mut Object = msg_send![nsuuid_cls, UUID];
                                let uuid_str: *mut Object =
                                    msg_send![uuid, UUIDString];
                                let trigger: *const Object = std::ptr::null();
                                let request: *mut Object = msg_send![request_cls,
                                    requestWithIdentifier:uuid_str
                                    content:content
                                    trigger:trigger];

                                // completionHandler is nullable.
                                let nil: *const std::ffi::c_void = std::ptr::null();
                                let _: () = msg_send![center,
                                    addNotificationRequest:request
                                    withCompletionHandler:nil];
                                delivered = true;
                            }
                        }
                    }
                }
            }
        }

        // ── Legacy fallback: NSUserNotificationCenter (pre-removal) ──
        if !delivered {
            if let (Some(notif_cls), Some(center_cls)) = (
                Class::get("NSUserNotification"),
                Class::get("NSUserNotificationCenter"),
            ) {
                let center: *mut Object =
                    msg_send![center_cls, defaultUserNotificationCenter];
                if !center.is_null() {
                    let notif: *mut Object = msg_send![notif_cls, alloc];
                    let notif: *mut Object = msg_send![notif, init];

                    let _: () = msg_send![notif, setTitle: title_ns];
                    let _: () = msg_send![notif, setSubtitle: sub_ns];
                    let _: () = msg_send![notif, setInformativeText: body_ns];

                    let _: () = msg_send![center, deliverNotification: notif];
                    delivered = true;
                }
            }
        }

        // ── Final fallback: osascript (notification attributed to Script Editor) ──
        if !delivered {
            fn escape(s: &str) -> String {
                s.replace('\\', "\\\\").replace('"', "\\\"")
            }
            let script = format!(
                "display notification \"{}\" with title \"{}\" subtitle \"{}\"",
                escape(body),
                escape(title),
                escape(subtitle),
            );
            let _ = Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .output();
        }

        let _: () = msg_send![pool, drain];
    }
}

#[cfg(not(target_os = "macos"))]
fn send_macos_notification(_title: &str, _subtitle: &str, _body: &str) {}
