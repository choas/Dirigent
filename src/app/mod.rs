mod code_viewer;
mod cue_pool;
mod dialogs;
mod panels;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use eframe::egui;

use crate::claude;
use crate::db::{Cue, CueStatus, Database};
use crate::diff_view::{DiffViewMode, ParsedDiff};
use crate::file_tree::FileTree;
use crate::git;
use crate::settings::{self, Settings};

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

pub struct DirigentApp {
    project_root: PathBuf,
    db: Database,

    // File tree
    file_tree: Option<FileTree>,
    expanded_dirs: HashSet<PathBuf>,

    // Code viewer
    current_file: Option<PathBuf>,
    current_file_content: Vec<String>,
    /// Start of the selected line range (1-based, always <= selection_end).
    selection_start: Option<usize>,
    /// End of the selected line range (1-based, always >= selection_start).
    selection_end: Option<usize>,
    cue_input: String,
    scroll_to_line: Option<usize>,

    // Cue pool
    cues: Vec<Cue>,

    // Claude execution
    claude_pending: Arc<Mutex<Vec<ClaudeResult>>>,

    // Diff review modal
    diff_review: Option<DiffReview>,

    // Git info
    git_info: Option<git::GitInfo>,

    // Commit history
    commit_history: Vec<git::CommitInfo>,

    // Syntax highlighting theme
    syntax_theme: egui_extras::syntax_highlighting::CodeTheme,

    // Settings (Feature 1)
    settings: Settings,
    show_settings: bool,
    needs_theme_apply: bool,

    // Global prompt (Feature 2)
    global_prompt_input: String,

    // Repo picker (Feature 4)
    show_repo_picker: bool,
    repo_path_input: String,

    // Worktrees (Feature 5)
    show_worktree_panel: bool,
    worktrees: Vec<git::WorktreeInfo>,
    new_worktree_name: String,

    // Inline cue editing
    editing_cue_id: Option<i64>,
    editing_cue_text: String,

    // Git log panel (left nav)
    show_git_log: bool,

    // Claude running logs (live stderr streaming)
    running_logs: HashMap<i64, Arc<Mutex<String>>>,
    running_start_times: HashMap<i64, Instant>,
    show_running_log: Option<i64>,

    // About dialog
    show_about: bool,
    logo_texture: Option<egui::TextureHandle>,

    // File-system watcher
    _fs_watcher: Option<RecommendedWatcher>,
    fs_changed: Arc<AtomicBool>,
    last_fs_rescan: Instant,
}

fn start_fs_watcher(root: &PathBuf, changed: &Arc<AtomicBool>) -> Option<RecommendedWatcher> {
    let flag = Arc::clone(changed);
    let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        if let Ok(event) = res {
            use notify::EventKind;
            match event.kind {
                EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
                    flag.store(true, Ordering::Relaxed);
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
        let cues = db.all_cues().unwrap_or_default();
        let git_info = git::read_git_info(&project_root);
        let settings = settings::load_settings(&project_root);
        let commit_history = git::read_commit_history(&project_root, 50);
        let worktrees = git::list_worktrees(&project_root).unwrap_or_default();

        let fs_changed = Arc::new(AtomicBool::new(false));
        let _fs_watcher = start_fs_watcher(&project_root, &fs_changed);

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
            current_file: None,
            current_file_content: Vec::new(),
            selection_start: None,
            selection_end: None,
            cue_input: String::new(),
            scroll_to_line: None,
            cues,
            claude_pending: Arc::new(Mutex::new(Vec::new())),
            diff_review: None,
            git_info,
            commit_history,
            syntax_theme,
            settings,
            show_settings: false,
            needs_theme_apply: true,
            global_prompt_input: String::new(),
            show_repo_picker: false,
            repo_path_input: String::new(),
            show_worktree_panel: false,
            worktrees,
            new_worktree_name: String::new(),
            editing_cue_id: None,
            editing_cue_text: String::new(),
            running_logs: HashMap::new(),
            running_start_times: HashMap::new(),
            show_running_log: None,
            show_git_log: false,
            show_about: false,
            logo_texture: None,
            _fs_watcher,
            fs_changed,
            last_fs_rescan: Instant::now(),
        }
    }

    fn format_elapsed(&self, cue_id: i64) -> String {
        if let Some(start) = self.running_start_times.get(&cue_id) {
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
        self.file_tree = FileTree::scan(&self.project_root).ok();
    }

    fn reload_cues(&mut self) {
        self.cues = self.db.all_cues().unwrap_or_default();
    }

    fn reload_git_info(&mut self) {
        self.git_info = git::read_git_info(&self.project_root);
    }

    fn reload_commit_history(&mut self) {
        self.commit_history = git::read_commit_history(&self.project_root, 50);
    }

    fn load_file(&mut self, path: PathBuf) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            self.current_file_content = content.lines().map(String::from).collect();
            self.current_file = Some(path);
            self.selection_start = None;
            self.selection_end = None;
            self.cue_input.clear();
        }
    }

    fn relative_path(&self, path: &PathBuf) -> String {
        path.strip_prefix(&self.project_root)
            .unwrap_or(path.as_path())
            .to_string_lossy()
            .to_string()
    }

    fn file_cues(&self) -> Vec<&Cue> {
        if let Some(ref current) = self.current_file {
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
        self.db = match Database::open(&new_root) {
            Ok(db) => db,
            Err(e) => {
                eprintln!("Failed to open database at {}: {}", new_root.display(), e);
                return;
            }
        };
        self.project_root = new_root.clone();
        self.file_tree = FileTree::scan(&self.project_root).ok();
        self.fs_changed.store(false, Ordering::Relaxed);
        self._fs_watcher = start_fs_watcher(&self.project_root, &self.fs_changed);
        self.cues = self.db.all_cues().unwrap_or_default();
        self.git_info = git::read_git_info(&self.project_root);
        self.current_file = None;
        self.commit_history = git::read_commit_history(&self.project_root, 50);
        self.current_file_content.clear();
        self.selection_start = None;
        self.selection_end = None;
        self.expanded_dirs.clear();
        self.diff_review = None;
        self.worktrees = git::list_worktrees(&self.project_root).unwrap_or_default();

        // Update recent repos
        let path_str = new_root.to_string_lossy().to_string();
        settings::add_recent_repo(&mut self.settings, &path_str);
        settings::save_settings(&self.project_root, &self.settings);
    }

    // -- Feature 5: Worktrees --

    fn reload_worktrees(&mut self) {
        self.worktrees = git::list_worktrees(&self.project_root).unwrap_or_default();
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

        // Create shared log buffer for live stderr streaming
        let log = Arc::new(Mutex::new(String::new()));
        self.running_logs.insert(cue_id, Arc::clone(&log));
        self.running_start_times.insert(cue_id, Instant::now());

        let project_root = self.project_root.clone();
        let pending = Arc::clone(&self.claude_pending);
        let model = self.settings.claude_model.clone();

        std::thread::spawn(move || {
            let result =
                match claude::invoke_claude_streaming(&prompt, &project_root, &model, log) {
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
            if let Ok(mut pending) = pending.lock() {
                pending.push(result);
            }
        });
    }

    fn process_claude_results(&mut self) {
        let results: Vec<ClaudeResult> = {
            if let Ok(mut pending) = self.claude_pending.lock() {
                pending.drain(..).collect()
            } else {
                return;
            }
        };

        for result in results {
            if let Some(ref error) = result.error {
                eprintln!("Claude error for cue {}: {}", result.cue_id, error);
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
                if let Some(ref path) = self.current_file {
                    let p = path.clone();
                    self.load_file(p);
                }
                self.reload_git_info();
            } else {
                // Claude ran but no files were changed
                let _ =
                    self.db
                        .complete_execution(result.exec_id, &result.response, None);
                eprintln!(
                    "Claude completed but no file changes detected for cue {}",
                    result.cue_id
                );
                let _ = self
                    .db
                    .update_cue_status(result.cue_id, CueStatus::Done);
            }
            self.reload_cues();
        }
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
            let preview = self
                .db
                .get_cue(cue_id)
                .ok()
                .flatten()
                .map(|c| {
                    let words: Vec<&str> = c.text.split_whitespace().take(6).collect();
                    let mut preview = words.join(" ");
                    if c.text.split_whitespace().count() > 6 {
                        preview.push_str("\u{2026}");
                    }
                    preview
                })
                .unwrap_or_else(|| format!("Cue #{}", cue_id));
            let msg = format!("{} \u{2014} ready for review.", preview);
            std::thread::spawn(move || {
                let _ = Command::new("osascript")
                    .args([
                        "-e",
                        &format!(
                            "display notification \"{}\" with title \"Dirigent\" subtitle \"Task moved to Review\"",
                            msg
                        ),
                    ])
                    .output();
            });
        }
    }

    // -- Theme --

    fn apply_theme(&mut self, ctx: &egui::Context) {
        if !self.needs_theme_apply {
            return;
        }
        self.needs_theme_apply = false;
        ctx.set_visuals(self.settings.theme.visuals());
        self.syntax_theme = if self.settings.theme.is_dark() {
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
        style.text_styles.insert(egui::TextStyle::Small, egui::FontId::new(size * 0.75, egui::FontFamily::Proportional));
        style.text_styles.insert(egui::TextStyle::Body, egui::FontId::new(size, egui::FontFamily::Proportional));
        style.text_styles.insert(egui::TextStyle::Monospace, egui::FontId::new(size, egui::FontFamily::Monospace));
        style.text_styles.insert(egui::TextStyle::Button, egui::FontId::new(size, egui::FontFamily::Proportional));
        style.text_styles.insert(egui::TextStyle::Heading, egui::FontId::new(size * 1.4, egui::FontFamily::Proportional));
        ctx.set_style(style);
    }
}

impl eframe::App for DirigentApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply theme if needed
        self.apply_theme(ctx);

        // Check for filesystem changes and rescan file tree (debounced)
        if self.fs_changed.load(Ordering::Relaxed)
            && self.last_fs_rescan.elapsed() >= std::time::Duration::from_secs(2)
        {
            self.fs_changed.store(false, Ordering::Relaxed);
            self.last_fs_rescan = Instant::now();
            self.reload_file_tree();
        }

        // Poll for Claude results
        self.process_claude_results();

        // Request repaint if there are pending Claude tasks
        if let Ok(pending) = self.claude_pending.lock() {
            if !pending.is_empty() {
                ctx.request_repaint();
            }
        }
        if self
            .cues
            .iter()
            .any(|c| c.status == CueStatus::Ready)
        {
            // Repaint faster when log window is open for live streaming
            let interval = if self.show_running_log.is_some() {
                100
            } else {
                500
            };
            ctx.request_repaint_after(std::time::Duration::from_millis(interval));
        } else if self.fs_changed.load(Ordering::Relaxed) {
            // Ensure we repaint to pick up filesystem changes after debounce
            ctx.request_repaint_after(std::time::Duration::from_secs(2));
        }

        // Render all panels (order matters for layout)
        self.render_menu_bar(ctx); // macOS-style menu bar
        self.render_repo_bar(ctx); // top
        self.render_status_bar(ctx); // bottom-most
        self.render_prompt_field(ctx); // above status bar
        self.render_file_tree_panel(ctx); // left side
        self.render_cue_pool(ctx); // right side
        self.render_code_viewer(ctx); // center (code / diff review / claude progress / settings)
        self.render_repo_picker(ctx); // floating
        self.render_worktree_panel(ctx); // floating
        self.render_about_dialog(ctx); // floating
    }
}
