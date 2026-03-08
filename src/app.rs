use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::claude;
use crate::db::{Comment, CommentStatus, Database};
use crate::diff_view::{self, DiffViewMode, ParsedDiff};
use crate::file_tree::{FileEntry, FileTree};
use crate::git;
use crate::settings::{self, Settings, ThemeChoice};

/// Result of a background Claude invocation.
struct ClaudeResult {
    comment_id: i64,
    exec_id: i64,
    diff: Option<String>,
    response: String,
    error: Option<String>,
}

/// State for reviewing a diff before accepting/rejecting.
struct DiffReview {
    comment_id: i64,
    diff: String,
    comment_text: String,
    parsed: ParsedDiff,
    view_mode: DiffViewMode,
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
    selected_line: Option<usize>,
    comment_input: String,
    scroll_to_line: Option<usize>,

    // Comment pool
    comments: Vec<Comment>,

    // Claude execution
    claude_pending: Arc<Mutex<Vec<ClaudeResult>>>,

    // Diff review modal
    diff_review: Option<DiffReview>,

    // Git info
    git_info: Option<git::GitInfo>,

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
}

impl DirigentApp {
    pub fn new(project_root: PathBuf) -> Self {
        let db = Database::open(&project_root).expect("failed to open database");
        let file_tree = FileTree::scan(&project_root).ok();
        let comments = db.all_comments().unwrap_or_default();
        let git_info = git::read_git_info(&project_root);
        let settings = settings::load_settings(&project_root);
        let worktrees = git::list_worktrees(&project_root).unwrap_or_default();

        let syntax_theme = match settings.theme {
            ThemeChoice::Dark => egui_extras::syntax_highlighting::CodeTheme::dark(12.0),
            ThemeChoice::Light => egui_extras::syntax_highlighting::CodeTheme::light(12.0),
        };

        DirigentApp {
            project_root,
            db,
            file_tree,
            expanded_dirs: HashSet::new(),
            current_file: None,
            current_file_content: Vec::new(),
            selected_line: None,
            comment_input: String::new(),
            scroll_to_line: None,
            comments,
            claude_pending: Arc::new(Mutex::new(Vec::new())),
            diff_review: None,
            git_info,
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
        }
    }

    fn reload_comments(&mut self) {
        self.comments = self.db.all_comments().unwrap_or_default();
    }

    fn reload_git_info(&mut self) {
        self.git_info = git::read_git_info(&self.project_root);
    }

    fn load_file(&mut self, path: PathBuf) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            self.current_file_content = content.lines().map(String::from).collect();
            self.current_file = Some(path);
            self.selected_line = None;
            self.comment_input.clear();
        }
    }

    fn relative_path(&self, path: &PathBuf) -> String {
        path.strip_prefix(&self.project_root)
            .unwrap_or(path.as_path())
            .to_string_lossy()
            .to_string()
    }

    fn file_comments(&self) -> Vec<&Comment> {
        if let Some(ref current) = self.current_file {
            let rel = self.relative_path(current);
            self.comments
                .iter()
                .filter(|c| c.file_path == rel)
                .collect()
        } else {
            Vec::new()
        }
    }

    fn lines_with_comments(&self) -> HashSet<usize> {
        self.file_comments()
            .iter()
            .map(|c| c.line_number)
            .collect()
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
        self.comments = self.db.all_comments().unwrap_or_default();
        self.git_info = git::read_git_info(&self.project_root);
        self.current_file = None;
        self.current_file_content.clear();
        self.selected_line = None;
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

    fn trigger_claude(&mut self, comment_id: i64) {
        let comment = match self.comments.iter().find(|c| c.id == comment_id) {
            Some(c) => c.clone(),
            None => return,
        };

        let prompt = claude::build_prompt(&comment.text, &comment.file_path, comment.line_number);

        // Insert execution record
        let exec_id = self.db.insert_execution(comment_id, &prompt).unwrap_or(0);

        let project_root = self.project_root.clone();
        let pending = Arc::clone(&self.claude_pending);
        let model = self.settings.claude_model.clone();

        std::thread::spawn(move || {
            let result = match claude::invoke_claude(&prompt, &project_root, &model) {
                Ok(response) => {
                    let diff = claude::parse_diff_from_response(&response.stdout);
                    ClaudeResult {
                        comment_id,
                        exec_id,
                        diff,
                        response: response.stdout,
                        error: None,
                    }
                }
                Err(e) => ClaudeResult {
                    comment_id,
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
                eprintln!("Claude error for comment {}: {}", result.comment_id, error);
                let _ = self.db.fail_execution(result.exec_id, error);
                let _ = self
                    .db
                    .update_comment_status(result.comment_id, CommentStatus::Inbox);
            } else if let Some(ref diff) = result.diff {
                // Persist the execution result (so Diff button works later)
                let _ =
                    self.db
                        .complete_execution(result.exec_id, &result.response, Some(diff));

                // Apply diff to working tree immediately
                match git::apply_diff(&self.project_root, diff) {
                    Ok(()) => {
                        let _ = self.db.update_comment_status(
                            result.comment_id,
                            CommentStatus::Review,
                        );
                        // Reload current file so user sees changes
                        if let Some(ref path) = self.current_file {
                            let p = path.clone();
                            self.load_file(p);
                        }
                        self.reload_git_info();
                    }
                    Err(e) => {
                        eprintln!(
                            "Failed to apply diff for comment {}: {}",
                            result.comment_id, e
                        );
                        let _ = self.db.update_comment_status(
                            result.comment_id,
                            CommentStatus::Inbox,
                        );
                    }
                }
            } else {
                let _ =
                    self.db
                        .complete_execution(result.exec_id, &result.response, None);
                eprintln!(
                    "No diff found in Claude response for comment {}",
                    result.comment_id
                );
                let _ = self
                    .db
                    .update_comment_status(result.comment_id, CommentStatus::Inbox);
            }
            self.reload_comments();
        }
    }

    // -- UI rendering --

    // Feature 4: Repo bar at top
    fn render_repo_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("repo_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("\u{1F4C1} {}", self.project_root.display()))
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
                egui::ScrollArea::vertical().show(ui, |ui| {
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

    fn render_code_viewer(&mut self, ctx: &egui::Context) {
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

            let lines_with_comments = self.lines_with_comments();
            let num_lines = self.current_file_content.len();
            let line_height = 16.0;

            let ext = file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            let mut new_selected = self.selected_line;
            let mut submit_comment = false;

            let scroll_area = egui::ScrollArea::vertical().auto_shrink([false; 2]);

            scroll_area.show_rows(ui, line_height, num_lines, |ui, row_range| {
                for line_idx in row_range {
                    let line_num = line_idx + 1;
                    let line_text = self
                        .current_file_content
                        .get(line_idx)
                        .map(|s| s.as_str())
                        .unwrap_or("");

                    let is_selected = self.selected_line == Some(line_num);
                    let has_comment = lines_with_comments.contains(&line_num);

                    let response = ui.horizontal(|ui| {
                        if has_comment {
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

                        if is_selected {
                            ui.painter().rect_filled(
                                rect,
                                0.0,
                                egui::Color32::from_rgba_premultiplied(60, 60, 120, 80),
                            );
                        }

                        response
                    });

                    if response.inner.clicked() {
                        new_selected = Some(line_num);
                    }

                    if is_selected {
                        ui.horizontal(|ui| {
                            ui.label("     ");
                            ui.label(
                                egui::RichText::new(">")
                                    .monospace()
                                    .color(egui::Color32::from_rgb(100, 200, 100)),
                            );
                            let input_response = ui.add(
                                egui::TextEdit::singleline(&mut self.comment_input)
                                    .desired_width(ui.available_width() - 80.0)
                                    .hint_text("Add a comment...")
                                    .font(egui::TextStyle::Monospace),
                            );
                            if ui.button("Add").clicked()
                                || (input_response.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                            {
                                submit_comment = true;
                            }
                        });
                    }
                }
            });

            if new_selected != self.selected_line {
                self.selected_line = new_selected;
                self.comment_input.clear();
            }

            if submit_comment && !self.comment_input.is_empty() {
                if let Some(line_num) = self.selected_line {
                    let text = self.comment_input.clone();
                    let _ = self.db.insert_comment(&text, &rel_path, line_num);
                    self.comment_input.clear();
                    self.reload_comments();
                }
            }
        });
    }

    fn render_comment_pool(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("comment_pool")
            .default_width(250.0)
            .min_width(200.0)
            .show(ctx, |ui| {
                ui.heading("Comments");
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut actions: Vec<(i64, CommentAction)> = Vec::new();

                    for &status in CommentStatus::all() {
                        let section_comments: Vec<&Comment> = self
                            .comments
                            .iter()
                            .filter(|c| c.status == status)
                            .collect();

                        let header = format!("{} ({})", status.label(), section_comments.len());
                        egui::CollapsingHeader::new(header)
                            .default_open(
                                status == CommentStatus::Inbox || status == CommentStatus::Review,
                            )
                            .show(ui, |ui| {
                                if section_comments.is_empty() {
                                    ui.label(
                                        egui::RichText::new("(empty)")
                                            .italics()
                                            .color(egui::Color32::from_gray(120)),
                                    );
                                }
                                for comment in &section_comments {
                                    self.render_comment_card(ui, comment, &mut actions);
                                }
                            });
                    }

                    // Process actions after iteration
                    for (id, action) in actions {
                        match action {
                            CommentAction::MoveTo(new_status) => {
                                let _ = self.db.update_comment_status(id, new_status);
                                if new_status == CommentStatus::Ready {
                                    self.reload_comments();
                                    self.trigger_claude(id);
                                }
                            }
                            CommentAction::Delete => {
                                let _ = self.db.delete_comment(id);
                            }
                            CommentAction::Navigate(file_path, line) => {
                                let full_path = self.project_root.join(&file_path);
                                if self.current_file.as_ref() != Some(&full_path) {
                                    self.load_file(full_path);
                                }
                                self.selected_line = Some(line);
                                self.scroll_to_line = Some(line);
                            }
                            CommentAction::ShowDiff(comment_id) => {
                                if let Ok(Some(exec)) = self.db.get_latest_execution(comment_id) {
                                    if let Some(diff) = exec.diff {
                                        let text = self
                                            .comments
                                            .iter()
                                            .find(|c| c.id == comment_id)
                                            .map(|c| c.text.clone())
                                            .unwrap_or_default();
                                        let parsed = diff_view::parse_unified_diff(&diff);
                                        self.diff_review = Some(DiffReview {
                                            comment_id,
                                            diff,
                                            comment_text: text,
                                            parsed,
                                            view_mode: DiffViewMode::Inline,
                                        });
                                    }
                                }
                            }
                            CommentAction::CommitReview(comment_id) => {
                                if let Ok(Some(exec)) =
                                    self.db.get_latest_execution(comment_id)
                                {
                                    if let Some(ref diff) = exec.diff {
                                        let file_paths = git::parse_diff_file_paths_for_repo(&self.project_root,diff);
                                        let comment_text = self
                                            .comments
                                            .iter()
                                            .find(|c| c.id == comment_id)
                                            .map(|c| c.text.clone())
                                            .unwrap_or_default();
                                        let commit_msg =
                                            git::generate_commit_message(&comment_text);
                                        match git::stage_and_commit(
                                            &self.project_root,
                                            &file_paths,
                                            &commit_msg,
                                        ) {
                                            Ok(hash) => {
                                                eprintln!("Committed: {}", hash);
                                                let _ = self.db.update_comment_status(
                                                    comment_id,
                                                    CommentStatus::Done,
                                                );
                                            }
                                            Err(e) => {
                                                eprintln!("Commit failed: {}", e);
                                            }
                                        }
                                    }
                                }
                                self.reload_git_info();
                            }
                            CommentAction::RevertReview(comment_id) => {
                                if let Ok(Some(exec)) =
                                    self.db.get_latest_execution(comment_id)
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
                                let _ = self.db.update_comment_status(
                                    comment_id,
                                    CommentStatus::Rejected,
                                );
                                // Reload file to show reverted content
                                if let Some(ref path) = self.current_file {
                                    let p = path.clone();
                                    self.load_file(p);
                                }
                                self.reload_git_info();
                            }
                        }
                        self.reload_comments();
                    }
                });
            });
    }

    fn render_comment_card(
        &self,
        ui: &mut egui::Ui,
        comment: &Comment,
        actions: &mut Vec<(i64, CommentAction)>,
    ) {
        egui::Frame::none()
            .inner_margin(4.0)
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(60)))
            .rounding(4.0)
            .show(ui, |ui| {
                // Comment text (truncated)
                let display_text = if comment.text.len() > 60 {
                    format!("{}...", &comment.text[..57])
                } else {
                    comment.text.clone()
                };
                ui.label(&display_text);

                // File:line link or "Global" label
                if comment.file_path.is_empty() {
                    ui.label(
                        egui::RichText::new("Global")
                            .small()
                            .color(egui::Color32::from_rgb(180, 140, 255)),
                    );
                } else {
                    let location = format!("{}:{}", comment.file_path, comment.line_number);
                    if ui
                        .small_button(&location)
                        .on_hover_text("Navigate to this location")
                        .clicked()
                    {
                        actions.push((
                            comment.id,
                            CommentAction::Navigate(
                                comment.file_path.clone(),
                                comment.line_number,
                            ),
                        ));
                    }
                }

                // Action buttons
                ui.horizontal(|ui| {
                    match comment.status {
                        CommentStatus::Inbox => {
                            if ui
                                .small_button("\u{25B6} Ready")
                                .on_hover_text("Send to Claude")
                                .clicked()
                            {
                                actions.push((
                                    comment.id,
                                    CommentAction::MoveTo(CommentStatus::Ready),
                                ));
                            }
                            if ui
                                .small_button("\u{2713} Done")
                                .on_hover_text("Mark done (no Claude)")
                                .clicked()
                            {
                                actions.push((
                                    comment.id,
                                    CommentAction::MoveTo(CommentStatus::Done),
                                ));
                            }
                        }
                        CommentStatus::Ready => {
                            ui.label(
                                egui::RichText::new("Running...")
                                    .color(egui::Color32::from_rgb(100, 180, 255)),
                            );
                            if ui
                                .small_button("\u{2715} Cancel")
                                .on_hover_text("Cancel and move back to Inbox")
                                .clicked()
                            {
                                actions.push((
                                    comment.id,
                                    CommentAction::MoveTo(CommentStatus::Inbox),
                                ));
                            }
                        }
                        CommentStatus::Review => {
                            if ui
                                .small_button("\u{1F50D} Diff")
                                .on_hover_text("View the diff")
                                .clicked()
                            {
                                actions
                                    .push((comment.id, CommentAction::ShowDiff(comment.id)));
                            }
                            if ui
                                .small_button("\u{2713} Commit")
                                .on_hover_text("Commit the applied changes")
                                .clicked()
                            {
                                actions.push((
                                    comment.id,
                                    CommentAction::CommitReview(comment.id),
                                ));
                            }
                            if ui
                                .small_button("\u{21BA} Revert")
                                .on_hover_text("Revert changes and move back to Inbox")
                                .clicked()
                            {
                                actions.push((
                                    comment.id,
                                    CommentAction::RevertReview(comment.id),
                                ));
                            }
                        }
                        CommentStatus::Done => {
                            ui.label(
                                egui::RichText::new("\u{2713}")
                                    .color(egui::Color32::from_rgb(100, 200, 100)),
                            );
                            if ui
                                .small_button("\u{21BA} Reopen")
                                .on_hover_text("Move back to Inbox")
                                .clicked()
                            {
                                actions.push((
                                    comment.id,
                                    CommentAction::MoveTo(CommentStatus::Inbox),
                                ));
                            }
                        }
                        CommentStatus::Rejected => {
                            if ui
                                .small_button("\u{21BA} Retry")
                                .on_hover_text("Move back to Inbox")
                                .clicked()
                            {
                                actions.push((
                                    comment.id,
                                    CommentAction::MoveTo(CommentStatus::Inbox),
                                ));
                            }
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("\u{2715}")
                            .on_hover_text("Delete comment")
                            .clicked()
                        {
                            actions.push((comment.id, CommentAction::Delete));
                        }
                    });
                });
            });

        ui.add_space(2.0);
    }

    // Feature 3: Rewritten diff review with inline + side-by-side
    fn render_diff_review(&mut self, ctx: &egui::Context) {
        if self.diff_review.is_none() {
            return;
        }

        let mut close = false;
        let mut accept = false;
        let mut reject = false;
        let mut toggle_mode = None;

        let review = self.diff_review.as_ref().unwrap();
        let comment_id = review.comment_id;
        let diff_text = review.diff.clone();
        let comment_text = review.comment_text.clone();
        let parsed = review.parsed.clone();
        let view_mode = review.view_mode;

        egui::Window::new("Diff Review")
            .collapsible(false)
            .resizable(true)
            .default_size([800.0, 550.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(format!("Comment: {}", comment_text)).strong(),
                );
                ui.separator();

                // View mode toggle
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
                });
                ui.separator();

                egui::ScrollArea::both()
                    .max_height(400.0)
                    .show(ui, |ui| {
                        match view_mode {
                            DiffViewMode::Inline => {
                                diff_view::render_inline_diff(ui, &parsed);
                            }
                            DiffViewMode::SideBySide => {
                                diff_view::render_side_by_side_diff(ui, &parsed);
                            }
                        }
                    });

                ui.separator();
                ui.horizontal(|ui| {
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
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });

        if let Some(mode) = toggle_mode {
            if let Some(ref mut review) = self.diff_review {
                review.view_mode = mode;
            }
        }

        if accept {
            // Diff already applied to working tree — just commit
            let file_paths = git::parse_diff_file_paths_for_repo(&self.project_root,&diff_text);
            let commit_msg = git::generate_commit_message(&comment_text);
            match git::stage_and_commit(&self.project_root, &file_paths, &commit_msg) {
                Ok(hash) => {
                    eprintln!("Committed: {}", hash);
                    let _ = self
                        .db
                        .update_comment_status(comment_id, CommentStatus::Done);
                }
                Err(e) => {
                    eprintln!("Commit failed: {}", e);
                }
            }
            self.reload_comments();
            self.reload_git_info();
            self.diff_review = None;
        } else if reject {
            // Revert the applied changes
            let file_paths = git::parse_diff_file_paths_for_repo(&self.project_root,&diff_text);
            if let Err(e) = git::revert_files(&self.project_root, &file_paths) {
                eprintln!("Revert failed: {}", e);
            }
            let _ = self
                .db
                .update_comment_status(comment_id, CommentStatus::Rejected);
            if let Some(ref path) = self.current_file {
                let p = path.clone();
                self.load_file(p);
            }
            self.reload_comments();
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
                        egui::RichText::new(format!("\u{2387} {}", info.branch))
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

                    let total = self.comments.len();
                    let inbox = self
                        .comments
                        .iter()
                        .filter(|c| c.status == CommentStatus::Inbox)
                        .count();
                    let review = self
                        .comments
                        .iter()
                        .filter(|c| c.status == CommentStatus::Review)
                        .count();
                    ui.label(
                        egui::RichText::new(format!(
                            "{} comments ({} inbox, {} review)",
                            total, inbox, review
                        ))
                        .monospace()
                        .small(),
                    );
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
                    let _ = self.db.insert_comment(&text, "", 0);
                    self.global_prompt_input.clear();
                    self.reload_comments();
                }
            });
        });
    }

    // Feature 1: Settings window
    fn render_settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }

        let mut open = self.show_settings;
        let mut save = false;

        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_size([350.0, 200.0])
            .show(ctx, |ui| {
                egui::Grid::new("settings_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Theme:");
                        let theme_label = match self.settings.theme {
                            ThemeChoice::Dark => "Dark",
                            ThemeChoice::Light => "Light",
                        };
                        egui::ComboBox::from_id_salt("theme_combo")
                            .selected_text(theme_label)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.settings.theme,
                                    ThemeChoice::Dark,
                                    "Dark",
                                );
                                ui.selectable_value(
                                    &mut self.settings.theme,
                                    ThemeChoice::Light,
                                    "Light",
                                );
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
                    });

                ui.add_space(12.0);
                if ui.button("Save").clicked() {
                    save = true;
                }
            });

        self.show_settings = open;

        if save {
            settings::save_settings(&self.project_root, &self.settings);
            self.needs_theme_apply = true;
        }
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        if !self.needs_theme_apply {
            return;
        }
        self.needs_theme_apply = false;
        match self.settings.theme {
            ThemeChoice::Dark => {
                ctx.set_visuals(egui::Visuals::dark());
                self.syntax_theme = egui_extras::syntax_highlighting::CodeTheme::dark(12.0);
            }
            ThemeChoice::Light => {
                ctx.set_visuals(egui::Visuals::light());
                self.syntax_theme = egui_extras::syntax_highlighting::CodeTheme::light(12.0);
            }
        }
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
                                    format!("\u{1F512} {}", wt.name)
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

enum CommentAction {
    MoveTo(CommentStatus),
    Delete,
    Navigate(String, usize),
    ShowDiff(i64),
    CommitReview(i64),
    RevertReview(i64),
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
            .comments
            .iter()
            .any(|c| c.status == CommentStatus::Ready)
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }

        // Render all panels (order matters for layout)
        self.render_repo_bar(ctx); // top
        self.render_status_bar(ctx); // bottom-most
        self.render_prompt_field(ctx); // above status bar
        self.render_file_tree_panel(ctx); // left side
        self.render_comment_pool(ctx); // right side
        self.render_code_viewer(ctx); // center
        self.render_diff_review(ctx); // floating
        self.render_repo_picker(ctx); // floating
        self.render_settings_window(ctx); // floating
        self.render_worktree_panel(ctx); // floating
    }
}
