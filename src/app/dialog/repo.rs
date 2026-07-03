use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM};
use crate::app::util::expand_tilde;
use crate::git;
use crate::jj;
use crate::settings::{SemanticColors, VcsBackend};

/// Format a byte size as a human-readable string (KB or MB).
fn format_size(size_bytes: u64) -> String {
    let size_kb = size_bytes as f64 / 1024.0;
    if size_kb >= 1024.0 {
        format!("{:.1} MB", size_kb / 1024.0)
    } else {
        format!("{:.0} KB", size_kb)
    }
}

/// Reveal a file or directory in the platform's file manager.
fn reveal_in_file_manager(path: &std::path::Path) -> Result<(), std::io::Error> {
    #[cfg(target_os = "macos")]
    let mut child = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()?;
    #[cfg(target_os = "windows")]
    let mut child = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn()?;
    #[cfg(target_os = "linux")]
    let mut child = {
        let dir = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| path.to_path_buf())
        };
        std::process::Command::new("xdg-open").arg(&dir).spawn()?
    };

    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Worktree label based on its current/locked state.
fn worktree_label(wt: &git::WorktreeInfo) -> String {
    // Solid diamond for the main checkout, hollow for secondary worktrees.
    let role = if wt.is_main { "\u{25C6}" } else { "\u{25C7}" };
    let mut s = format!("{role} {}", wt.name);
    if wt.is_current {
        s.push_str(" (current)");
    }
    if wt.is_locked {
        s.push_str(" \u{1F512}");
    }
    if wt.orphaned {
        s.push_str(" (orphaned)");
    }
    s
}

fn branch_push_status_label(branch: &str, status: Option<&jj::BookmarkPushStatus>) -> String {
    match status {
        Some(jj::BookmarkPushStatus::NotPushed) => format!("{branch}  \u{2191} not pushed"),
        Some(jj::BookmarkPushStatus::NeedsPush) => format!("{branch}  \u{2191} needs push"),
        Some(jj::BookmarkPushStatus::Synced) => format!("{branch}  \u{2713} synced"),
        None => branch.to_string(),
    }
}

fn branch_label_text(label: &str, is_own: bool, accent: egui::Color32) -> egui::RichText {
    let text = egui::RichText::new(label).monospace();
    if is_own {
        text.color(accent)
    } else {
        text
    }
}

fn render_checked_out_branch_label(ui: &mut egui::Ui, branch: &str, color: egui::Color32) {
    ui.add_enabled(
        false,
        egui::Label::new(
            egui::RichText::new(format!("{branch}  (in a worktree)"))
                .monospace()
                .color(color),
        ),
    );
}

fn force_remove_message(dirty: &HashMap<String, char>) -> String {
    let mut files: Vec<_> = dirty.iter().collect();
    files.sort_by_key(|(path, _)| (*path).clone());
    let listing: Vec<String> = files
        .iter()
        .take(10)
        .map(|(path, status)| format!("  {} {}", status, path))
        .collect();
    let mut msg = format!(
        "{} modified or untracked file(s):\n{}",
        dirty.len(),
        listing.join("\n")
    );
    if dirty.len() > 10 {
        msg.push_str(&format!("\n  … and {} more", dirty.len() - 10));
    }
    msg
}

/// Accumulated deferred actions from the worktree panel UI.
#[derive(Default)]
struct WorktreeActions {
    switch_to: Option<PathBuf>,
    remove_path: Option<PathBuf>,
    create_name: Option<String>,
    delete_archive_pending: Option<PathBuf>,
    reveal_path: Option<PathBuf>,
    /// Run `git worktree prune` to clear orphaned registrations.
    prune: bool,
    /// Compare this worktree folder against another (picked) folder.
    compare_left: Option<PathBuf>,
}

impl DirigentApp {
    /// Render a confirmation dialog with Cancel and a destructive confirm button.
    /// The `body` closure draws the dialog content; `confirm_label` is the red button text.
    /// Returns `(confirmed, cancelled)`.
    fn confirmation_dialog(
        &self,
        ctx: &egui::Context,
        title: &str,
        confirm_label: &str,
        width: f32,
        body: impl FnOnce(&mut egui::Ui, &SemanticColors),
    ) -> (bool, bool) {
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .default_size([width, 0.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                body(ui, &self.semantic);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(egui::RichText::new(confirm_label).color(egui::Color32::RED))
                            .clicked()
                        {
                            confirm = true;
                        }
                    });
                });
            });
        (confirm, cancel)
    }

    /// Render an italic empty-state label in tertiary text color.
    fn empty_label(&self, ui: &mut egui::Ui, text: &str) {
        ui.label(
            egui::RichText::new(text)
                .italics()
                .color(self.semantic.tertiary_text),
        );
    }

    // ── Repo picker ──────────────────────────────────────────────────

    /// Repo picker window.
    pub(in crate::app) fn render_repo_picker(&mut self, ctx: &egui::Context) {
        if !self.show_repo_picker {
            return;
        }

        // Populate the cached list of existing repos once when the picker opens,
        // instead of calling is_dir() on every frame.
        if self.cached_existing_repos.is_empty() && !self.settings.recent_repos.is_empty() {
            self.cached_existing_repos = self
                .settings
                .recent_repos
                .iter()
                .filter(|p| Path::new(p.as_str()).is_dir())
                .cloned()
                .collect();
        }

        let mut open = self.show_repo_picker;
        let mut switch_to: Option<PathBuf> = None;
        let mut error_msg: Option<String> = None;

        egui::Window::new("Open Repository")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_size([450.0, 300.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.interact(ui.max_rect(), ui.id().with("body_bg"), egui::Sense::click());
                self.render_repo_path_input(ui, &mut switch_to, &mut error_msg);
                ui.separator();
                self.render_recent_repos(ui, &mut switch_to);
            });

        self.show_repo_picker = open;
        if !open {
            self.cached_existing_repos.clear();
        }
        self.apply_repo_picker_result(switch_to, error_msg);
    }

    /// Path input row with Open button.
    fn render_repo_path_input(
        &mut self,
        ui: &mut egui::Ui,
        switch_to: &mut Option<PathBuf>,
        error_msg: &mut Option<String>,
    ) {
        ui.horizontal(|ui| {
            ui.label("Path:");
            ui.add(
                egui::TextEdit::singleline(&mut self.repo_path_input)
                    .desired_width(300.0)
                    .font(egui::TextStyle::Monospace),
            );
            if ui.button("Open").clicked() {
                let path = expand_tilde(&self.repo_path_input);
                if let Ok(canonical) = std::fs::canonicalize(&path) {
                    *switch_to = Some(canonical);
                } else {
                    *error_msg = Some(format!("Path not found: {}", path.display()));
                }
            }
        });
    }

    /// Recent repositories list.
    fn render_recent_repos(&self, ui: &mut egui::Ui, switch_to: &mut Option<PathBuf>) {
        ui.label("Recent repositories:");
        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                for repo_path in &self.cached_existing_repos {
                    if ui.button(repo_path.as_str()).clicked() {
                        let path = PathBuf::from(repo_path.as_str());
                        if let Ok(canonical) = std::fs::canonicalize(&path) {
                            *switch_to = Some(canonical);
                        }
                    }
                }
                if self.cached_existing_repos.is_empty() {
                    self.empty_label(ui, "(none)");
                }
            });
    }

    /// Apply the result of the repo picker dialog.
    fn apply_repo_picker_result(&mut self, switch_to: Option<PathBuf>, error_msg: Option<String>) {
        if let Some(msg) = error_msg {
            self.set_status_message(msg);
        }
        if let Some(new_root) = switch_to {
            self.show_repo_picker = false;
            self.cached_existing_repos.clear();
            self.switch_repo(new_root);
        }
    }

    // ── Switch Branch dialog ────────────────────────────────────────

    /// Dialog to switch to a different branch.
    pub(in crate::app) fn render_switch_branch_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_switch_branch {
            return;
        }

        let mut open = true;
        let mut switch_to: Option<String> = None;
        let mut create: Option<String> = None;

        let title = if self.settings.vcs_backend == VcsBackend::Jj {
            "Switch Bookmark"
        } else {
            "Switch Branch"
        };
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([350.0, 300.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                self.render_switch_branch_body(ui, &mut switch_to, &mut create);
            });

        if !open {
            self.git.show_switch_branch = false;
        }

        if let Some(name) = create {
            self.perform_branch_create(&name);
        } else if let Some(branch) = switch_to {
            self.perform_branch_switch(&branch);
        }
    }

    /// Body of the Switch Branch dialog: a create field, the current branch
    /// label, and the list of branches to switch to.
    fn render_switch_branch_body(
        &mut self,
        ui: &mut egui::Ui,
        switch_to: &mut Option<String>,
        create: &mut Option<String>,
    ) {
        self.render_switch_branch_create_row(ui, create);
        ui.add_space(6.0);
        self.render_current_branch_label(ui);

        if self.git.available_branches.is_empty() {
            self.empty_label(ui, self.empty_branch_list_message());
            return;
        }

        self.render_switch_branch_list(ui, switch_to);
    }

    fn render_switch_branch_create_row(&mut self, ui: &mut egui::Ui, create: &mut Option<String>) {
        let is_jj = self.settings.vcs_backend == VcsBackend::Jj;
        ui.horizontal(|ui| {
            let hint = if is_jj { "New bookmark" } else { "New branch" };
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.git.new_branch_name)
                    .hint_text(hint)
                    .desired_width(220.0),
            );
            let name = self.git.new_branch_name.trim().to_string();
            let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let clicked = ui
                .add_enabled(!name.is_empty(), egui::Button::new("Create"))
                .clicked();
            if !name.is_empty() && (clicked || submit) {
                *create = Some(name);
            }
        });
    }

    fn render_current_branch_label(&self, ui: &mut egui::Ui) {
        if let Some(ref info) = self.git.info {
            ui.label(
                egui::RichText::new(format!("Current: {}", info.branch))
                    .small()
                    .color(self.semantic.secondary_text),
            );
            ui.add_space(4.0);
        }
    }

    fn empty_branch_list_message(&self) -> &'static str {
        match self.settings.vcs_backend {
            VcsBackend::Jj => "No other bookmarks available",
            VcsBackend::Git => "No other branches available",
        }
    }

    fn render_switch_branch_list(&self, ui: &mut egui::Ui, switch_to: &mut Option<String>) {
        egui::ScrollArea::vertical()
            .max_height(250.0)
            .show(ui, |ui| {
                for branch in &self.git.available_branches {
                    if self.render_switch_branch_row(ui, branch) {
                        *switch_to = Some(branch.clone());
                    }
                }
            });
    }

    fn render_switch_branch_row(&self, ui: &mut egui::Ui, branch: &str) -> bool {
        let label = branch_push_status_label(branch, self.git.bookmark_push_statuses.get(branch));
        let text = branch_label_text(
            &label,
            self.git.own_branches.contains(branch),
            self.semantic.accent,
        );
        ui.selectable_label(self.is_trunk_branch(branch), text)
            .clicked()
    }

    fn is_trunk_branch(&self, branch: &str) -> bool {
        self.git
            .trunk_bookmarks
            .iter()
            .any(|bookmark| bookmark == branch)
            || branch == "main"
            || branch == "master"
    }

    /// Execute a branch switch and update state accordingly.
    fn perform_branch_switch(&mut self, branch: &str) {
        self.git.show_switch_branch = false;
        let switch_result = match self.settings.vcs_backend {
            VcsBackend::Jj => {
                jj::jj_checkout_bookmark(&self.project_root, branch, &self.settings.jj_cli_path)
            }
            VcsBackend::Git => git::checkout_branch(&self.project_root, branch),
        };
        match switch_result {
            Ok(()) => {
                self.git.active_bookmark = Some(branch.to_string());
                self.set_status_message(format!("Switched to '{}'", branch));
                self.reload_git_info();
                self.reload_commit_history();
                self.force_reload_open_tabs();
                self.reload_file_tree();
                self.git.recompute_dirty_dirs(&self.project_root);
            }
            Err(e) => {
                let noun = if self.settings.vcs_backend == VcsBackend::Jj {
                    "bookmark"
                } else {
                    "branch"
                };
                self.set_status_message(format!("Failed to switch {}: {}", noun, e));
            }
        }
    }

    /// Create a new branch (or bookmark) and switch to it.
    fn perform_branch_create(&mut self, name: &str) {
        let create_result = match self.settings.vcs_backend {
            VcsBackend::Jj => {
                jj::jj_create_bookmark(&self.project_root, name, &self.settings.jj_cli_path)
            }
            VcsBackend::Git => git::create_branch(&self.project_root, name),
        };
        let noun = if self.settings.vcs_backend == VcsBackend::Jj {
            "bookmark"
        } else {
            "branch"
        };
        match create_result {
            Ok(()) => {
                self.git.new_branch_name.clear();
                self.git.active_bookmark = Some(name.to_string());
                self.set_status_message(format!("Created {} '{}'", noun, name));
                self.reload_git_info();
                self.reload_commit_history();
                self.force_reload_open_tabs();
                self.reload_file_tree();
                self.git.recompute_dirty_dirs(&self.project_root);
                // Refresh the dialog's branch list so the new branch appears.
                self.refresh_branch_list();
            }
            Err(e) => {
                self.set_status_message(format!("Failed to create {}: {}", noun, e));
            }
        }
    }

    // ── Worktree panel ───────────────────────────────────────────────

    /// Worktree panel.
    pub(in crate::app) fn render_worktree_panel(&mut self, ctx: &egui::Context) {
        if !self.git.show_worktree_panel {
            return;
        }

        let mut open = self.git.show_worktree_panel;
        let mut actions = WorktreeActions::default();

        egui::Window::new("Git Worktrees")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([400.0, 300.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.interact(ui.max_rect(), ui.id().with("body_bg"), egui::Sense::click());
                self.render_worktree_list(ui, &mut actions);
                self.render_create_worktree_input(ui, &mut actions);
                self.render_archived_dbs_section(ui, &mut actions);
            });

        self.git.show_worktree_panel = open;
        self.handle_worktree_actions(actions);
    }

    /// Scrollable list of existing worktrees with Switch/Remove buttons.
    fn render_worktree_list(&self, ui: &mut egui::Ui, actions: &mut WorktreeActions) {
        let fs = self.settings.font_size;
        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                for wt in &self.git.worktrees {
                    self.render_worktree_row(ui, wt, fs, actions);
                    ui.separator();
                }

                if self.git.worktrees.is_empty() {
                    self.empty_label(ui, "No worktrees found");
                }
            });

        // Offer a prune action when git still lists worktrees whose folders are
        // gone (deleted by hand), which can otherwise confuse later operations.
        if self.git.worktrees.iter().any(|w| w.orphaned) {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("\u{26A0} Orphaned worktrees detected")
                        .small()
                        .color(self.semantic.warning),
                );
                if ui
                    .small_button("Prune")
                    .on_hover_text(
                        "git worktree prune — clear registrations whose folders are gone",
                    )
                    .clicked()
                {
                    actions.prune = true;
                }
            });
        }
    }

    /// Single worktree row with label and action buttons.
    fn render_worktree_row(
        &self,
        ui: &mut egui::Ui,
        wt: &git::WorktreeInfo,
        fs: f32,
        actions: &mut WorktreeActions,
    ) {
        ui.horizontal(|ui| {
            let label = worktree_label(wt);
            ui.label(icon(&label, fs).strong());
            ui.label(
                egui::RichText::new(wt.path.to_string_lossy().as_ref())
                    .small()
                    .color(self.semantic.secondary_text),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !wt.is_current
                    && !wt.is_locked
                    && !wt.is_main
                    && ui
                        .small_button("Retire")
                        .on_hover_text(
                            "Remove this worktree's folder and unregister it. \
                             The branch and its commits are kept.",
                        )
                        .clicked()
                {
                    actions.remove_path = Some(wt.path.clone());
                }
                // A switch only makes sense for a worktree whose folder still exists.
                if !wt.is_current && !wt.orphaned && ui.small_button("Switch").clicked() {
                    actions.switch_to = Some(wt.path.clone());
                }
                if !wt.orphaned
                    && ui
                        .small_button("Compare\u{2026}")
                        .on_hover_text("Compare this worktree folder against another folder")
                        .clicked()
                {
                    actions.compare_left = Some(wt.path.clone());
                }
            });
        });
    }

    /// Input field, branch picker, and Create button for new worktrees.
    fn render_create_worktree_input(&mut self, ui: &mut egui::Ui, actions: &mut WorktreeActions) {
        ui.add_space(SPACE_SM);
        ui.label("Create new worktree:");

        if !self.git.available_branches.is_empty() {
            if let Some(branch) = self.render_worktree_branch_picker(ui) {
                self.git.new_worktree_name = branch;
            }
        }

        self.render_worktree_name_input(ui, actions);
    }

    fn render_worktree_branch_picker(&self, ui: &mut egui::Ui) -> Option<String> {
        ui.add_space(2.0);
        self.render_worktree_hint(ui, "Pick an existing branch:");
        let checked_out = self.checked_out_worktree_names();
        let selected = self.render_worktree_branch_picker_rows(ui, &checked_out);
        ui.add_space(2.0);
        self.render_worktree_hint(ui, "Or type a new branch name:");
        selected
    }

    fn render_worktree_hint(&self, ui: &mut egui::Ui, text: &str) {
        ui.label(
            egui::RichText::new(text)
                .small()
                .color(self.semantic.secondary_text),
        );
    }

    fn checked_out_worktree_names(&self) -> HashSet<&str> {
        self.git
            .worktrees
            .iter()
            .filter(|w| !w.orphaned)
            .map(|w| w.name.as_str())
            .collect()
    }

    fn render_worktree_branch_picker_rows(
        &self,
        ui: &mut egui::Ui,
        checked_out: &HashSet<&str>,
    ) -> Option<String> {
        let mut selected_branch = None;
        egui::ScrollArea::vertical()
            .id_salt("branch_picker")
            .max_height(120.0)
            .show(ui, |ui| {
                for branch in self.git.available_branches.iter() {
                    if checked_out.contains(branch.as_str()) {
                        render_checked_out_branch_label(ui, branch, self.semantic.tertiary_text);
                    } else if self.render_worktree_branch_option(ui, branch) {
                        selected_branch = Some(branch.clone());
                    }
                }
            });
        selected_branch
    }

    fn render_worktree_branch_option(&self, ui: &mut egui::Ui, branch: &str) -> bool {
        let label = branch_push_status_label(branch, self.git.bookmark_push_statuses.get(branch));
        ui.selectable_label(
            self.git.new_worktree_name == branch,
            egui::RichText::new(&label).monospace(),
        )
        .clicked()
    }

    fn render_worktree_name_input(&mut self, ui: &mut egui::Ui, actions: &mut WorktreeActions) {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.git.new_worktree_name)
                    .desired_width(200.0)
                    .hint_text("branch-name")
                    .font(egui::TextStyle::Monospace),
            );
            if ui.button("Create").clicked() {
                let name = self.git.new_worktree_name.trim();
                if !name.is_empty() {
                    actions.create_name = Some(name.to_string());
                }
            }
        });
    }

    /// Collapsible section listing archived worktree databases.
    fn render_archived_dbs_section(&mut self, ui: &mut egui::Ui, actions: &mut WorktreeActions) {
        if self.git.archived_dbs.is_empty() {
            return;
        }

        ui.add_space(SPACE_SM);
        let arrow = if self.git.show_archived_dbs {
            "\u{25BC}"
        } else {
            "\u{25B6}"
        };
        let header = format!(
            "{} Archived Worktree DBs ({})",
            arrow,
            self.git.archived_dbs.len()
        );
        if ui
            .selectable_label(false, egui::RichText::new(&header).strong())
            .clicked()
        {
            self.git.show_archived_dbs = !self.git.show_archived_dbs;
        }

        if !self.git.show_archived_dbs {
            return;
        }

        for db in &self.git.archived_dbs {
            self.render_archived_db_row(ui, db, actions);
        }
    }

    /// Single archived DB row with name, size, and action buttons.
    fn render_archived_db_row(
        &self,
        ui: &mut egui::Ui,
        db: &git::ArchivedDb,
        actions: &mut WorktreeActions,
    ) {
        ui.horizontal(|ui| {
            ui.label(&db.name);
            ui.label(
                egui::RichText::new(format_size(db.size_bytes))
                    .small()
                    .color(self.semantic.secondary_text),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Delete").clicked() {
                    actions.delete_archive_pending = Some(db.path.clone());
                }
                if ui.small_button("Reveal").clicked() {
                    actions.reveal_path = Some(db.path.clone());
                }
            });
        });
    }

    /// Process all deferred actions collected during the worktree panel frame.
    fn handle_worktree_actions(&mut self, actions: WorktreeActions) {
        if let Some(path) = actions.switch_to {
            self.switch_to_worktree(path);
        }

        if let Some(path) = actions.remove_path {
            self.do_remove_worktree(path, false);
        }

        if let Some(name) = actions.create_name {
            self.create_worktree_from_action(&name);
        }

        if let Some(path) = actions.delete_archive_pending {
            self.git.pending_delete_archive = Some(path);
        }

        if let Some(path) = actions.reveal_path {
            self.reveal_worktree_path(&path);
        }

        if actions.prune {
            self.prune_worktrees_from_action();
        }

        if let Some(left) = actions.compare_left {
            self.start_folder_compare(left);
        }
    }

    fn switch_to_worktree(&mut self, path: PathBuf) {
        self.git.show_worktree_panel = false;
        match std::fs::canonicalize(&path) {
            Ok(canonical) => self.switch_repo(canonical),
            Err(_) => self.switch_repo(path),
        }
    }

    fn create_worktree_from_action(&mut self, name: &str) {
        match self.create_worktree(name) {
            Ok(_path) => {
                self.git.new_worktree_name.clear();
                self.reload_worktrees();
            }
            Err(e) => {
                self.set_status_message(format!(
                    "Failed to create {}: {}",
                    self.worktree_kind_label(),
                    e
                ));
            }
        }
    }

    fn create_worktree(&self, name: &str) -> crate::error::Result<PathBuf> {
        match self.settings.vcs_backend {
            VcsBackend::Jj => {
                jj::jj_create_workspace(&self.project_root, name, &self.settings.jj_cli_path)
            }
            VcsBackend::Git => git::create_worktree(&self.project_root, name),
        }
    }

    fn reveal_worktree_path(&mut self, path: &Path) {
        if let Err(e) = reveal_in_file_manager(path) {
            self.set_status_message(format!("Failed to reveal in file manager: {}", e));
        }
    }

    fn prune_worktrees_from_action(&mut self) {
        match git::prune_worktrees(&self.project_root) {
            Ok(()) => {
                self.set_status_message("Pruned orphaned worktrees".into());
                self.reload_worktrees();
            }
            Err(e) => self.set_status_message(format!("Prune failed: {e}")),
        }
    }

    // ── Worktree removal ─────────────────────────────────────────────

    /// Attempt to remove a worktree, archiving its DB first.
    /// If removal fails due to modified/untracked files, stores state for the
    /// force-remove confirmation dialog instead of just showing a status message.
    fn do_remove_worktree(&mut self, path: PathBuf, force: bool) {
        if let Some(msg) = self.force_remove_warning(&path, force) {
            self.git.pending_force_remove = Some((path, msg));
            self.git.pending_archive_msg = None;
            return;
        }

        let label = self.worktree_kind_label();
        let Some(archive_msg) = self.archive_worktree_before_remove(&path, label) else {
            return;
        };

        match self.remove_worktree(&path, force) {
            Ok(()) => {
                self.git.pending_force_remove = None;
                self.git.pending_archive_msg = None;
                self.reload_worktrees();
                self.report_worktree_removed(label, archive_msg);
            }
            Err(e) => {
                self.set_status_message(format!("Failed to remove {}: {}", label, e));
            }
        }
    }

    fn force_remove_warning(&self, path: &Path, force: bool) -> Option<String> {
        if force {
            return None;
        }
        let dirty = super::super::vcs_dispatch::get_dirty_files(
            &self.settings.vcs_backend,
            &self.settings.jj_cli_path,
            path,
        );
        (!dirty.is_empty()).then(|| force_remove_message(&dirty))
    }

    fn archive_worktree_before_remove(
        &mut self,
        path: &Path,
        label: &str,
    ) -> Option<Option<String>> {
        let main_path = match git::main_worktree_path(&self.project_root) {
            Ok(main_path) => main_path,
            Err(e) => {
                self.set_status_message(format!(
                    "Cannot remove {}: could not determine main worktree path: {}",
                    label, e
                ));
                return None;
            }
        };
        let wt_name = self.worktree_name_for_path(path, "unknown");
        match git::archive_worktree_db(&main_path, path, &wt_name) {
            Ok(Some(archive_path)) => Some(Some(format!(
                "DB archived as {}",
                archive_path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default()
            ))),
            Ok(None) => Some(None),
            Err(e) => {
                self.set_status_message(format!(
                    "Cannot remove {}: failed to archive DB: {}",
                    label, e
                ));
                None
            }
        }
    }

    fn remove_worktree(&self, path: &Path, force: bool) -> crate::error::Result<()> {
        match self.settings.vcs_backend {
            VcsBackend::Jj => {
                let ws_name = self.worktree_name_for_path(path, "");
                jj::jj_remove_workspace(&self.project_root, &ws_name, &self.settings.jj_cli_path)
            }
            VcsBackend::Git => git::remove_worktree(&self.project_root, path, force),
        }
    }

    fn worktree_name_for_path(&self, path: &Path, fallback: &str) -> String {
        self.git
            .worktrees
            .iter()
            .find(|wt| wt.path == path)
            .map(|wt| wt.name.clone())
            .or_else(|| path.file_name().map(|f| f.to_string_lossy().to_string()))
            .unwrap_or_else(|| fallback.to_string())
    }

    fn worktree_kind_label(&self) -> &'static str {
        match self.settings.vcs_backend {
            VcsBackend::Jj => "workspace",
            VcsBackend::Git => "worktree",
        }
    }

    fn report_worktree_removed(&mut self, label: &str, archive_msg: Option<String>) {
        if let Some(msg) = archive_msg {
            let noun = if label == "workspace" {
                "Workspace"
            } else {
                "Worktree"
            };
            self.set_status_message(format!("{noun} removed. {msg}"));
        }
    }

    /// Renders the force-remove confirmation dialog when a worktree has
    /// modified or untracked files.
    pub(in crate::app) fn render_force_remove_dialog(&mut self, ctx: &egui::Context) {
        let Some((ref path, ref error_msg)) = self.git.pending_force_remove else {
            return;
        };
        let path = path.clone();
        let error_msg = error_msg.clone();

        let (force, cancel) = self.confirmation_dialog(
            ctx,
            "Remove Worktree",
            "Force Remove",
            420.0,
            |ui, semantic| {
                ui.label("The worktree contains modified or untracked files:");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(path.to_string_lossy().as_ref())
                        .monospace()
                        .color(semantic.secondary_text),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(&error_msg)
                        .small()
                        .color(semantic.tertiary_text),
                );
                ui.add_space(8.0);
                ui.label("Force remove will delete all changes in this worktree.");
            },
        );

        if cancel {
            self.git.pending_force_remove = None;
            self.git.pending_archive_msg = None;
        } else if force {
            self.git.pending_force_remove = None;
            self.do_remove_worktree(path, true);
        }
    }

    /// Renders the confirmation dialog for deleting an archived worktree DB.
    pub(in crate::app) fn render_delete_archive_dialog(&mut self, ctx: &egui::Context) {
        let Some(ref path) = self.git.pending_delete_archive else {
            return;
        };
        let path = path.clone();

        let (confirm, cancel) = self.confirmation_dialog(
            ctx,
            "Delete Archived DB",
            "Delete",
            400.0,
            |ui, semantic| {
                ui.label("Are you sure you want to delete this archived database?");
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(path.to_string_lossy().as_ref())
                        .monospace()
                        .color(semantic.secondary_text),
                );
                ui.add_space(8.0);
                ui.label("This action cannot be undone.");
            },
        );

        if cancel {
            self.git.pending_delete_archive = None;
        } else if confirm {
            self.git.pending_delete_archive = None;
            match std::fs::remove_file(&path) {
                Ok(()) => self.reload_worktrees(),
                Err(e) => self.set_status_message(format!("Failed to delete archived DB: {}", e)),
            }
        }
    }
}
