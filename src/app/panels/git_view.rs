use std::collections::{BTreeMap, HashSet};

use eframe::egui;

use super::super::{DirigentApp, FONT_SCALE_SUBHEADING, SPACE_SM};
use super::file_tree::{allocate_tree_row, paint_git_status_badge, paint_hover_highlight};
use crate::diff_view::{self, DiffViewMode};
use crate::settings::SemanticColors;

use super::super::vcs_dispatch;

use super::super::types::GitViewDiffMode;

struct DirtyTreeNode {
    children: BTreeMap<String, DirtyTreeNode>,
    file_status: Option<(String, char)>,
}

impl DirtyTreeNode {
    fn new() -> Self {
        Self {
            children: BTreeMap::new(),
            file_status: None,
        }
    }

    fn insert(&mut self, components: &[&str], rel_path: &str, status: char) {
        if components.len() == 1 {
            let entry = self
                .children
                .entry(components[0].to_string())
                .or_insert_with(Self::new);
            entry.file_status = Some((rel_path.to_string(), status));
        } else {
            let entry = self
                .children
                .entry(components[0].to_string())
                .or_insert_with(Self::new);
            entry.insert(&components[1..], rel_path, status);
        }
    }

    fn is_dir(&self) -> bool {
        self.file_status.is_none()
    }
}

fn build_dirty_tree(files: &[(String, char)]) -> DirtyTreeNode {
    let mut root = DirtyTreeNode::new();
    for (path, status) in files {
        let components: Vec<&str> = path.split('/').collect();
        root.insert(&components, path, *status);
    }
    root
}

impl DirigentApp {
    pub(in super::super) fn render_git_view_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("git_view")
            .default_size(220.0)
            .min_size(150.0)
            .max_size(400.0)
            .show_inside(ui, |ui| {
                self.render_git_view_header(ui);
                ui.separator();
                self.render_git_view_mode_toggle(ui);
                ui.separator();
                self.render_git_view_footer(ui);
                self.render_git_view_file_list(ui);
            });
    }

    fn render_git_view_header(&mut self, ui: &mut egui::Ui) {
        let fs = self.settings.font_size * FONT_SCALE_SUBHEADING;
        ui.horizontal(|ui| {
            if ui
                .selectable_label(false, egui::RichText::new("Files").size(fs))
                .clicked()
            {
                self.git.show_git_view = false;
            }
            let label = format!("Changes ({})", self.git.dirty_files.len());
            let _ = ui.selectable_label(true, egui::RichText::new(label).size(fs));
        });
    }

    fn render_git_view_mode_toggle(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .selectable_label(
                    self.git.git_view_diff_mode == GitViewDiffMode::DiffOnly,
                    "Diff",
                )
                .clicked()
            {
                self.git.git_view_diff_mode = GitViewDiffMode::DiffOnly;
            }
            if ui
                .selectable_label(
                    self.git.git_view_diff_mode == GitViewDiffMode::FullFile,
                    "Full File",
                )
                .clicked()
            {
                self.git.git_view_diff_mode = GitViewDiffMode::FullFile;
            }
        });
    }

    fn render_git_view_file_list(&mut self, ui: &mut egui::Ui) {
        let mut sorted_files: Vec<(String, char)> = self
            .git
            .dirty_files
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        sorted_files.sort_by(|a, b| a.0.cmp(&b.0));

        let mut action: Option<GitViewAction> = None;
        let mut view_all = false;

        let tree = build_dirty_tree(&sorted_files);

        egui::ScrollArea::vertical()
            .id_salt("git_view_scroll")
            .show(ui, |ui| {
                if sorted_files.is_empty() {
                    ui.add_space(SPACE_SM);
                    ui.label(egui::RichText::new("No uncommitted changes").weak());
                    return;
                }

                ui.add_space(2.0);
                if ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new("View All Changes")
                                .small()
                                .color(ui.visuals().hyperlink_color),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .clicked()
                {
                    view_all = true;
                }
                ui.add_space(2.0);

                render_dirty_tree_children(
                    ui,
                    &tree,
                    0,
                    &mut self.git.git_view_expanded_dirs,
                    &self.git.selected_files,
                    &self.semantic,
                    self.settings.font_size,
                    "",
                    &mut action,
                );
            });

        if view_all {
            self.open_all_changes_diff();
        }
        match action {
            Some(GitViewAction::ClickFile(rel_path)) => {
                self.open_git_view_file(&rel_path);
            }
            Some(GitViewAction::Delete(rel_path)) => {
                let abs_path = self.project_root.join(&rel_path);
                self.pending_file_delete = Some((abs_path, false));
            }
            Some(GitViewAction::AddToGitignore(rel_path)) => {
                let abs_path = self.project_root.join(&rel_path);
                self.handle_add_to_gitignore(&abs_path);
                self.reload_git_info();
            }
            Some(GitViewAction::Restore(rel_path)) => {
                match vcs_dispatch::revert_files(
                    &self.settings.vcs_backend,
                    &self.settings.jj_cli_path,
                    &self.project_root,
                    &[rel_path.clone()],
                ) {
                    Ok(()) => {
                        self.git.selected_files.remove(&rel_path);
                        self.set_status_message(format!("Restored {rel_path}"));
                        self.reload_open_tabs();
                        self.reload_git_info();
                    }
                    Err(e) => {
                        self.set_status_message(format!("Restore failed: {e}"));
                    }
                }
            }
            Some(GitViewAction::ToggleSelect(rel_path)) => {
                if !self.git.selected_files.remove(&rel_path) {
                    self.git.selected_files.insert(rel_path);
                }
            }
            Some(GitViewAction::Reveal(rel_path)) => {
                let abs_path = self.project_root.join(&rel_path);
                if let Err(e) = super::file_tree::spawn_reveal(&abs_path) {
                    self.set_status_message(format!("Failed to reveal: {e}"));
                }
            }
            None => {}
        }
    }

    fn render_git_view_footer(&mut self, ui: &mut egui::Ui) {
        if self.git.dirty_files.is_empty() {
            return;
        }
        let has_selection = !self.git.selected_files.is_empty();
        egui::Panel::bottom("git_view_footer").show_inside(ui, |ui| {
            ui.add_space(SPACE_SM);
            let label = if has_selection {
                "\u{2714} Commit Selected"
            } else {
                "\u{2714} Commit Changes"
            };
            let btn =
                egui::Button::new(egui::RichText::new(label).color(self.semantic.accent_text()))
                    .fill(self.semantic.accent);
            if ui
                .add_sized([ui.available_width(), 0.0], btn)
                .on_hover_text("Analyze the changes and commit with an AI-generated message")
                .clicked()
            {
                self.open_commit_dialog_for_changes();
            }
            ui.add_space(SPACE_SM);
            let analyzing = self.change_set_generating;
            let analyze_label = if analyzing {
                "\u{29D7} Analyzing…"
            } else {
                "\u{2728} Analyze Changes"
            };
            if ui
                .add_enabled(
                    !analyzing,
                    egui::Button::new(analyze_label).min_size(egui::vec2(ui.available_width(), 0.0)),
                )
                .on_hover_text(
                    "Group the working tree into logical change sets for review \
                     (uses the selected CLI — slower but more precise)",
                )
                .clicked()
            {
                self.start_change_set_analysis();
            }
            if has_selection {
                ui.add_space(SPACE_SM);
                if ui
                    .add_sized(
                        [ui.available_width(), 0.0],
                        egui::Button::new("Reset Selected"),
                    )
                    .on_hover_text("Clear the current file selection")
                    .clicked()
                {
                    self.git.selected_files.clear();
                }
            }
            ui.add_space(SPACE_SM);
        });
    }

    fn open_git_view_file(&mut self, rel_path: &str) {
        match self.git.git_view_diff_mode {
            GitViewDiffMode::FullFile => {
                let abs_path = self.project_root.join(rel_path);
                self.push_nav_history();
                self.load_file(abs_path);
            }
            GitViewDiffMode::DiffOnly => {
                self.open_file_diff(rel_path);
            }
        }
    }

    fn open_all_changes_diff(&mut self) {
        if let Some(diff_text) = vcs_dispatch::get_working_diff(
            &self.settings.vcs_backend,
            &self.settings.jj_cli_path,
            &self.project_root,
            &[],
        ) {
            let parsed = diff_view::parse_unified_diff(&diff_text);
            self.dismiss_central_overlays();
            self.diff_review = Some(super::super::DiffReview {
                cue_id: 0,
                diff: diff_text,
                cue_text: "All uncommitted changes".to_string(),
                commit_hash: None,
                commit_author: None,
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
    }

    pub(super) fn expand_git_view_dirs(&mut self) {
        self.git.git_view_expanded_dirs.clear();
        for path in self.git.dirty_files.keys() {
            let mut accumulated = String::new();
            let components: Vec<&str> = path.split('/').collect();
            for component in &components[..components.len().saturating_sub(1)] {
                if accumulated.is_empty() {
                    accumulated = component.to_string();
                } else {
                    accumulated = format!("{}/{}", accumulated, component);
                }
                self.git.git_view_expanded_dirs.insert(accumulated.clone());
            }
        }
    }

    /// Open the Commit dialog for the working-copy changes (or the current
    /// selection) and immediately start analyzing the diff to draft a commit
    /// message via the configured CLI / Fast LLM.
    fn open_commit_dialog_for_changes(&mut self) {
        let mut files: Vec<String> = self.git.selected_files.iter().cloned().collect();
        files.sort_unstable();
        self.git.commit_files = files;
        self.git.commit_review_cue_id = None;
        self.git.commit_change_set_cue_id = None;
        self.git.commit_in_background = false;
        self.git.commit_message_input.clear();
        self.git.commit_needs_focus = true;
        self.git.show_commit_dialog = true;
        self.spawn_commit_message_suggestion();
    }
}

enum GitViewAction {
    ClickFile(String),
    Delete(String),
    AddToGitignore(String),
    Restore(String),
    ToggleSelect(String),
    Reveal(String),
}

/// Platform-specific label for the "reveal in file manager" menu item.
fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Reveal in Finder"
    } else if cfg!(target_os = "windows") {
        "Reveal in Explorer"
    } else {
        "Reveal in File Manager"
    }
}

fn render_dirty_tree_children(
    ui: &mut egui::Ui,
    node: &DirtyTreeNode,
    depth: usize,
    expanded: &mut HashSet<String>,
    selected_files: &HashSet<String>,
    semantic: &SemanticColors,
    font_size: f32,
    parent_path: &str,
    action: &mut Option<GitViewAction>,
) {
    let indent = depth as f32 * 16.0;

    for (name, child) in &node.children {
        let node_path = if parent_path.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", parent_path, name)
        };

        if child.is_dir() {
            let is_expanded = expanded.contains(&node_path);
            let (row_rect, response) = allocate_tree_row(ui);
            paint_hover_highlight(ui, &response, row_rect);

            let triangle = if is_expanded { "\u{25BC}" } else { "\u{25B6}" };
            let text_pos = row_rect.left_center() + egui::vec2(indent, 0.0);
            ui.painter().text(
                egui::pos2(text_pos.x + 6.0, text_pos.y),
                egui::Align2::LEFT_CENTER,
                triangle,
                egui::FontId::proportional(10.0),
                ui.visuals().weak_text_color(),
            );

            ui.painter().text(
                egui::pos2(text_pos.x + 20.0, text_pos.y),
                egui::Align2::LEFT_CENTER,
                name,
                egui::FontId::proportional(font_size),
                semantic.warning,
            );

            if response.clicked() {
                if is_expanded {
                    expanded.remove(&node_path);
                } else {
                    expanded.insert(node_path.clone());
                }
            }

            response.context_menu(|ui| {
                if ui.button(reveal_label()).clicked() {
                    *action = Some(GitViewAction::Reveal(node_path.clone()));
                    ui.close();
                }
            });

            if is_expanded {
                render_dirty_tree_children(
                    ui,
                    child,
                    depth + 1,
                    expanded,
                    selected_files,
                    semantic,
                    font_size,
                    &node_path,
                    action,
                );
            }
        } else if let Some((rel_path, status)) = &child.file_status {
            let (row_rect, response) = allocate_tree_row(ui);
            paint_hover_highlight(ui, &response, row_rect);

            let is_selected = selected_files.contains(rel_path.as_str());
            let status_color = match status {
                'D' => semantic.danger,
                'A' | '?' => semantic.success,
                _ => semantic.warning,
            };
            let text_pos = row_rect.left_center() + egui::vec2(indent + 20.0, 0.0);
            let name_offset = if is_selected {
                ui.painter().text(
                    text_pos,
                    egui::Align2::LEFT_CENTER,
                    "\u{2714} ",
                    egui::FontId::proportional(font_size),
                    semantic.accent,
                );
                14.0
            } else {
                0.0
            };
            ui.painter().text(
                text_pos + egui::vec2(name_offset, 0.0),
                egui::Align2::LEFT_CENTER,
                name,
                egui::FontId::proportional(font_size),
                status_color,
            );

            paint_git_status_badge(ui, row_rect, Some(*status), semantic);

            if response.clicked() {
                *action = Some(GitViewAction::ClickFile(rel_path.clone()));
            }

            response.context_menu(|ui| {
                let is_selected = selected_files.contains(rel_path.as_str());
                let select_label = if is_selected { "Deselect" } else { "Select" };
                if ui.button(select_label).clicked() {
                    *action = Some(GitViewAction::ToggleSelect(rel_path.clone()));
                    ui.close();
                }
                if ui.button("Restore").clicked() {
                    *action = Some(GitViewAction::Restore(rel_path.clone()));
                    ui.close();
                }
                if ui.button(reveal_label()).clicked() {
                    *action = Some(GitViewAction::Reveal(rel_path.clone()));
                    ui.close();
                }
                ui.separator();
                if ui
                    .button(egui::RichText::new("Delete File").color(semantic.danger))
                    .clicked()
                {
                    *action = Some(GitViewAction::Delete(rel_path.clone()));
                    ui.close();
                }
                if ui.button("Add to .gitignore").clicked() {
                    *action = Some(GitViewAction::AddToGitignore(rel_path.clone()));
                    ui.close();
                }
            });

            response.on_hover_text(format!("{} [{}]", rel_path, status));
        }
    }
}
