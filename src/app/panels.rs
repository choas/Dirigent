use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use eframe::egui;

use super::{
    icon, icon_small, DiffReview, DirigentApp, FONT_SCALE_SUBHEADING, SPACE_MD, SPACE_SM, SPACE_XS,
};
use crate::agents::{AgentKind, AgentStatus};
use crate::db::CueStatus;
use crate::diff_view::{self, DiffViewMode};
use crate::file_tree::FileEntry;
use crate::git;
use crate::prompt_hints;
use crate::prompt_suggestions;
use crate::settings::SemanticColors;

/// Bundled context for recursive file-tree rendering, reducing parameter count.
struct FileTreeCtx<'a> {
    expanded: &'a mut HashSet<PathBuf>,
    current_file: &'a Option<PathBuf>,
    action: &'a mut Option<FileTreeAction>,
    project_root: &'a Path,
    dirty_files: &'a HashMap<String, char>,
    semantic: &'a SemanticColors,
    depth: usize,
    font_size: f32,
    status_msg: &'a mut Option<String>,
}

/// Actions triggered from the file tree context menu.
enum FileTreeAction {
    Open(PathBuf),
    AddToGitignore(PathBuf),
    Delete(PathBuf, bool),
    RenameStart(PathBuf),
}

/// Actions deferred from the menu bar closures.
#[derive(Default)]
struct MenuBarActions {
    push_clicked: bool,
    pull_clicked: bool,
    create_pr_clicked: bool,
    import_pr_clicked: bool,
    run_all_agents: bool,
    agent_to_trigger: Option<AgentKind>,
    agent_to_cancel: Option<AgentKind>,
}

impl DirigentApp {
    pub(super) fn render_menu_bar(&mut self, ctx: &egui::Context) {
        let mut actions = MenuBarActions::default();

        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                self.render_dirigent_menu(ui);
                self.render_git_menu(ui, &mut actions);
                self.render_agents_menu(ui, &mut actions);
            });
        });

        self.apply_menu_bar_actions(actions);
    }

    fn render_dirigent_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Dirigent", |ui| {
            if ui.button("About Dirigent").clicked() {
                self.show_about = true;
                ui.close();
            }
            ui.separator();
            if ui.button("New Window  \u{2318}N").clicked() {
                crate::spawn_new_instance();
                ui.close();
            }
            ui.separator();
            if ui.button("Settings...").clicked() {
                self.dismiss_central_overlays();
                self.reload_settings_from_disk();
                self.show_settings = true;
                ui.close();
            }
        });
    }

    fn render_git_menu(&mut self, ui: &mut egui::Ui, actions: &mut MenuBarActions) {
        ui.menu_button("Git", |ui| {
            if self.git.info.is_none() {
                ui.label(
                    egui::RichText::new("No git repository")
                        .italics()
                        .color(self.semantic.tertiary_text),
                );
                return;
            }

            if let Some(ref info) = self.git.info {
                ui.label(egui::RichText::new(format!("\u{25CF} {}", info.branch)).strong());
                ui.separator();
            }

            self.render_git_menu_pull_push(ui, actions);

            ui.separator();

            self.render_git_menu_pr(ui, actions);
        });
    }

    fn render_git_menu_pull_push(&self, ui: &mut egui::Ui, actions: &mut MenuBarActions) {
        let pull_label = if self.git.pulling {
            "Pulling..."
        } else {
            "Pull"
        };
        if ui
            .add_enabled(!self.git.pulling, egui::Button::new(pull_label))
            .clicked()
        {
            actions.pull_clicked = true;
            ui.close();
        }

        if self.git.ahead_of_remote == 0 && !self.git.pushing {
            ui.add_enabled(false, egui::Button::new("  Nothing to push  "));
        } else {
            let push_label = if self.git.pushing {
                "Pushing..."
            } else {
                "Push"
            };
            if ui
                .add_enabled(!self.git.pushing, egui::Button::new(push_label))
                .clicked()
            {
                actions.push_clicked = true;
                ui.close();
            }
        }
    }

    fn render_git_menu_pr(&self, ui: &mut egui::Ui, actions: &mut MenuBarActions) {
        let is_default_branch = self
            .git
            .info
            .as_ref()
            .map(|i| i.branch == "main" || i.branch == "master")
            .unwrap_or(true);
        let pr_label = if self.git.creating_pr {
            "Creating PR..."
        } else {
            "Create Pull Request"
        };
        let pr_enabled = !self.git.creating_pr && !is_default_branch;
        if ui
            .add_enabled(pr_enabled, egui::Button::new(pr_label))
            .clicked()
        {
            actions.create_pr_clicked = true;
            ui.close();
        }

        let import_label = if self.git.importing_pr {
            "Importing PR..."
        } else {
            "Import PR Findings"
        };
        if ui
            .add_enabled(!self.git.importing_pr, egui::Button::new(import_label))
            .clicked()
        {
            actions.import_pr_clicked = true;
            ui.close();
        }
    }

    fn render_agents_menu(&mut self, ui: &mut egui::Ui, actions: &mut MenuBarActions) {
        ui.menu_button("Agents", |ui| {
            let enabled_agents: Vec<_> = self
                .settings
                .agents
                .iter()
                .filter(|a| a.enabled && !a.command.is_empty())
                .map(|a| {
                    let status = self
                        .agent_state
                        .statuses
                        .get(&a.kind)
                        .copied()
                        .unwrap_or(AgentStatus::Idle);
                    (
                        a.kind,
                        a.display_name().to_string(),
                        a.command.clone(),
                        status,
                    )
                })
                .collect();

            if enabled_agents.is_empty() {
                self.render_agents_menu_empty(ui);
                return;
            }

            self.render_agents_menu_run_all(ui, &enabled_agents, actions);
            ui.separator();
            Self::render_agents_menu_items(ui, &enabled_agents, &self.semantic, actions);

            ui.separator();
            if ui.button("Settings...").clicked() {
                self.dismiss_central_overlays();
                self.reload_settings_from_disk();
                self.show_settings = true;
                self.agents_expanded = true;
                ui.close();
            }
        });
    }

    fn render_agents_menu_empty(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new("No agents configured")
                .italics()
                .color(self.semantic.tertiary_text),
        );
        ui.separator();
        if ui.button("Open Settings...").clicked() {
            self.dismiss_central_overlays();
            self.reload_settings_from_disk();
            self.show_settings = true;
            self.agents_expanded = true;
            ui.close();
        }
    }

    fn render_agents_menu_run_all(
        &self,
        ui: &mut egui::Ui,
        enabled_agents: &[(AgentKind, String, String, AgentStatus)],
        actions: &mut MenuBarActions,
    ) {
        let any_idle = enabled_agents
            .iter()
            .any(|(_, _, _, s)| *s != AgentStatus::Running);
        if ui
            .add_enabled(any_idle, egui::Button::new("Run All"))
            .clicked()
        {
            actions.run_all_agents = true;
            ui.close();
        }
    }

    fn render_agents_menu_items(
        ui: &mut egui::Ui,
        enabled_agents: &[(AgentKind, String, String, AgentStatus)],
        semantic: &SemanticColors,
        actions: &mut MenuBarActions,
    ) {
        for (kind, name, command, status) in enabled_agents {
            let (status_icon, status_color) = match status {
                AgentStatus::Idle => ("", semantic.secondary_text),
                AgentStatus::Running => ("\u{21BB} ", semantic.accent),
                AgentStatus::Passed => ("\u{2713} ", semantic.success),
                AgentStatus::Failed => ("\u{2717} ", semantic.danger),
                AgentStatus::Error => ("! ", semantic.danger),
            };

            let is_running = *status == AgentStatus::Running;
            let label = format!("{}{}", status_icon, name);

            if is_running {
                if ui
                    .button(egui::RichText::new(&label).color(status_color))
                    .on_hover_text(format!("Cancel {}", name))
                    .clicked()
                {
                    actions.agent_to_cancel = Some(*kind);
                    ui.close();
                }
            } else if ui.button(&label).on_hover_text(command).clicked() {
                actions.agent_to_trigger = Some(*kind);
                ui.close();
            }
        }
    }

    fn apply_menu_bar_actions(&mut self, actions: MenuBarActions) {
        if actions.pull_clicked {
            self.start_git_pull();
        }
        if actions.push_clicked {
            self.start_git_push();
        }
        if actions.create_pr_clicked {
            self.open_create_pr_dialog();
        }
        if actions.import_pr_clicked {
            self.open_import_pr_dialog();
        }
        if let Some(kind) = actions.agent_to_cancel {
            self.cancel_agent(kind);
        }
        if actions.run_all_agents {
            self.run_all_agents();
        } else if let Some(kind) = actions.agent_to_trigger {
            self.trigger_agent_manual(kind);
        }
    }

    /// Trigger all enabled agents that are not currently running.
    fn run_all_agents(&mut self) {
        let kinds: Vec<AgentKind> = self
            .settings
            .agents
            .iter()
            .filter(|a| {
                a.enabled
                    && !a.command.is_empty()
                    && self.agent_state.statuses.get(&a.kind).copied() != Some(AgentStatus::Running)
            })
            .map(|a| a.kind)
            .collect();
        for kind in kinds {
            self.trigger_agent_manual(kind);
        }
    }

    pub(super) fn render_about_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }

        self.ensure_logo_texture(ctx);

        let mut open = self.show_about;
        egui::Window::new("About Dirigent")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.about_dialog_frame())
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    if let Some(ref tex) = self.logo_texture {
                        ui.add(egui::Image::new(tex).max_size(egui::vec2(128.0, 128.0)));
                    }
                    ui.add_space(SPACE_MD);
                    ui.heading("Dirigent");
                    ui.add_space(SPACE_XS);
                    ui.label(format!("Version {}", env!("BUILD_VERSION")));
                    ui.add_space(SPACE_SM);
                    ui.label(
                        egui::RichText::new(
                            "A read-only code viewer where humans direct and AI performs.",
                        )
                        .weak(),
                    );
                    ui.add_space(24.0);
                });
            });
        self.show_about = open;
    }

    // Feature 4: Repo bar at top
    pub(super) fn render_repo_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("repo_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(icon_small(
                    &format!("\u{25B6} {}", self.project_root.display()),
                    self.settings.font_size,
                ));
                if ui.small_button("Change...").clicked() {
                    self.repo_path_input = self.project_root.to_string_lossy().to_string();
                    self.show_repo_picker = true;
                }
                if ui.small_button("Worktrees").clicked() {
                    self.reload_worktrees();
                    self.git.show_worktree_panel = true;
                }
            });
        });
    }

    pub(super) fn render_file_tree_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("file_tree")
            .default_width(220.0)
            .min_width(150.0)
            .show(ctx, |ui| {
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
        let symbols = match self.viewer.active().map(|t| &t.symbols) {
            Some(s) if !s.is_empty() => s,
            _ => return,
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
                            let label = if kind_label.is_empty() {
                                sym.name.clone()
                            } else {
                                format!("{} {}", kind_label, sym.name)
                            };
                            if ui
                                .add(
                                    egui::Label::new(egui::RichText::new(&label).small())
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
                    let msg = if commit.message.len() > max_msg_chars + 3 {
                        format!("{}...", super::truncate_str(&commit.message, max_msg_chars))
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
                    if ui
                        .selectable_label(false, text)
                        .on_hover_text(hover)
                        .clicked()
                    {
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
        let name_color = dir_name_color(ui, entry.is_ignored, dir_has_dirty, ctx.semantic);
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
            .to_string();
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
            file_name_color(ui, entry.is_ignored, status_letter.is_some(), ctx.semantic);
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
                .to_string();
            return dirty_files.contains_key(&rel);
        }
        entry
            .children
            .iter()
            .any(|child| Self::dir_has_dirty_files(child, project_root, dirty_files))
    }

    pub(super) fn render_status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                self.render_status_bar_git_info(ui);
                self.render_status_bar_db_cost(ui);
                self.render_status_bar_agents(ui, ctx);
                self.render_status_bar_cached_cost(ui);
                self.render_status_bar_message(ui, ctx);
            });
        });
    }

    /// Render the git branch and status summary in the status bar.
    fn render_status_bar_git_info(&mut self, ui: &mut egui::Ui) {
        if let Some(ref info) = self.git.info {
            let branch_label = ui.label(icon_small(
                &format!("\u{25CF} {}", info.branch),
                self.settings.font_size,
            ));
            branch_label.on_hover_text(format!(
                "{} {}",
                info.last_commit_hash, info.last_commit_message
            ));
            let summary = git::format_status_summary(info);
            if !summary.is_empty() {
                ui.separator();
                ui.label(egui::RichText::new(summary).monospace().small());
            }
        } else if ui
            .add(
                egui::Label::new(
                    egui::RichText::new("not a git repository \u{2014} click to init")
                        .monospace()
                        .small()
                        .color(self.semantic.tertiary_text),
                )
                .sense(egui::Sense::click()),
            )
            .clicked()
        {
            self.git_init_confirm = Some(self.project_root.clone());
        }
    }

    /// Render the total DB cost (inline, left-aligned) in the status bar.
    fn render_status_bar_db_cost(&self, ui: &mut egui::Ui) {
        if let Ok(total_cost) = self.db.total_cost() {
            if total_cost > 0.0 {
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("${:.2}", total_cost))
                        .monospace()
                        .small()
                        .color(self.semantic.tertiary_text),
                )
                .on_hover_text("Total API cost for this project");
            }
        }
    }

    /// Render agent status indicators and request repaint while running.
    fn render_status_bar_agents(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let has_any_status = self
            .settings
            .agents
            .iter()
            .any(|a| a.enabled && self.agent_state.statuses.contains_key(&a.kind));

        if has_any_status {
            ui.separator();
            // Collect agent info to avoid borrowing self.settings while calling &mut self.
            let agent_items: Vec<(AgentKind, String)> = self
                .settings
                .agents
                .iter()
                .filter(|a| a.enabled)
                .map(|a| (a.kind, a.display_name().to_string()))
                .collect();
            for (kind, name) in &agent_items {
                self.render_single_agent_status(ui, *kind, name);
            }
        }
        if self
            .agent_state
            .statuses
            .values()
            .any(|s| *s == AgentStatus::Running)
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    /// Render a single agent's status indicator in the status bar.
    fn render_single_agent_status(&mut self, ui: &mut egui::Ui, kind: AgentKind, name: &str) {
        let status = self
            .agent_state
            .statuses
            .get(&kind)
            .copied()
            .unwrap_or(AgentStatus::Idle);
        let (icon_str, color) = match status {
            AgentStatus::Idle => return,
            AgentStatus::Running => ("\u{21BB}", self.semantic.accent),
            AgentStatus::Passed => ("\u{2713}", self.semantic.success),
            AgentStatus::Failed => ("\u{2717}", self.semantic.danger),
            AgentStatus::Error => ("!", self.semantic.danger),
        };
        let label_text = format!("{} {}", name, icon_str);
        let mut resp = ui.add(
            egui::Label::new(
                egui::RichText::new(&label_text)
                    .monospace()
                    .small()
                    .color(color),
            )
            .sense(egui::Sense::click()),
        );
        if let Some(output) = self.agent_state.latest_output.get(&kind) {
            let preview = if output.len() > 300 {
                format!("{}...", super::truncate_str(output, 300))
            } else {
                output.clone()
            };
            resp = resp.on_hover_text(preview);
        }
        if resp.clicked() {
            if self.agent_state.show_output == Some(kind) {
                self.agent_state.show_output = None;
            } else {
                self.agent_state.show_output = Some(kind);
            }
        }
    }

    /// Render the cached total cost (right-aligned) in the status bar.
    fn render_status_bar_cached_cost(&self, ui: &mut egui::Ui) {
        if self.cached_total_cost > 0.0 {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("${:.2}", self.cached_total_cost))
                        .monospace()
                        .small()
                        .color(self.semantic.muted_text()),
                )
                .on_hover_text("Total project cost across all runs");
            });
        }
    }

    /// Render the transient status message with auto-dismiss and fade.
    fn render_status_bar_message(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let busy = self.git.importing_pr
            || self.git.pushing
            || self.git.pulling
            || self.git.creating_pr
            || self.git.notifying_pr;
        let expired = !busy
            && matches!(&self.status_message, Some((_, when)) if when.elapsed().as_secs() >= 6);
        if expired {
            self.status_message = None;
        }
        if let Some((ref msg, ref when)) = self.status_message {
            let elapsed = when.elapsed().as_secs_f32();
            let alpha = if elapsed > 4.0 {
                ((6.0 - elapsed) / 2.0).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let color = self.semantic.status_message_with_alpha(alpha);
            ui.separator();
            ui.label(
                egui::RichText::new(msg.as_str())
                    .monospace()
                    .small()
                    .color(color),
            );
            if elapsed > 4.0 {
                ctx.request_repaint();
            }
        }
    }

    // Feature 2: Global prompt input
    pub(super) fn render_prompt_field(&mut self, ctx: &egui::Context) {
        let prompt_frame = egui::Frame::NONE
            .fill(self.semantic.prompt_surface())
            .inner_margin(egui::Margin::symmetric(SPACE_SM as i8, SPACE_SM as i8));

        egui::TopBottomPanel::bottom("prompt_field")
            .frame(prompt_frame)
            .show(ctx, |ui| {
                // Top border line to visually separate from content above
                let rect = ui.available_rect_before_wrap();
                ui.painter().hline(
                    rect.x_range(),
                    rect.top(),
                    egui::Stroke::new(1.0, self.semantic.prompt_border()),
                );

                self.render_prompt_attached_images(ui);
                self.render_prompt_input_row(ui);
                self.render_prompt_hints_and_suggestions(ui);
            });
    }

    /// Render attached image thumbnails above the prompt input.
    fn render_prompt_attached_images(&mut self, ui: &mut egui::Ui) {
        if self.global_prompt_images.is_empty() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new("Attached:")
                    .small()
                    .color(self.semantic.accent),
            );
            let mut remove_idx = None;
            for (i, path) in self.global_prompt_images.iter().enumerate() {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string_lossy().to_string());
                ui.label(egui::RichText::new(&name).monospace().small());
                if ui
                    .small_button("\u{2715}")
                    .on_hover_text("Remove")
                    .clicked()
                {
                    remove_idx = Some(i);
                }
            }
            if let Some(i) = remove_idx {
                self.global_prompt_images.remove(i);
            }
        });
        ui.add_space(SPACE_XS);
    }

    /// Render the main prompt input row (attach button, text edit, send button).
    fn render_prompt_input_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(icon("\u{25B6}", self.settings.font_size).color(self.semantic.accent));
            if ui
                .button(icon("+", self.settings.font_size))
                .on_hover_text("Attach files (or drag & drop)")
                .clicked()
            {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("All files", &["*"])
                    .pick_files()
                {
                    self.global_prompt_images.extend(paths);
                }
            }
            let input_response = ui.add(
                egui::TextEdit::multiline(&mut self.global_prompt_input)
                    .desired_width(ui.available_width() - 44.0)
                    .desired_rows(2)
                    .hint_text("Describe what you want...")
                    .font(egui::TextStyle::Monospace),
            );
            self.render_prompt_send_button(ui, &input_response);
        });
    }

    /// Render the send button and handle submit logic.
    fn render_prompt_send_button(&mut self, ui: &mut egui::Ui, input_response: &egui::Response) {
        ui.vertical_centered(|ui| {
            let input_h = input_response.rect.height();
            let btn_size = self.settings.font_size + 12.0;
            ui.add_space((input_h - btn_size) / 2.0);
            let send_btn = egui::Button::new(
                icon("\u{2191}", self.settings.font_size).color(self.semantic.accent_text()),
            )
            .fill(self.semantic.accent)
            .corner_radius(btn_size as u8 / 2)
            .min_size(egui::vec2(btn_size, btn_size));
            let btn_clicked = ui
                .add(send_btn)
                .on_hover_text("Create cue  (\u{2318}Enter to run)")
                .clicked();
            let (enter_submitted, cmd_enter) = if input_response.has_focus() {
                ui.input(|i| {
                    let pressed = i.key_pressed(egui::Key::Enter) && !i.modifiers.shift;
                    (
                        pressed && !i.modifiers.command,
                        pressed && i.modifiers.command,
                    )
                })
            } else {
                (false, false)
            };
            if (btn_clicked || enter_submitted || cmd_enter) && !self.global_prompt_input.is_empty()
            {
                self.submit_prompt(cmd_enter);
            }
        });
    }

    /// Submit the current prompt text as a new cue.
    fn submit_prompt(&mut self, run_immediately: bool) {
        let text = self.global_prompt_input.trim().to_string();
        if text.is_empty() {
            return;
        }
        let images: Vec<String> = self
            .global_prompt_images
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        match self.db.insert_cue(&text, "", 0, None, &images) {
            Ok(id) => {
                self.global_prompt_images.clear();
                self.global_prompt_input.clear();
                if run_immediately {
                    match self.db.update_cue_status(id, CueStatus::Ready) {
                        Ok(()) => {
                            self.claude.expand_running = true;
                            self.trigger_claude(id);
                        }
                        Err(e) => {
                            self.set_status_message(format!("Failed to update cue status: {e}"));
                        }
                    }
                }
                self.reload_cues();
            }
            Err(e) => {
                self.set_status_message(format!("Failed to create cue: {e}"));
                self.reload_cues();
            }
        }
    }

    /// Render prompt refinement hints and suggestions below the input.
    fn render_prompt_hints_and_suggestions(&self, ui: &mut egui::Ui) {
        if !self.settings.prompt_suggestions_enabled {
            return;
        }
        let hints = prompt_hints::analyze(&self.global_prompt_input);
        if !hints.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for hint in &hints {
                    ui.label(
                        egui::RichText::new(format!("\u{26A0} {}", hint.label))
                            .small()
                            .color(self.semantic.warning),
                    )
                    .on_hover_text(hint.detail);
                }
            });
        }

        let suggestions = prompt_suggestions::analyse_prompt(&self.global_prompt_input, false);
        if !suggestions.is_empty() {
            ui.add_space(SPACE_XS);
            for suggestion in &suggestions {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("\u{26A0} {}", suggestion.label))
                            .small()
                            .color(self.semantic.warning),
                    );
                    ui.label(
                        egui::RichText::new(suggestion.detail)
                            .small()
                            .color(self.semantic.muted_text()),
                    );
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Free helper functions for file tree rendering (extracted to reduce complexity)
// ---------------------------------------------------------------------------

/// Allocate a full-width clickable row for a file tree entry.
fn allocate_tree_row(ui: &mut egui::Ui) -> (egui::Rect, egui::Response) {
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
    let available_width = ui.available_width();
    ui.allocate_exact_size(
        egui::vec2(available_width, row_height),
        egui::Sense::click(),
    )
}

/// Paint a hover highlight behind a tree row if hovered.
fn paint_hover_highlight(ui: &egui::Ui, response: &egui::Response, row_rect: egui::Rect) {
    if response.hovered() {
        let hover = if ui.visuals().dark_mode {
            egui::Color32::from_white_alpha(15)
        } else {
            egui::Color32::from_black_alpha(12)
        };
        ui.painter().rect_filled(row_rect, 0, hover);
    }
}

/// Determine the display color for a directory name.
fn dir_name_color(
    ui: &egui::Ui,
    is_ignored: bool,
    has_dirty: bool,
    semantic: &SemanticColors,
) -> egui::Color32 {
    if is_ignored {
        ui.visuals().weak_text_color()
    } else if has_dirty {
        semantic.warning
    } else {
        ui.visuals().text_color()
    }
}

/// Determine the display color for a file name.
fn file_name_color(
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
fn paint_git_status_badge(
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
