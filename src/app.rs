use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::claude;
use crate::db::{Cue, CueStatus, Database};
use crate::diff_view::{self, DiffViewMode, ParsedDiff};
use crate::file_tree::{FileEntry, FileTree};
use crate::git;
use crate::settings::{self, Settings, ThemeChoice};

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
    show_running_log: Option<i64>,

    // About dialog
    show_about: bool,
    logo_texture: Option<egui::TextureHandle>,
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
            show_running_log: None,
            show_git_log: false,
            show_about: false,
            logo_texture: None,
        }
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

    fn lines_with_cues(&self) -> HashSet<usize> {
        let mut set = HashSet::new();
        for c in self.file_cues() {
            let start = c.line_number;
            let end = c.line_number_end.unwrap_or(start);
            for line in start..=end {
                set.insert(line);
            }
        }
        set
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
                        preview.push_str("…");
                    }
                    preview
                })
                .unwrap_or_else(|| format!("Cue #{}", cue_id));
            let msg = format!("{} — ready for review.", preview);
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

    // -- UI rendering --

    fn render_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("Dirigent", |ui| {
                    if ui.button("About Dirigent").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Settings...").clicked() {
                        self.show_settings = true;
                        ui.close_menu();
                    }
                });
            });
        });
    }

    fn render_about_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }

        // Lazily load the logo texture
        if self.logo_texture.is_none() {
            let png_bytes = include_bytes!("../assets/logo.png");
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

        let mut open = self.show_about;
        egui::Window::new("About Dirigent")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    if let Some(ref tex) = self.logo_texture {
                        ui.add(egui::Image::new(tex).max_size(egui::vec2(128.0, 128.0)));
                    }
                    ui.add_space(8.0);
                    ui.heading("Dirigent");
                    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("A read-only code viewer where humans direct and AI performs.")
                            .weak(),
                    );
                    ui.add_space(12.0);
                });
            });
        self.show_about = open;
    }

    // Feature 4: Repo bar at top
    fn render_repo_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("repo_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("\u{25B8} {}", self.project_root.display()))
                        .monospace()
                        .small(),
                );
                if ui.small_button("Change...").clicked() {
                    self.repo_path_input = self.project_root.to_string_lossy().to_string();
                    self.show_repo_picker = true;
                }
                if ui.small_button("Worktrees").clicked() {
                    self.reload_worktrees();
                    self.show_worktree_panel = true;
                }
            });
        });
    }

    fn render_file_tree_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("file_tree")
            .default_width(220.0)
            .min_width(150.0)
            .show(ctx, |ui| {
                ui.heading("Files");
                ui.separator();
                // File tree takes remaining space above git log
                let git_log_open = self.show_git_log;
                let available = ui.available_height();
                // When git log is open, give file tree ~60% of space; otherwise all of it
                let file_tree_height = if git_log_open {
                    available * 0.6
                } else {
                    available - 24.0 // leave room for the git log header
                };
                egui::ScrollArea::vertical()
                    .id_salt("file_tree_scroll")
                    .max_height(file_tree_height)
                    .show(ui, |ui| {
                        if let Some(tree) = self.file_tree.clone() {
                            let mut file_to_load = None;
                            for entry in &tree.entries {
                                Self::render_file_entry(
                                    ui,
                                    entry,
                                    &mut self.expanded_dirs,
                                    &self.current_file,
                                    &mut file_to_load,
                                );
                            }
                            if let Some(path) = file_to_load {
                                self.load_file(path);
                            }
                        }
                    });

                ui.separator();

                // Git Log collapsible section
                let header_text = format!("Git Log ({})", self.commit_history.len());
                let header_resp = egui::CollapsingHeader::new(header_text)
                    .default_open(self.show_git_log)
                    .show(ui, |ui| {
                        let mut clicked_commit: Option<(String, String)> = None;
                        egui::ScrollArea::vertical()
                            .id_salt("git_log_scroll")
                            .show(ui, |ui| {
                                for commit in &self.commit_history {
                                    let msg = if commit.message.len() > 30 {
                                        format!("{}...", &commit.message[..27])
                                    } else {
                                        commit.message.clone()
                                    };
                                    let label =
                                        format!("{} {}", commit.short_hash, msg);
                                    if ui
                                        .selectable_label(
                                            false,
                                            egui::RichText::new(&label)
                                                .monospace()
                                                .small(),
                                        )
                                        .on_hover_text(format!(
                                            "{} - {}\n{}\n{}",
                                            commit.short_hash,
                                            commit.author,
                                            commit.message,
                                            commit.time_ago
                                        ))
                                        .clicked()
                                    {
                                        clicked_commit =
                                            Some((commit.full_hash.clone(), commit.message.clone()));
                                    }
                                }
                            });
                        clicked_commit
                    });
                self.show_git_log = header_resp.fully_open();
                if let Some(inner) = header_resp.body_returned {
                    if let Some((full_hash, message)) = inner {
                        let short_hash = &full_hash[..7.min(full_hash.len())];
                        let diff_text = git::get_commit_diff(&self.project_root, &full_hash)
                            .unwrap_or_default();
                        let parsed = diff_view::parse_unified_diff(&diff_text);
                        self.diff_review = Some(DiffReview {
                            cue_id: 0,
                            diff: diff_text,
                            cue_text: format!("{} {}", short_hash, message),
                            parsed,
                            view_mode: DiffViewMode::Inline,
                            read_only: true,
                            collapsed_files: HashSet::new(),
                        });
                    }
                }
            });
    }

    fn render_file_entry(
        ui: &mut egui::Ui,
        entry: &FileEntry,
        expanded: &mut HashSet<PathBuf>,
        current_file: &Option<PathBuf>,
        file_to_load: &mut Option<PathBuf>,
    ) {
        if entry.is_dir {
            let is_expanded = expanded.contains(&entry.path);
            let header = egui::CollapsingHeader::new(&entry.name)
                .default_open(is_expanded)
                .show(ui, |ui| {
                    for child in &entry.children {
                        Self::render_file_entry(ui, child, expanded, current_file, file_to_load);
                    }
                });
            if header.fully_open() {
                expanded.insert(entry.path.clone());
            } else {
                expanded.remove(&entry.path);
            }
        } else {
            let is_selected = current_file.as_ref() == Some(&entry.path);
            if ui
                .selectable_label(is_selected, &entry.name)
                .clicked()
            {
                *file_to_load = Some(entry.path.clone());
            }
        }
    }

    fn render_settings_panel(&mut self, ctx: &egui::Context) {
        let mut save = false;
        let mut close = false;

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Settings");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("✕").on_hover_text("Close settings").clicked() {
                        close = true;
                    }
                });
            });
            ui.separator();
            ui.add_space(8.0);

            egui::Grid::new("settings_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Theme:");
                    let theme_label = self.settings.theme.display_name();
                    egui::ComboBox::from_id_salt("theme_combo")
                        .selected_text(theme_label)
                        .show_ui(ui, |ui| {
                            let mut prev_was_dark = true;
                            for variant in ThemeChoice::all_variants() {
                                if prev_was_dark && !variant.is_dark() {
                                    ui.separator();
                                    prev_was_dark = false;
                                }
                                ui.selectable_value(
                                    &mut self.settings.theme,
                                    variant.clone(),
                                    variant.display_name(),
                                );
                            }
                        });
                    ui.end_row();

                    ui.label("Claude Model:");
                    egui::ComboBox::from_id_salt("model_combo")
                        .selected_text(&self.settings.claude_model)
                        .show_ui(ui, |ui| {
                            for model in &[
                                "claude-opus-4-6",
                                "claude-sonnet-4-6",
                            ] {
                                ui.selectable_value(
                                    &mut self.settings.claude_model,
                                    model.to_string(),
                                    *model,
                                );
                            }
                        });
                    ui.end_row();

                    ui.label("Font:");
                    egui::ComboBox::from_id_salt("font_combo")
                        .selected_text(&self.settings.font_family)
                        .show_ui(ui, |ui| {
                            for font in &[
                                "Menlo",
                                "SF Mono",
                                "Monaco",
                                "Courier New",
                                "JetBrains Mono",
                                "Fira Code",
                                "Source Code Pro",
                                "Cascadia Code",
                            ] {
                                ui.selectable_value(
                                    &mut self.settings.font_family,
                                    font.to_string(),
                                    *font,
                                );
                            }
                        });
                    ui.end_row();

                    ui.label("Font Size:");
                    ui.add(egui::Slider::new(&mut self.settings.font_size, 8.0..=32.0).suffix(" px"));
                    ui.end_row();

                    ui.label("Notifications:");
                    ui.end_row();

                    ui.label("  Sound:");
                    ui.checkbox(&mut self.settings.notify_sound, "Play sound on task review");
                    ui.end_row();

                    ui.label("  Popup:");
                    ui.checkbox(&mut self.settings.notify_popup, "Show macOS notification");
                    ui.end_row();
                });

            ui.add_space(12.0);
            if ui.button("Save").clicked() {
                save = true;
            }
        });

        if close {
            self.show_settings = false;
        }
        if save {
            settings::save_settings(&self.project_root, &self.settings);
            self.needs_theme_apply = true;
        }
    }

    fn render_code_viewer(&mut self, ctx: &egui::Context) {
        if self.show_settings {
            self.render_settings_panel(ctx);
            return;
        }

        // Diff Review in central panel
        if self.diff_review.is_some() {
            self.render_diff_review_central(ctx);
            return;
        }

        // Claude Progress in central panel
        if self.show_running_log.is_some() {
            self.render_running_log_central(ctx);
            return;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.current_file.is_none() {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a file from the tree to view");
                });
                return;
            }

            let file_path = self.current_file.clone().unwrap();
            let rel_path = self.relative_path(&file_path);

            ui.horizontal(|ui| {
                ui.strong(&rel_path);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{} lines", self.current_file_content.len()));
                });
            });
            ui.separator();

            let lines_with_cues = self.lines_with_cues();
            let num_lines = self.current_file_content.len();
            let line_height = 16.0;

            let ext = file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            let sel_start = self.selection_start;
            let sel_end = self.selection_end;
            let mut new_sel_start = sel_start;
            let mut new_sel_end = sel_end;
            let mut submit_cue = false;
            let mut clear_selection = false;

            let scroll_area = egui::ScrollArea::vertical().auto_shrink([false; 2]);

            scroll_area.show_rows(ui, line_height, num_lines, |ui, row_range| {
                for line_idx in row_range {
                    let line_num = line_idx + 1;
                    let line_text = self
                        .current_file_content
                        .get(line_idx)
                        .map(|s| s.as_str())
                        .unwrap_or("");

                    let is_in_selection = match (sel_start, sel_end) {
                        (Some(s), Some(e)) => line_num >= s && line_num <= e,
                        _ => false,
                    };
                    let is_selection_end = sel_end == Some(line_num);
                    let has_cue = lines_with_cues.contains(&line_num);

                    let response = ui.horizontal(|ui| {
                        if has_cue {
                            ui.colored_label(egui::Color32::from_rgb(255, 180, 50), "\u{2022}");
                        } else {
                            ui.label(" ");
                        }

                        let num_text = format!("{:>4} ", line_num);
                        ui.label(
                            egui::RichText::new(num_text)
                                .monospace()
                                .color(egui::Color32::from_gray(100)),
                        );

                        let layout_job = egui_extras::syntax_highlighting::highlight(
                            ui.ctx(),
                            ui.style(),
                            &self.syntax_theme,
                            line_text,
                            ext,
                        );
                        let response = ui.label(layout_job);

                        let rect = response.rect.union(ui.available_rect_before_wrap());
                        let response = ui.interact(
                            rect,
                            egui::Id::new(("code_line", line_idx)),
                            egui::Sense::click(),
                        );

                        if is_in_selection {
                            ui.painter().rect_filled(
                                rect,
                                0.0,
                                egui::Color32::from_rgba_premultiplied(60, 60, 120, 80),
                            );
                        }

                        response
                    });

                    if response.inner.clicked() {
                        let shift_held = ui.input(|i| i.modifiers.shift);
                        if shift_held {
                            // Shift-click: extend selection from existing start (or set new range)
                            if let Some(anchor) = sel_start {
                                let lo = anchor.min(line_num);
                                let hi = anchor.max(line_num);
                                new_sel_start = Some(lo);
                                new_sel_end = Some(hi);
                            } else {
                                new_sel_start = Some(line_num);
                                new_sel_end = Some(line_num);
                            }
                        } else {
                            // Plain click: select single line
                            new_sel_start = Some(line_num);
                            new_sel_end = Some(line_num);
                        }
                    }

                    // Show cue input after the last line of the selection
                    if is_selection_end {
                        let range_label = if sel_start == sel_end {
                            format!("L{}", sel_start.unwrap_or(0))
                        } else {
                            format!(
                                "L{}-{}",
                                sel_start.unwrap_or(0),
                                sel_end.unwrap_or(0)
                            )
                        };
                        ui.horizontal(|ui| {
                            ui.label("     ");
                            ui.label(
                                egui::RichText::new(range_label)
                                    .monospace()
                                    .color(egui::Color32::from_rgb(100, 200, 100)),
                            );
                            let input_response = ui.add(
                                egui::TextEdit::singleline(&mut self.cue_input)
                                    .desired_width(ui.available_width() - 80.0)
                                    .hint_text("Add a cue...")
                                    .font(egui::TextStyle::Monospace),
                            );
                            if ui.button("Add").clicked()
                                || (input_response.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                            {
                                submit_cue = true;
                            }
                            if ui.button("✕").clicked()
                                || ui.input(|i| i.key_pressed(egui::Key::Escape))
                            {
                                clear_selection = true;
                            }
                        });
                    }
                }
            });

            if clear_selection {
                new_sel_start = None;
                new_sel_end = None;
            }

            if new_sel_start != self.selection_start || new_sel_end != self.selection_end {
                self.selection_start = new_sel_start;
                self.selection_end = new_sel_end;
                self.cue_input.clear();
            }

            if submit_cue && !self.cue_input.is_empty() {
                if let Some(start) = self.selection_start {
                    let end = self.selection_end.unwrap_or(start);
                    let line_end = if end > start { Some(end) } else { None };
                    let text = self.cue_input.clone();
                    let _ = self.db.insert_cue(&text, &rel_path, start, line_end);
                    self.cue_input.clear();
                    self.reload_cues();
                }
            }
        });
    }

    fn render_cue_pool(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("cue_pool")
            .default_width(250.0)
            .min_width(200.0)
            .show(ctx, |ui| {
                ui.heading("Cues");
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut actions: Vec<(i64, CueAction)> = Vec::new();

                    let cues_snapshot = self.cues.clone();
                    for &status in CueStatus::all() {
                        let section_cues: Vec<&Cue> = cues_snapshot
                            .iter()
                            .filter(|c| c.status == status)
                            .collect();

                        let header = format!("{} ({})", status.label(), section_cues.len());
                        egui::CollapsingHeader::new(header)
                            .default_open(
                                status == CueStatus::Inbox || status == CueStatus::Review,
                            )
                            .show(ui, |ui| {
                                if section_cues.is_empty() {
                                    ui.label(
                                        egui::RichText::new("(empty)")
                                            .italics()
                                            .color(egui::Color32::from_gray(120)),
                                    );
                                }
                                for cue in &section_cues {
                                    self.render_cue_card(ui, cue, &mut actions, status);
                                }
                            });
                    }

                    // Process actions after iteration
                    for (id, action) in actions {
                        match action {
                            CueAction::StartEdit(text) => {
                                self.editing_cue_id = Some(id);
                                self.editing_cue_text = text;
                            }
                            CueAction::CancelEdit => {
                                self.editing_cue_id = None;
                            }
                            CueAction::SaveEdit(new_text) => {
                                let _ = self.db.update_cue_text(id, &new_text);
                                self.editing_cue_id = None;
                            }
                            CueAction::MoveTo(new_status) => {
                                let _ = self.db.update_cue_status(id, new_status);
                                if new_status == CueStatus::Ready {
                                    self.reload_cues();
                                    self.trigger_claude(id);
                                }
                            }
                            CueAction::Delete => {
                                let _ = self.db.delete_cue(id);
                            }
                            CueAction::Navigate(file_path, line, line_end) => {
                                let full_path = self.project_root.join(&file_path);
                                if self.current_file.as_ref() != Some(&full_path) {
                                    self.load_file(full_path);
                                }
                                self.selection_start = Some(line);
                                self.selection_end = Some(line_end.unwrap_or(line));
                                self.scroll_to_line = Some(line);
                            }
                            CueAction::ShowDiff(cue_id) => {
                                if let Ok(Some(exec)) = self.db.get_latest_execution(cue_id) {
                                    if let Some(diff) = exec.diff {
                                        let cue = self
                                            .cues
                                            .iter()
                                            .find(|c| c.id == cue_id);
                                        let text = cue
                                            .map(|c| c.text.clone())
                                            .unwrap_or_default();
                                        let read_only = cue
                                            .map(|c| c.status != CueStatus::Review)
                                            .unwrap_or(true);
                                        let parsed = diff_view::parse_unified_diff(&diff);
                                        self.diff_review = Some(DiffReview {
                                            cue_id,
                                            diff,
                                            cue_text: text,
                                            parsed,
                                            view_mode: DiffViewMode::Inline,
                                            read_only,
                                            collapsed_files: HashSet::new(),
                                        });
                                    }
                                }
                            }
                            CueAction::CommitReview(cue_id) => {
                                if let Ok(Some(exec)) =
                                    self.db.get_latest_execution(cue_id)
                                {
                                    if let Some(ref diff) = exec.diff {
                                        let file_paths = git::parse_diff_file_paths_for_repo(&self.project_root,diff);
                                        let cue_text = self
                                            .cues
                                            .iter()
                                            .find(|c| c.id == cue_id)
                                            .map(|c| c.text.clone())
                                            .unwrap_or_default();
                                        let commit_msg =
                                            git::generate_commit_message(&cue_text);
                                        match git::stage_and_commit(
                                            &self.project_root,
                                            &file_paths,
                                            &commit_msg,
                                        ) {
                                            Ok(hash) => {
                                                eprintln!("Committed: {}", hash);
                                                let _ = self.db.update_cue_status(
                                                    cue_id,
                                                    CueStatus::Done,
                                                );
                                            }
                                            Err(e) => {
                                                eprintln!("Commit failed: {}", e);
                                            }
                                        }
                                    }
                                }
                                self.reload_git_info();
                                self.reload_commit_history();
                            }
                            CueAction::RevertReview(cue_id) => {
                                if let Ok(Some(exec)) =
                                    self.db.get_latest_execution(cue_id)
                                {
                                    if let Some(ref diff) = exec.diff {
                                        let file_paths = git::parse_diff_file_paths_for_repo(&self.project_root,diff);
                                        if let Err(e) = git::revert_files(
                                            &self.project_root,
                                            &file_paths,
                                        ) {
                                            eprintln!("Revert failed: {}", e);
                                        }
                                    }
                                }
                                let _ = self.db.update_cue_status(
                                    cue_id,
                                    CueStatus::Inbox,
                                );
                                // Reload file to show reverted content
                                if let Some(ref path) = self.current_file {
                                    let p = path.clone();
                                    self.load_file(p);
                                }
                                self.reload_git_info();
                            }
                            CueAction::ShowRunningLog(cue_id) => {
                                self.show_running_log = Some(cue_id);
                            }
                        }
                        self.reload_cues();
                    }
                });
            });
    }

    fn render_cue_card(
        &mut self,
        ui: &mut egui::Ui,
        cue: &Cue,
        actions: &mut Vec<(i64, CueAction)>,
        status: CueStatus,
    ) {
        egui::Frame::none()
            .inner_margin(4.0)
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(60)))
            .rounding(4.0)
            .show(ui, |ui| {
                // Cue text - inline editable for Inbox
                if self.editing_cue_id == Some(cue.id) {
                    let response = ui.text_edit_multiline(&mut self.editing_cue_text);
                    ui.horizontal(|ui| {
                        if ui.small_button("\u{2713} Save").clicked() {
                            actions.push((
                                cue.id,
                                CueAction::SaveEdit(self.editing_cue_text.clone()),
                            ));
                        }
                        if ui.small_button("\u{2715} Cancel").clicked() {
                            actions.push((cue.id, CueAction::CancelEdit));
                        }
                    });
                    // Request focus on first frame
                    if response.gained_focus() || !response.has_focus() {
                        response.request_focus();
                    }
                } else {
                    let display_text = if cue.text.len() > 60 {
                        format!("{}...", &cue.text[..57])
                    } else {
                        cue.text.clone()
                    };
                    let label_response = ui.label(&display_text);
                    // Double-click label to edit (Inbox only)
                    if status == CueStatus::Inbox && label_response.double_clicked() {
                        actions.push((
                            cue.id,
                            CueAction::StartEdit(cue.text.clone()),
                        ));
                    }
                    // Single-click to show diff (Review/Done/Archived)
                    if matches!(status, CueStatus::Review | CueStatus::Done | CueStatus::Archived)
                        && label_response.clicked()
                    {
                        actions.push((cue.id, CueAction::ShowDiff(cue.id)));
                    }
                }

                // File:line link or "Global" label
                if cue.file_path.is_empty() {
                    ui.label(
                        egui::RichText::new("Global")
                            .small()
                            .color(egui::Color32::from_rgb(180, 140, 255)),
                    );
                } else {
                    let location = if let Some(end) = cue.line_number_end {
                        format!("{}:{}-{}", cue.file_path, cue.line_number, end)
                    } else {
                        format!("{}:{}", cue.file_path, cue.line_number)
                    };
                    if ui
                        .small_button(&location)
                        .on_hover_text("Navigate to this location")
                        .clicked()
                    {
                        actions.push((
                            cue.id,
                            CueAction::Navigate(
                                cue.file_path.clone(),
                                cue.line_number,
                                cue.line_number_end,
                            ),
                        ));
                    }
                }

                // Action buttons
                ui.horizontal(|ui| {
                    match cue.status {
                        CueStatus::Inbox => {
                            if self.editing_cue_id != Some(cue.id) {
                                if ui
                                    .small_button("Edit")
                                    .on_hover_text("Edit cue")
                                    .clicked()
                                {
                                    actions.push((
                                        cue.id,
                                        CueAction::StartEdit(cue.text.clone()),
                                    ));
                                }
                            }
                            if ui
                                .small_button("\u{25B6} Run")
                                .on_hover_text("Send to Claude")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::MoveTo(CueStatus::Ready),
                                ));
                            }
                            if ui
                                .small_button("\u{2713} Done")
                                .on_hover_text("Mark done (no Claude)")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::MoveTo(CueStatus::Done),
                                ));
                            }
                        }
                        CueStatus::Ready => {
                            if ui
                                .small_button(
                                    egui::RichText::new("\u{2022} Running...")
                                        .color(egui::Color32::from_rgb(100, 180, 255)),
                                )
                                .on_hover_text("View Claude's progress")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::ShowRunningLog(cue.id),
                                ));
                            }
                            if ui
                                .small_button("\u{2715} Cancel")
                                .on_hover_text("Cancel and move back to Inbox")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::MoveTo(CueStatus::Inbox),
                                ));
                            }
                        }
                        CueStatus::Review => {
                            if ui
                                .small_button("\u{25B6} Diff")
                                .on_hover_text("View the diff")
                                .clicked()
                            {
                                actions
                                    .push((cue.id, CueAction::ShowDiff(cue.id)));
                            }
                            if ui
                                .small_button("\u{2713} Commit")
                                .on_hover_text("Commit the applied changes")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::CommitReview(cue.id),
                                ));
                            }
                            if ui
                                .small_button("\u{21BA} Revert")
                                .on_hover_text("Revert changes and move back to Inbox")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::RevertReview(cue.id),
                                ));
                            }
                        }
                        CueStatus::Done => {
                            ui.label(
                                egui::RichText::new("\u{2713}")
                                    .color(egui::Color32::from_rgb(100, 200, 100)),
                            );
                            if ui
                                .small_button("\u{2193} Archive")
                                .on_hover_text("Move to Archived")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::MoveTo(CueStatus::Archived),
                                ));
                            }
                            if ui
                                .small_button("\u{21BA} Reopen")
                                .on_hover_text("Move back to Inbox")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::MoveTo(CueStatus::Inbox),
                                ));
                            }
                        }
                        CueStatus::Archived => {
                            if ui
                                .small_button("\u{21BA} Unarchive")
                                .on_hover_text("Move back to Done")
                                .clicked()
                            {
                                actions.push((
                                    cue.id,
                                    CueAction::MoveTo(CueStatus::Done),
                                ));
                            }
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("\u{2715}")
                            .on_hover_text("Delete cue")
                            .clicked()
                        {
                            actions.push((cue.id, CueAction::Delete));
                        }
                    });
                });
            });

        ui.add_space(2.0);
    }

    // Diff review rendered in the central panel (replaces code viewer)
    fn render_diff_review_central(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut accept = false;
        let mut reject = false;
        let mut toggle_mode = None;

        let review = self.diff_review.as_mut().unwrap();
        let cue_id = review.cue_id;
        let diff_text = review.diff.clone();
        let cue_text = review.cue_text.clone();
        let parsed = review.parsed.clone();
        let view_mode = review.view_mode;
        let read_only = review.read_only;
        let collapsed_files = &mut review.collapsed_files;

        egui::CentralPanel::default().show(ctx, |ui| {
            // Header bar
            ui.horizontal(|ui| {
                if ui.button("\u{2190} Back").clicked() {
                    close = true;
                }
                ui.separator();
                ui.strong("Diff Review");
                ui.separator();
                if read_only {
                    ui.label(
                        egui::RichText::new(format!("Commit: {}", cue_text))
                            .color(egui::Color32::from_gray(180)),
                    );
                } else {
                    ui.label(
                        egui::RichText::new(format!("Cue: {}", cue_text))
                            .color(egui::Color32::from_gray(180)),
                    );
                }
            });
            ui.separator();

            // View mode toggle + action buttons
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(view_mode == DiffViewMode::Inline, "Inline")
                    .clicked()
                {
                    toggle_mode = Some(DiffViewMode::Inline);
                }
                if ui
                    .selectable_label(view_mode == DiffViewMode::SideBySide, "Side-by-Side")
                    .clicked()
                {
                    toggle_mode = Some(DiffViewMode::SideBySide);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !read_only {
                        if ui
                            .button(
                                egui::RichText::new("\u{21BA} Revert")
                                    .color(egui::Color32::from_rgb(220, 100, 100)),
                            )
                            .on_hover_text("Revert changes back to previous state")
                            .clicked()
                        {
                            reject = true;
                        }
                        if ui
                            .button(
                                egui::RichText::new("\u{2713} Commit")
                                    .color(egui::Color32::from_rgb(100, 200, 100)),
                            )
                            .on_hover_text("Commit the applied changes")
                            .clicked()
                        {
                            accept = true;
                        }
                    }
                });
            });
            ui.separator();

            // Diff content fills the rest
            egui::ScrollArea::both()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    if parsed.files.is_empty() {
                        ui.add_space(20.0);
                        ui.label(
                            egui::RichText::new("No file changes in this commit.")
                                .italics()
                                .color(egui::Color32::from_rgb(150, 150, 150)),
                        );
                    } else {
                        match view_mode {
                            DiffViewMode::Inline => {
                                diff_view::render_inline_diff(ui, &parsed, collapsed_files);
                            }
                            DiffViewMode::SideBySide => {
                                diff_view::render_side_by_side_diff(ui, &parsed, collapsed_files);
                            }
                        }
                    }
                });
        });

        if let Some(mode) = toggle_mode {
            if let Some(ref mut review) = self.diff_review {
                review.view_mode = mode;
            }
        }

        if accept {
            let file_paths = git::parse_diff_file_paths_for_repo(&self.project_root, &diff_text);
            let commit_msg = git::generate_commit_message(&cue_text);
            match git::stage_and_commit(&self.project_root, &file_paths, &commit_msg) {
                Ok(hash) => {
                    eprintln!("Committed: {}", hash);
                    let _ = self
                        .db
                        .update_cue_status(cue_id, CueStatus::Done);
                }
                Err(e) => {
                    eprintln!("Commit failed: {}", e);
                }
            }
            self.reload_cues();
            self.reload_git_info();
            self.reload_commit_history();
            self.diff_review = None;
        } else if reject {
            let file_paths = git::parse_diff_file_paths_for_repo(&self.project_root, &diff_text);
            if let Err(e) = git::revert_files(&self.project_root, &file_paths) {
                eprintln!("Revert failed: {}", e);
            }
            let _ = self
                .db
                .update_cue_status(cue_id, CueStatus::Inbox);
            if let Some(ref path) = self.current_file {
                let p = path.clone();
                self.load_file(p);
            }
            self.reload_cues();
            self.reload_git_info();
            self.diff_review = None;
        } else if close {
            self.diff_review = None;
        }
    }

    fn render_status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(ref info) = self.git_info {
                    ui.label(
                        egui::RichText::new(format!("\u{25CF} {}", info.branch))
                            .monospace()
                            .small(),
                    );
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {}",
                            info.last_commit_hash, info.last_commit_message
                        ))
                        .monospace()
                        .small()
                        .color(egui::Color32::from_gray(140)),
                    );
                    ui.separator();
                    ui.label(
                        egui::RichText::new(git::format_status_summary(info))
                            .monospace()
                            .small(),
                    );
                } else {
                    ui.label(
                        egui::RichText::new("not a git repository")
                            .monospace()
                            .small()
                            .color(egui::Color32::from_gray(100)),
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Settings gear button
                    if ui
                        .small_button("\u{2699}")
                        .on_hover_text("Settings")
                        .clicked()
                    {
                        self.show_settings = !self.show_settings;
                    }

                    ui.separator();

                    let total = self.cues.len();
                    let inbox = self
                        .cues
                        .iter()
                        .filter(|c| c.status == CueStatus::Inbox)
                        .count();
                    let review = self
                        .cues
                        .iter()
                        .filter(|c| c.status == CueStatus::Review)
                        .count();
                    ui.label(
                        egui::RichText::new(format!(
                            "{} cues ({} inbox, {} review)",
                            total, inbox, review
                        ))
                        .monospace()
                        .small(),
                    );
                });

                ui.add_space(8.0);
                ui.separator();
                egui::CollapsingHeader::new(format!(
                    "Commits ({})",
                    self.commit_history.len()
                ))
                .default_open(false)
                .show(ui, |ui| {
                    let mut clicked_hash: Option<String> = None;
                    for commit in &self.commit_history {
                        let msg = if commit.message.len() > 30 {
                            format!("{}...", &commit.message[..27])
                        } else {
                            commit.message.clone()
                        };
                        let label = format!("{} {}", commit.short_hash, msg);
                        if ui
                            .selectable_label(
                                false,
                                egui::RichText::new(&label).monospace().small(),
                            )
                            .on_hover_text(format!(
                                "{}\n{}\n{}",
                                commit.short_hash, commit.message, commit.time_ago
                            ))
                            .clicked()
                        {
                            clicked_hash = Some(commit.short_hash.clone());
                        }
                    }
                    if let Some(hash) = clicked_hash {
                        if let Some(diff_text) = git::get_commit_diff(&self.project_root, &hash) {
                            let parsed = diff_view::parse_unified_diff(&diff_text);
                            self.diff_review = Some(DiffReview {
                                cue_id: 0,
                                diff: diff_text,
                                cue_text: format!("Commit {}", hash),
                                parsed,
                                view_mode: DiffViewMode::Inline,
                                read_only: true,
                                collapsed_files: HashSet::new(),
                            });
                        }
                    }
                });
            });
        });
    }

    // Feature 2: Global prompt input
    fn render_prompt_field(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("prompt_field").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("\u{276F}")
                        .monospace()
                        .color(egui::Color32::from_rgb(100, 180, 255)),
                );
                let input_response = ui.add(
                    egui::TextEdit::singleline(&mut self.global_prompt_input)
                        .desired_width(ui.available_width() - 60.0)
                        .hint_text("Global prompt (no file context)...")
                        .font(egui::TextStyle::Monospace),
                );
                let submitted = ui.button("Send").clicked()
                    || (input_response.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if submitted && !self.global_prompt_input.is_empty() {
                    let text = self.global_prompt_input.clone();
                    let _ = self.db.insert_cue(&text, "", 0, None);
                    self.global_prompt_input.clear();
                    self.reload_cues();
                }
            });
        });
    }

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
        ctx.set_fonts(font_def);

        // Scale all text styles based on the chosen font size
        style.text_styles.insert(egui::TextStyle::Small, egui::FontId::new(size * 0.75, egui::FontFamily::Proportional));
        style.text_styles.insert(egui::TextStyle::Body, egui::FontId::new(size, egui::FontFamily::Proportional));
        style.text_styles.insert(egui::TextStyle::Monospace, egui::FontId::new(size, egui::FontFamily::Monospace));
        style.text_styles.insert(egui::TextStyle::Button, egui::FontId::new(size, egui::FontFamily::Proportional));
        style.text_styles.insert(egui::TextStyle::Heading, egui::FontId::new(size * 1.4, egui::FontFamily::Proportional));
        ctx.set_style(style);
    }

    // Feature 4: Repo picker window
    fn render_repo_picker(&mut self, ctx: &egui::Context) {
        if !self.show_repo_picker {
            return;
        }

        let mut open = self.show_repo_picker;
        let mut switch_to: Option<PathBuf> = None;

        egui::Window::new("Open Repository")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_size([450.0, 300.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Path:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.repo_path_input)
                            .desired_width(300.0)
                            .font(egui::TextStyle::Monospace),
                    );
                    if ui.button("Open").clicked() {
                        let path = PathBuf::from(&self.repo_path_input);
                        if let Ok(canonical) = std::fs::canonicalize(&path) {
                            if git2::Repository::discover(&canonical).is_ok() {
                                switch_to = Some(canonical);
                            } else {
                                eprintln!("Not a git repository: {}", path.display());
                            }
                        } else {
                            eprintln!("Path not found: {}", path.display());
                        }
                    }
                });

                ui.separator();
                ui.label("Recent repositories:");
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for repo_path in self.settings.recent_repos.clone() {
                            if ui.button(&repo_path).clicked() {
                                let path = PathBuf::from(&repo_path);
                                if let Ok(canonical) = std::fs::canonicalize(&path) {
                                    switch_to = Some(canonical);
                                }
                            }
                        }
                        if self.settings.recent_repos.is_empty() {
                            ui.label(
                                egui::RichText::new("(none)")
                                    .italics()
                                    .color(egui::Color32::from_gray(120)),
                            );
                        }
                    });
            });

        self.show_repo_picker = open;

        if let Some(new_root) = switch_to {
            self.show_repo_picker = false;
            self.switch_repo(new_root);
        }
    }

    // Feature 5: Worktree panel
    fn render_worktree_panel(&mut self, ctx: &egui::Context) {
        if !self.show_worktree_panel {
            return;
        }

        let mut open = self.show_worktree_panel;
        let mut switch_to: Option<PathBuf> = None;
        let mut remove_path: Option<PathBuf> = None;
        let mut create_name: Option<String> = None;

        egui::Window::new("Git Worktrees")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([400.0, 300.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for wt in &self.worktrees {
                            ui.horizontal(|ui| {
                                let label = if wt.is_current {
                                    format!("\u{25B6} {} (current)", wt.name)
                                } else if wt.is_locked {
                                    format!("\u{25A0} {}", wt.name)
                                } else {
                                    wt.name.clone()
                                };
                                ui.label(egui::RichText::new(&label).strong());
                                ui.label(
                                    egui::RichText::new(wt.path.to_string_lossy().as_ref())
                                        .small()
                                        .color(egui::Color32::from_gray(140)),
                                );

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if !wt.is_current && !wt.is_locked {
                                            if ui.small_button("Remove").clicked() {
                                                remove_path = Some(wt.path.clone());
                                            }
                                        }
                                        if !wt.is_current {
                                            if ui.small_button("Switch").clicked() {
                                                switch_to = Some(wt.path.clone());
                                            }
                                        }
                                    },
                                );
                            });
                            ui.separator();
                        }

                        if self.worktrees.is_empty() {
                            ui.label(
                                egui::RichText::new("No worktrees found")
                                    .italics()
                                    .color(egui::Color32::from_gray(120)),
                            );
                        }
                    });

                ui.add_space(8.0);
                ui.label("Create new worktree:");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_worktree_name)
                            .desired_width(200.0)
                            .hint_text("branch-name")
                            .font(egui::TextStyle::Monospace),
                    );
                    if ui.button("Create").clicked() && !self.new_worktree_name.is_empty() {
                        create_name = Some(self.new_worktree_name.clone());
                    }
                });
            });

        self.show_worktree_panel = open;

        if let Some(path) = switch_to {
            self.show_worktree_panel = false;
            if let Ok(canonical) = std::fs::canonicalize(&path) {
                self.switch_repo(canonical);
            } else {
                self.switch_repo(path);
            }
        }

        if let Some(path) = remove_path {
            match git::remove_worktree(&self.project_root, &path) {
                Ok(()) => {
                    self.reload_worktrees();
                }
                Err(e) => {
                    eprintln!("Failed to remove worktree: {}", e);
                }
            }
        }

        if let Some(name) = create_name {
            match git::create_worktree(&self.project_root, &name) {
                Ok(_path) => {
                    self.new_worktree_name.clear();
                    self.reload_worktrees();
                }
                Err(e) => {
                    eprintln!("Failed to create worktree: {}", e);
                }
            }
        }
    }
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

impl DirigentApp {
    // Claude progress rendered in the central panel (replaces code viewer)
    fn render_running_log_central(&mut self, ctx: &egui::Context) {
        let cue_id = self.show_running_log.unwrap();

        let log_text = self
            .running_logs
            .get(&cue_id)
            .and_then(|log| log.lock().ok())
            .map(|log| log.clone())
            .unwrap_or_default();

        let is_running = self
            .cues
            .iter()
            .any(|c| c.id == cue_id && c.status == CueStatus::Ready);

        let cue_text = self
            .cues
            .iter()
            .find(|c| c.id == cue_id)
            .map(|c| {
                if c.text.len() > 80 {
                    format!("{}...", &c.text[..77])
                } else {
                    c.text.clone()
                }
            })
            .unwrap_or_default();

        let mut close = false;

        egui::CentralPanel::default().show(ctx, |ui| {
            // Header bar
            ui.horizontal(|ui| {
                if ui.button("\u{2190} Back").clicked() {
                    close = true;
                }
                ui.separator();
                ui.strong("Claude Progress");
                ui.separator();
                if is_running {
                    ui.label(
                        egui::RichText::new("\u{2022} Running")
                            .color(egui::Color32::from_rgb(100, 180, 255)),
                    );
                } else {
                    ui.label(
                        egui::RichText::new("\u{2713} Completed")
                            .color(egui::Color32::from_rgb(100, 200, 100)),
                    );
                }
                ui.separator();
                ui.label(
                    egui::RichText::new(&cue_text)
                        .small()
                        .color(egui::Color32::from_gray(160)),
                );
            });
            ui.separator();

            // Log content fills the rest
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if log_text.is_empty() {
                        ui.label(
                            egui::RichText::new("Waiting for output...")
                                .italics()
                                .color(egui::Color32::from_gray(120)),
                        );
                    } else {
                        ui.label(egui::RichText::new(&log_text).monospace().small());
                    }
                });
        });

        if close {
            self.show_running_log = None;
        }
    }
}

impl eframe::App for DirigentApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply theme if needed
        self.apply_theme(ctx);

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
