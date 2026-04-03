use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use eframe::egui;

use super::super::{DiffReview, DirigentApp, FONT_SCALE_SUBHEADING};
use crate::diff_view::{self, DiffViewMode};
use crate::file_tree::FileEntry;
use crate::git;
use crate::settings::SemanticColors;

/// Bundled context for recursive file-tree rendering, reducing parameter count.
pub(super) struct FileTreeCtx<'a> {
    pub expanded: &'a mut HashSet<PathBuf>,
    pub current_file: &'a Option<PathBuf>,
    pub action: &'a mut Option<FileTreeAction>,
    pub project_root: &'a Path,
    pub dirty_files: &'a HashMap<String, char>,
    pub semantic: &'a SemanticColors,
    pub depth: usize,
    pub font_size: f32,
    pub status_msg: &'a mut Option<String>,
}

/// Actions triggered from the file tree context menu.
pub(super) enum FileTreeAction {
    Open(PathBuf),
    AddToGitignore(PathBuf),
    Delete(PathBuf, bool),
    RenameStart(PathBuf),
}

impl DirigentApp {
    pub(in super::super) fn render_file_tree_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("file_tree")
            .default_size(220.0)
            .min_size(150.0)
            .max_size(400.0)
            .show_inside(ui, |ui| {
                ui.label(
                    egui::RichText::new("Files")
                        .size(self.settings.font_size * FONT_SCALE_SUBHEADING)
                        .strong(),
                );
                ui.separator();

                let file_tree_height = Self::compute_file_tree_height(
                    ui.available_height(),
                    self.viewer.active().is_some_and(|t| !t.symbols.is_empty()),
                    self.git.show_log,
                );
                let (tree_action, tree_status_msg) = egui::ScrollArea::vertical()
                    .id_salt("file_tree_scroll")
                    .max_height(file_tree_height)
                    .show(ui, |ui| {
                        let mut action = None;
                        let mut status_msg = None;
                        if let Some(ref tree) = self.file_tree {
                            let current_file = self.viewer.current_file().cloned();
                            let mut ctx = FileTreeCtx {
                                expanded: &mut self.expanded_dirs,
                                current_file: &current_file,
                                action: &mut action,
                                project_root: &self.project_root,
                                dirty_files: &self.git.dirty_files,
                                semantic: &self.semantic,
                                depth: 0,
                                font_size: self.settings.font_size,
                                status_msg: &mut status_msg,
                            };
                            for entry in &tree.entries {
                                Self::render_file_entry(ui, entry, &mut ctx);
                            }
                        }
                        (action, status_msg)
                    })
                    .inner;
                if let Some(msg) = tree_status_msg {
                    self.set_status_message(msg);
                }
                self.handle_file_tree_action(tree_action);

                ui.separator();

                self.render_symbol_outline(ui);
                self.render_git_log_section(ui);
            });
    }

    /// Compute the height available for the file tree scroll area.
    fn compute_file_tree_height(available: f32, has_outline: bool, git_log_open: bool) -> f32 {
        let reserved = match (has_outline, git_log_open) {
            (true, true) => 174.0 + available * 0.3,
            (true, false) => 174.0,
            (false, true) => available * 0.4,
            (false, false) => 24.0,
        };
        (available - reserved).max(80.0)
    }

    /// Process actions returned from the file tree (open, gitignore, delete, rename).
    fn handle_file_tree_action(&mut self, action: Option<FileTreeAction>) {
        match action {
            Some(FileTreeAction::Open(path)) => {
                self.push_nav_history();
                self.load_file(path);
            }
            Some(FileTreeAction::AddToGitignore(path)) => {
                self.handle_add_to_gitignore(&path);
            }
            Some(FileTreeAction::Delete(path, is_dir)) => {
                self.pending_file_delete = Some((path, is_dir));
            }
            Some(FileTreeAction::RenameStart(path)) => {
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                self.rename_target = Some(path);
                self.rename_buffer = name;
                self.rename_focus_requested = false;
            }
            None => {}
        }
    }

    /// Append a path to .gitignore.
    fn handle_add_to_gitignore(&mut self, path: &Path) {
        let rel = path
            .strip_prefix(&self.project_root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let gitignore = self.project_root.join(".gitignore");
        let entry_line = if path.is_dir() {
            format!("{}/", rel)
        } else {
            rel.clone()
        };
        let current = match std::fs::read_to_string(&gitignore) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                self.set_status_message(format!("Failed to read .gitignore: {}", e));
                return;
            }
        };
        let separator = if current.ends_with('\n') || current.is_empty() {
            ""
        } else {
            "\n"
        };
        if let Err(e) = std::fs::write(
            &gitignore,
            format!("{}{}{}\n", current, separator, entry_line),
        ) {
            self.set_status_message(format!("Failed to update .gitignore: {}", e));
        } else {
            self.set_status_message(format!("Added '{}' to .gitignore", entry_line));
            self.reload_file_tree();
        }
    }

    /// Render the symbol outline collapsible section.
    fn render_symbol_outline(&mut self, ui: &mut egui::Ui) {
        let Some(symbols) = self
            .viewer
            .active()
            .map(|t| &t.symbols)
            .filter(|s| !s.is_empty())
        else {
            return;
        };

        let outline_header = egui::CollapsingHeader::new(
            egui::RichText::new(format!("Outline ({})", symbols.len()))
                .size(self.settings.font_size * FONT_SCALE_SUBHEADING)
                .strong(),
        )
        .default_open(self.viewer.show_outline);
        let accent = self.semantic.accent;
        let outline_resp = outline_header.show(ui, |ui| {
            let mut scroll_to: Option<usize> = None;
            egui::ScrollArea::vertical()
                .id_salt("outline_scroll")
                .max_height(200.0)
                .show(ui, |ui| {
                    for sym in symbols {
                        let indent = sym.depth as f32 * 12.0;
                        ui.horizontal(|ui| {
                            ui.add_space(indent);
                            ui.label(
                                egui::RichText::new(sym.kind.icon())
                                    .monospace()
                                    .small()
                                    .color(accent),
                            );
                            let kind_label = sym.kind.label();
                            let mut label = sym.name.clone();
                            if !kind_label.is_empty() {
                                label = format!("{} {}", kind_label, sym.name);
                            }
                            if ui
                                .add(
                                    egui::Label::new(egui::RichText::new(&label).small())
                                        .truncate()
                                        .sense(egui::Sense::click()),
                                )
                                .clicked()
                            {
                                scroll_to = Some(sym.line);
                            }
                        });
                    }
                });
            scroll_to
        });
        self.viewer.show_outline = outline_resp.fully_open();
        if let Some(line) = outline_resp.body_returned.flatten() {
            self.viewer.scroll_to_line = Some(line);
        }

        ui.separator();
    }

    /// Render the git log collapsible section.
    fn render_git_log_section(&mut self, ui: &mut egui::Ui) {
        let ahead_label = if self.git.ahead_of_remote > 0 {
            format!(" [+{}]", self.git.ahead_of_remote)
        } else {
            String::new()
        };
        let header_text = format!(
            "Git Log ({}/{}){}",
            self.git.commit_history.len(),
            self.git.commit_history_total,
            ahead_label
        );
        let header_resp = egui::CollapsingHeader::new(
            egui::RichText::new(header_text)
                .size(self.settings.font_size * FONT_SCALE_SUBHEADING)
                .strong(),
        )
        .default_open(self.git.show_log)
        .show(ui, |ui| self.render_git_log_entries(ui));
        self.git.show_log = header_resp.fully_open();
        if let Some(Some((full_hash, message, body))) = header_resp.body_returned {
            self.open_commit_diff_review(&full_hash, &message, body);
        }
    }

    /// Render individual commit entries inside the git log scroll area.
    fn render_git_log_entries(&mut self, ui: &mut egui::Ui) -> Option<(String, String, String)> {
        let mut clicked_commit: Option<(String, String, String)> = None;
        let mut load_more = false;
        egui::ScrollArea::vertical()
            .id_salt("git_log_scroll")
            .show(ui, |ui| {
                let avail_width = ui.available_width();
                let char_width = self.settings.font_size * 0.52;
                let hash_prefix_len = 8;
                let max_msg_chars = ((avail_width / char_width) as usize)
                    .saturating_sub(hash_prefix_len)
                    .max(10);
                let ahead = self.git.ahead_of_remote;
                for (idx, commit) in self.git.commit_history.iter().enumerate() {
                    let is_unpushed = idx < ahead;
                    if Self::render_commit_entry(ui, commit, is_unpushed, max_msg_chars) {
                        clicked_commit = Some((
                            commit.full_hash.clone(),
                            commit.message.clone(),
                            commit.body.clone(),
                        ));
                    }
                }
                if self.git.commit_history.len() == self.git.commit_history_limit {
                    ui.add_space(4.0);
                    if ui
                        .button(
                            egui::RichText::new("Load More\u{2026}")
                                .small()
                                .color(ui.visuals().hyperlink_color),
                        )
                        .clicked()
                    {
                        load_more = true;
                    }
                }
            });
        if load_more {
            self.git.commit_history_limit += 10;
            self.reload_commit_history();
        }
        clicked_commit
    }

    /// Render a single commit entry and return whether it was clicked.
    fn render_commit_entry(
        ui: &mut egui::Ui,
        commit: &crate::git::CommitInfo,
        is_unpushed: bool,
        max_msg_chars: usize,
    ) -> bool {
        let msg = if commit.message.len() > max_msg_chars + 3 {
            format!(
                "{}...",
                super::super::truncate_str(&commit.message, max_msg_chars)
            )
        } else {
            commit.message.clone()
        };
        let dot = if is_unpushed { "\u{25CF} " } else { "" };
        let label = format!("{}{} {}", dot, commit.short_hash, msg);
        let mut text = egui::RichText::new(&label).monospace().small();
        if is_unpushed {
            text = text.color(ui.visuals().warn_fg_color);
        }
        let hover = Self::format_commit_hover(commit, is_unpushed);
        ui.selectable_label(false, text)
            .on_hover_text(hover)
            .clicked()
    }

    /// Format the hover tooltip for a commit entry.
    fn format_commit_hover(commit: &crate::git::CommitInfo, is_unpushed: bool) -> String {
        if is_unpushed {
            format!(
                "\u{2B06} Not pushed\n{} - {}\n{}\n{}",
                commit.short_hash, commit.author, commit.message, commit.time_ago
            )
        } else {
            format!(
                "{} - {}\n{}\n{}",
                commit.short_hash, commit.author, commit.message, commit.time_ago
            )
        }
    }

    /// Open a diff review for the given commit.
    fn open_commit_diff_review(&mut self, full_hash: &str, message: &str, body: String) {
        let short_hash = &full_hash[..7.min(full_hash.len())];
        let diff_text = git::get_commit_diff(&self.project_root, full_hash).unwrap_or_default();
        let parsed = diff_view::parse_unified_diff(&diff_text);
        let cue_text = if body.len() > message.len() {
            body
        } else {
            format!("{} {}", short_hash, message)
        };
        self.dismiss_central_overlays();
        self.diff_review = Some(DiffReview {
            cue_id: 0,
            diff: diff_text,
            cue_text,
            parsed,
            view_mode: DiffViewMode::Inline,
            read_only: true,
            collapsed_files: HashSet::new(),
            prompt_expanded: false,
            reply_text: String::new(),
            search_active: false,
            search_query: String::new(),
            search_matches: Vec::new(),
            search_current: None,
        });
    }

    fn render_file_entry(ui: &mut egui::Ui, entry: &FileEntry, ctx: &mut FileTreeCtx<'_>) {
        if entry.is_dir {
            Self::render_dir_entry(ui, entry, ctx);
        } else {
            Self::render_file_leaf_entry(ui, entry, ctx);
        }
    }

    /// Render a directory entry row (disclosure triangle, name, context menu, children).
    fn render_dir_entry(ui: &mut egui::Ui, entry: &FileEntry, ctx: &mut FileTreeCtx<'_>) {
        let indent = ctx.depth as f32 * 16.0;
        let is_expanded = ctx.expanded.contains(&entry.path);
        let dir_has_dirty = Self::dir_has_dirty_files(entry, ctx.project_root, ctx.dirty_files);

        let (row_rect, response) = allocate_tree_row(ui);
        paint_hover_highlight(ui, &response, row_rect);

        // Disclosure triangle
        let triangle = if is_expanded { "\u{25BC}" } else { "\u{25B6}" };
        let text_pos = row_rect.left_center() + egui::vec2(indent, 0.0);
        ui.painter().text(
            egui::pos2(text_pos.x + 6.0, text_pos.y),
            egui::Align2::LEFT_CENTER,
            triangle,
            egui::FontId::proportional(10.0),
            ui.visuals().weak_text_color(),
        );

        // Directory name
        let name_color = entry_name_color(ui, entry.is_ignored, dir_has_dirty, ctx.semantic);
        ui.painter().text(
            egui::pos2(text_pos.x + 20.0, text_pos.y),
            egui::Align2::LEFT_CENTER,
            &entry.name,
            egui::FontId::proportional(ctx.font_size),
            name_color,
        );

        if response.clicked() {
            if is_expanded {
                ctx.expanded.remove(&entry.path);
            } else {
                ctx.expanded.insert(entry.path.clone());
            }
        }

        render_dir_context_menu(
            &response,
            entry,
            ctx.project_root,
            ctx.semantic,
            ctx.action,
            ctx.status_msg,
        );

        if is_expanded {
            let child_depth = ctx.depth + 1;
            let prev_depth = ctx.depth;
            ctx.depth = child_depth;
            for child in &entry.children {
                Self::render_file_entry(ui, child, ctx);
            }
            ctx.depth = prev_depth;
        }
    }

    /// Render a file (leaf) entry row (name, git badge, context menu).
    fn render_file_leaf_entry(ui: &mut egui::Ui, entry: &FileEntry, ctx: &mut FileTreeCtx<'_>) {
        let indent = ctx.depth as f32 * 16.0;
        let is_selected = ctx.current_file.as_ref() == Some(&entry.path);
        let rel = entry
            .path
            .strip_prefix(ctx.project_root)
            .unwrap_or(&entry.path)
            .to_string_lossy()
            .replace('\\', "/");
        let status_letter = ctx.dirty_files.get(&rel).copied();

        let (row_rect, response) = allocate_tree_row(ui);

        if is_selected {
            ui.painter()
                .rect_filled(row_rect, 0, ctx.semantic.selection_bg());
        }
        if !is_selected {
            paint_hover_highlight(ui, &response, row_rect);
        }

        // File name
        let name_color =
            entry_name_color(ui, entry.is_ignored, status_letter.is_some(), ctx.semantic);
        let text_pos = row_rect.left_center() + egui::vec2(indent + 20.0, 0.0);
        ui.painter().text(
            text_pos,
            egui::Align2::LEFT_CENTER,
            &entry.name,
            egui::FontId::proportional(ctx.font_size),
            name_color,
        );

        paint_git_status_badge(ui, row_rect, status_letter, ctx.semantic);

        if response.clicked() {
            *ctx.action = Some(FileTreeAction::Open(entry.path.clone()));
        }

        render_file_context_menu(
            &response,
            entry,
            &rel,
            ctx.project_root,
            ctx.semantic,
            ctx.action,
            ctx.status_msg,
        );
    }

    /// Check if a directory contains any dirty files (recursively).
    fn dir_has_dirty_files(
        entry: &FileEntry,
        project_root: &Path,
        dirty_files: &HashMap<String, char>,
    ) -> bool {
        if !entry.is_dir {
            let rel = entry
                .path
                .strip_prefix(project_root)
                .unwrap_or(&entry.path)
                .to_string_lossy()
                .replace('\\', "/");
            return dirty_files.contains_key(&rel);
        }
        entry
            .children
            .iter()
            .any(|child| Self::dir_has_dirty_files(child, project_root, dirty_files))
    }
}

// ---------------------------------------------------------------------------
// Free helper functions for file tree rendering (extracted to reduce complexity)
// ---------------------------------------------------------------------------

/// Allocate a full-width clickable row for a file tree entry.
pub(super) fn allocate_tree_row(ui: &mut egui::Ui) -> (egui::Rect, egui::Response) {
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
    let available_width = ui.available_width();
    ui.allocate_exact_size(
        egui::vec2(available_width, row_height),
        egui::Sense::click(),
    )
}

/// Paint a hover highlight behind a tree row if hovered.
pub(super) fn paint_hover_highlight(
    ui: &egui::Ui,
    response: &egui::Response,
    row_rect: egui::Rect,
) {
    if response.hovered() {
        let hover = if ui.visuals().dark_mode {
            egui::Color32::from_white_alpha(15)
        } else {
            egui::Color32::from_black_alpha(12)
        };
        ui.painter().rect_filled(row_rect, 0, hover);
    }
}

/// Determine the display color for a file or directory name.
pub(super) fn entry_name_color(
    ui: &egui::Ui,
    is_ignored: bool,
    is_dirty: bool,
    semantic: &SemanticColors,
) -> egui::Color32 {
    if is_ignored {
        ui.visuals().weak_text_color()
    } else if is_dirty {
        semantic.warning
    } else {
        ui.visuals().text_color()
    }
}

/// Paint a git status badge character right-aligned in the row.
pub(super) fn paint_git_status_badge(
    ui: &egui::Ui,
    row_rect: egui::Rect,
    status_letter: Option<char>,
    semantic: &SemanticColors,
) {
    if let Some(letter) = status_letter {
        let badge_color = match letter {
            'D' => semantic.danger,
            'A' | '?' => semantic.success,
            _ => semantic.warning,
        };
        let badge_text = format!("{}", letter);
        let badge_pos = egui::pos2(row_rect.right() - 14.0, row_rect.center().y);
        ui.painter().text(
            badge_pos,
            egui::Align2::CENTER_CENTER,
            &badge_text,
            egui::FontId::monospace(10.0),
            badge_color,
        );
    }
}

/// Render the context menu for a directory entry.
fn render_dir_context_menu(
    response: &egui::Response,
    entry: &FileEntry,
    project_root: &Path,
    semantic: &SemanticColors,
    action: &mut Option<FileTreeAction>,
    status_msg: &mut Option<String>,
) {
    let entry_path = entry.path.clone();
    let rel_path = entry_path
        .strip_prefix(project_root)
        .unwrap_or(&entry_path)
        .to_string_lossy()
        .to_string();
    let is_ignored = entry.is_ignored;

    response.context_menu(|ui| {
        render_copy_path_items(ui, &entry_path, &rel_path);
        ui.separator();
        render_reveal_open_terminal_items(ui, &entry_path, &entry_path, status_msg);
        ui.separator();
        if !is_ignored && ui.button("Add to .gitignore").clicked() {
            *action = Some(FileTreeAction::AddToGitignore(entry_path.clone()));
            ui.close();
        }
        if ui.button("Rename\u{2026}").clicked() {
            *action = Some(FileTreeAction::RenameStart(entry_path.clone()));
            ui.close();
        }
        if ui
            .button(egui::RichText::new("Delete Directory\u{2026}").color(semantic.danger))
            .clicked()
        {
            *action = Some(FileTreeAction::Delete(entry_path.clone(), true));
            ui.close();
        }
    });
}

/// Render the context menu for a file entry.
fn render_file_context_menu(
    response: &egui::Response,
    entry: &FileEntry,
    rel: &str,
    _project_root: &Path,
    semantic: &SemanticColors,
    action: &mut Option<FileTreeAction>,
    status_msg: &mut Option<String>,
) {
    let entry_path = entry.path.clone();
    let rel_clone = rel.to_string();
    let parent_dir = entry_path.parent().unwrap_or(&entry_path).to_path_buf();
    let is_ignored = entry.is_ignored;

    response.context_menu(|ui| {
        render_copy_path_items(ui, &entry_path, &rel_clone);
        ui.separator();
        render_reveal_open_terminal_items(ui, &entry_path, &parent_dir, status_msg);
        ui.separator();
        if !is_ignored && ui.button("Add to .gitignore").clicked() {
            *action = Some(FileTreeAction::AddToGitignore(entry_path.clone()));
            ui.close();
        }
        if ui.button("Rename\u{2026}").clicked() {
            *action = Some(FileTreeAction::RenameStart(entry_path.clone()));
            ui.close();
        }
        if ui
            .button(egui::RichText::new("Delete File\u{2026}").color(semantic.danger))
            .clicked()
        {
            *action = Some(FileTreeAction::Delete(entry_path.clone(), false));
            ui.close();
        }
    });
}

/// Render "Copy Path" and "Copy Relative Path" context menu items.
fn render_copy_path_items(ui: &mut egui::Ui, abs_path: &Path, rel_path: &str) {
    if ui.button("Copy Path").clicked() {
        ui.ctx().copy_text(abs_path.to_string_lossy().to_string());
        ui.close();
    }
    if ui.button("Copy Relative Path").clicked() {
        ui.ctx().copy_text(rel_path.to_string());
        ui.close();
    }
}

/// Render "Reveal in File Manager" and "Open in Terminal" context menu items.
fn render_reveal_open_terminal_items(
    ui: &mut egui::Ui,
    reveal_path: &Path,
    terminal_path: &Path,
    status_msg: &mut Option<String>,
) {
    let reveal_label = if cfg!(target_os = "macos") {
        "Reveal in Finder"
    } else if cfg!(target_os = "windows") {
        "Reveal in Explorer"
    } else {
        "Reveal in File Manager"
    };

    if ui.button(reveal_label).clicked() {
        match spawn_reveal(reveal_path) {
            Ok(_) => ui.close(),
            Err(e) => {
                *status_msg = Some(format!("Failed to reveal: {e}"));
            }
        }
    }
    if ui.button("Open in Terminal").clicked() {
        match spawn_terminal(terminal_path) {
            Ok(_) => ui.close(),
            Err(e) => {
                *status_msg = Some(format!("Failed to open terminal: {e}"));
            }
        }
    }
}

/// Open the system file manager to reveal the given path.
fn spawn_reveal(path: &Path) -> std::io::Result<std::process::Child> {
    if cfg!(target_os = "macos") {
        if path.is_file() {
            std::process::Command::new("open")
                .arg("-R")
                .arg(path)
                .spawn()
        } else {
            std::process::Command::new("open").arg(path).spawn()
        }
    } else if cfg!(target_os = "windows") {
        if path.is_file() {
            std::process::Command::new("explorer")
                .arg(format!("/select,\"{}\"", path.display()))
                .spawn()
        } else {
            std::process::Command::new("explorer").arg(path).spawn()
        }
    } else {
        // Linux / other: xdg-open on the parent directory for files
        let target = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        std::process::Command::new("xdg-open").arg(target).spawn()
    }
}

/// Open a terminal emulator at the given directory.
fn spawn_terminal(dir: &Path) -> std::io::Result<std::process::Child> {
    if cfg!(target_os = "macos") {
        std::process::Command::new("open")
            .args(["-a", "Terminal"])
            .arg(dir)
            .spawn()
    } else if cfg!(target_os = "windows") {
        // Try Windows Terminal first, fall back to cmd.exe
        std::process::Command::new("wt")
            .arg("-d")
            .arg(dir)
            .spawn()
            .or_else(|_| {
                std::process::Command::new("cmd.exe")
                    .args(["/C", "start", "cmd.exe"])
                    .current_dir(dir)
                    .spawn()
            })
    } else {
        // Linux: try common terminals in order of preference
        std::process::Command::new("gnome-terminal")
            .arg(format!("--working-directory={}", dir.display()))
            .spawn()
            .or_else(|_| {
                std::process::Command::new("konsole")
                    .arg(format!("--workdir={}", dir.display()))
                    .spawn()
            })
            .or_else(|_| {
                std::process::Command::new("x-terminal-emulator")
                    .current_dir(dir)
                    .spawn()
            })
            .or_else(|_| std::process::Command::new("xdg-open").arg(dir).spawn())
    }
}
