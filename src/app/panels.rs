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
use crate::settings::SemanticColors;

impl DirigentApp {
    pub(super) fn render_menu_bar(&mut self, ctx: &egui::Context) {
        let mut push_clicked = false;
        let mut pull_clicked = false;
        let mut create_pr_clicked = false;
        let mut import_pr_clicked = false;
        let mut run_all_agents = false;
        let mut agent_to_trigger: Option<AgentKind> = None;
        let mut agent_to_cancel: Option<AgentKind> = None;

        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
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

                // Git menu
                ui.menu_button("Git", |ui| {
                    let has_repo = self.git.info.is_some();
                    if !has_repo {
                        ui.label(
                            egui::RichText::new("No git repository")
                                .italics()
                                .color(self.semantic.tertiary_text),
                        );
                        return;
                    }

                    // Show branch info
                    if let Some(ref info) = self.git.info {
                        ui.label(egui::RichText::new(format!("\u{25CF} {}", info.branch)).strong());
                        ui.separator();
                    }

                    // Pull
                    let pull_label = if self.git.pulling {
                        "Pulling..."
                    } else {
                        "Pull"
                    };
                    if ui
                        .add_enabled(!self.git.pulling, egui::Button::new(pull_label))
                        .clicked()
                    {
                        pull_clicked = true;
                        ui.close();
                    }

                    // Push
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
                            push_clicked = true;
                            ui.close();
                        }
                    }

                    ui.separator();

                    // Create PR (disabled on default branch)
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
                        create_pr_clicked = true;
                        ui.close();
                    }

                    // Import PR Findings
                    let import_label = if self.git.importing_pr {
                        "Importing PR..."
                    } else {
                        "Import PR Findings"
                    };
                    if ui
                        .add_enabled(!self.git.importing_pr, egui::Button::new(import_label))
                        .clicked()
                    {
                        import_pr_clicked = true;
                        ui.close();
                    }
                });

                // Agents menu
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
                        return;
                    }

                    // Run All button
                    let any_idle = enabled_agents
                        .iter()
                        .any(|(_, _, _, s)| *s != AgentStatus::Running);
                    if ui
                        .add_enabled(any_idle, egui::Button::new("Run All"))
                        .clicked()
                    {
                        run_all_agents = true;
                        ui.close();
                    }
                    ui.separator();

                    for (kind, name, command, status) in &enabled_agents {
                        let (status_icon, status_color) = match status {
                            AgentStatus::Idle => ("", self.semantic.secondary_text),
                            AgentStatus::Running => ("\u{21BB} ", self.semantic.accent),
                            AgentStatus::Passed => ("\u{2713} ", self.semantic.success),
                            AgentStatus::Failed => ("\u{2717} ", self.semantic.danger),
                            AgentStatus::Error => ("! ", self.semantic.danger),
                        };

                        let is_running = *status == AgentStatus::Running;
                        let label = format!("{}{}", status_icon, name);

                        if is_running {
                            if ui
                                .button(egui::RichText::new(&label).color(status_color))
                                .on_hover_text(format!("Cancel {}", name))
                                .clicked()
                            {
                                agent_to_cancel = Some(*kind);
                                ui.close();
                            }
                        } else {
                            if ui.button(&label).on_hover_text(command).clicked() {
                                agent_to_trigger = Some(*kind);
                                ui.close();
                            }
                        }
                    }

                    ui.separator();
                    if ui.button("Settings...").clicked() {
                        self.dismiss_central_overlays();
                        self.reload_settings_from_disk();
                        self.show_settings = true;
                        self.agents_expanded = true;
                        ui.close();
                    }
                });
            });
        });

        // Handle deferred actions outside the UI closure
        if pull_clicked {
            self.start_git_pull();
        }
        if push_clicked {
            self.start_git_push();
        }
        if create_pr_clicked {
            self.open_create_pr_dialog();
        }
        if import_pr_clicked {
            self.open_import_pr_dialog();
        }
        if let Some(kind) = agent_to_cancel {
            self.cancel_agent(kind);
        }
        if run_all_agents {
            self.run_all_agents();
        } else if let Some(kind) = agent_to_trigger {
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
                // File tree takes remaining space above git log
                let git_log_open = self.git.show_log;
                let available = ui.available_height();
                // When git log is open, give file tree ~60% of space; otherwise all of it
                let file_tree_height = if git_log_open {
                    available * 0.6
                } else {
                    available - 24.0 // leave room for the git log header
                };
                let file_to_load = egui::ScrollArea::vertical()
                    .id_salt("file_tree_scroll")
                    .max_height(file_tree_height)
                    .show(ui, |ui| {
                        let mut file_to_load = None;
                        if let Some(ref tree) = self.file_tree {
                            for entry in &tree.entries {
                                Self::render_file_entry(
                                    ui,
                                    entry,
                                    &mut self.expanded_dirs,
                                    &self.viewer.current_file,
                                    &mut file_to_load,
                                    &self.project_root,
                                    &self.git.dirty_files,
                                    &self.semantic,
                                    0,
                                    self.settings.font_size,
                                );
                            }
                        }
                        file_to_load
                    })
                    .inner;
                if let Some(path) = file_to_load {
                    self.load_file(path);
                }

                ui.separator();

                // Git Log collapsible section
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
                .show(ui, |ui| {
                    let mut clicked_commit: Option<(String, String, String)> = None;
                    let mut load_more = false;
                    egui::ScrollArea::vertical()
                        .id_salt("git_log_scroll")
                        .show(ui, |ui| {
                            // Estimate how many characters fit based on the
                            // available panel width and the monospace small font.
                            let avail_width = ui.available_width();
                            let char_width = self.settings.font_size * 0.52; // monospace small approx
                            let hash_prefix_len = 8; // "abcdef0 "
                            let max_msg_chars = ((avail_width / char_width) as usize)
                                .saturating_sub(hash_prefix_len)
                                .max(10);
                            for commit in &self.git.commit_history {
                                let msg = if commit.message.len() > max_msg_chars + 3 {
                                    format!(
                                        "{}...",
                                        super::truncate_str(&commit.message, max_msg_chars)
                                    )
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
                                        "{} - {}\n{}\n{}",
                                        commit.short_hash,
                                        commit.author,
                                        commit.message,
                                        commit.time_ago
                                    ))
                                    .clicked()
                                {
                                    clicked_commit = Some((
                                        commit.full_hash.clone(),
                                        commit.message.clone(),
                                        commit.body.clone(),
                                    ));
                                }
                            }
                            // Show "Load More" if we might have more commits
                            if self.git.commit_history.len() == self.git.commit_history_limit {
                                ui.add_space(4.0);
                                if ui
                                    .button(
                                        egui::RichText::new("Load More…")
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
                });
                self.git.show_log = header_resp.fully_open();
                if let Some(inner) = header_resp.body_returned {
                    if let Some((full_hash, message, body)) = inner {
                        let short_hash = &full_hash[..7.min(full_hash.len())];
                        let diff_text = git::get_commit_diff(&self.project_root, &full_hash)
                            .unwrap_or_default();
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
                }
            });
    }

    fn render_file_entry(
        ui: &mut egui::Ui,
        entry: &FileEntry,
        expanded: &mut HashSet<PathBuf>,
        current_file: &Option<PathBuf>,
        file_to_load: &mut Option<PathBuf>,
        project_root: &Path,
        dirty_files: &HashMap<String, char>,
        semantic: &SemanticColors,
        depth: usize,
        font_size: f32,
    ) {
        let ignored_color = ui.visuals().weak_text_color();
        let indent = depth as f32 * 16.0;

        if entry.is_dir {
            let is_expanded = expanded.contains(&entry.path);
            let dir_has_dirty = Self::dir_has_dirty_files(entry, project_root, dirty_files);

            // Allocate a full-width row
            let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
            let available_width = ui.available_width();
            let (row_rect, response) = ui.allocate_exact_size(
                egui::vec2(available_width, row_height),
                egui::Sense::click(),
            );

            // Hover highlight
            if response.hovered() {
                let hover = if ui.visuals().dark_mode {
                    egui::Color32::from_white_alpha(15)
                } else {
                    egui::Color32::from_black_alpha(12)
                };
                ui.painter().rect_filled(row_rect, 0, hover);
            }

            // Disclosure triangle
            let triangle = if is_expanded { "\u{25BC}" } else { "\u{25B6}" };
            let triangle_color = ui.visuals().weak_text_color();
            let text_pos = row_rect.left_center() + egui::vec2(indent, 0.0);
            ui.painter().text(
                egui::pos2(text_pos.x + 6.0, text_pos.y),
                egui::Align2::LEFT_CENTER,
                triangle,
                egui::FontId::proportional(10.0),
                triangle_color,
            );

            // Directory name
            let name_color = if entry.is_ignored {
                ignored_color
            } else if dir_has_dirty {
                semantic.warning
            } else {
                ui.visuals().text_color()
            };
            ui.painter().text(
                egui::pos2(text_pos.x + 20.0, text_pos.y),
                egui::Align2::LEFT_CENTER,
                &entry.name,
                egui::FontId::proportional(font_size),
                name_color,
            );

            if response.clicked() {
                if is_expanded {
                    expanded.remove(&entry.path);
                } else {
                    expanded.insert(entry.path.clone());
                }
            }

            // Render children if expanded
            if is_expanded {
                for child in &entry.children {
                    Self::render_file_entry(
                        ui,
                        child,
                        expanded,
                        current_file,
                        file_to_load,
                        project_root,
                        dirty_files,
                        semantic,
                        depth + 1,
                        font_size,
                    );
                }
            }
        } else {
            let is_selected = current_file.as_ref() == Some(&entry.path);
            let rel = entry
                .path
                .strip_prefix(project_root)
                .unwrap_or(&entry.path)
                .to_string_lossy()
                .to_string();
            let status_letter = dirty_files.get(&rel).copied();

            // Allocate a full-width row
            let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
            let available_width = ui.available_width();
            let (row_rect, response) = ui.allocate_exact_size(
                egui::vec2(available_width, row_height),
                egui::Sense::click(),
            );

            // Selected highlight
            if is_selected {
                ui.painter()
                    .rect_filled(row_rect, 0, semantic.selection_bg());
            }

            // Hover highlight
            if response.hovered() && !is_selected {
                let hover = if ui.visuals().dark_mode {
                    egui::Color32::from_white_alpha(15)
                } else {
                    egui::Color32::from_black_alpha(12)
                };
                ui.painter().rect_filled(row_rect, 0, hover);
            }

            // File name (indented with extra space to align past disclosure triangles)
            let text_pos = row_rect.left_center() + egui::vec2(indent + 20.0, 0.0);
            let name_color = if entry.is_ignored {
                ignored_color
            } else if status_letter.is_some() {
                semantic.warning
            } else {
                ui.visuals().text_color()
            };
            ui.painter().text(
                text_pos,
                egui::Align2::LEFT_CENTER,
                &entry.name,
                egui::FontId::proportional(font_size),
                name_color,
            );

            // Git status badge (right-aligned)
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

            if response.clicked() {
                *file_to_load = Some(entry.path.clone());
            }
        }
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
                if let Some(ref info) = self.git.info {
                    let branch_label = ui.label(
                        icon_small(&format!("\u{25CF} {}", info.branch), self.settings.font_size),
                    );
                    branch_label.on_hover_text(format!(
                        "{} {}",
                        info.last_commit_hash, info.last_commit_message
                    ));
                    let summary = git::format_status_summary(info);
                    if !summary.is_empty() {
                        ui.separator();
                        ui.label(
                            egui::RichText::new(summary)
                                .monospace()
                                .small(),
                        );
                    }
                } else {
                    if ui.add(egui::Label::new(
                        egui::RichText::new("not a git repository — click to init")
                            .monospace()
                            .small()
                            .color(self.semantic.tertiary_text),
                    ).sense(egui::Sense::click())).clicked() {
                        self.git_init_confirm = Some(self.project_root.clone());
                    }
                }

                // Agent status indicators (format, lint, build, test)
                {
                    let has_any_status = self.settings.agents.iter().any(|a| {
                        a.enabled && self.agent_state.statuses.contains_key(&a.kind)
                    });
                    if has_any_status {
                        ui.separator();
                        for config in &self.settings.agents {
                            if !config.enabled {
                                continue;
                            }
                            let status = self
                                .agent_state
                                .statuses
                                .get(&config.kind)
                                .copied()
                                .unwrap_or(AgentStatus::Idle);
                            let (icon_str, color) = match status {
                                AgentStatus::Idle => continue,
                                AgentStatus::Running => ("\u{21BB}", self.semantic.accent),
                                AgentStatus::Passed => ("\u{2713}", self.semantic.success),
                                AgentStatus::Failed => ("\u{2717}", self.semantic.danger),
                                AgentStatus::Error => ("!", self.semantic.danger),
                            };
                            let label_text = format!("{} {}", config.display_name(), icon_str);
                            let mut resp = ui.label(
                                egui::RichText::new(&label_text)
                                    .monospace()
                                    .small()
                                    .color(color),
                            );
                            // Show output on hover
                            if let Some(output) = self.agent_state.latest_output.get(&config.kind)
                            {
                                let preview = if output.len() > 300 {
                                    format!("{}...", super::truncate_str(output, 300))
                                } else {
                                    output.clone()
                                };
                                resp = resp.on_hover_text(preview);
                            }
                            // Click to show/hide full output
                            if resp.clicked() {
                                if self.agent_state.show_output == Some(config.kind) {
                                    self.agent_state.show_output = None;
                                } else {
                                    self.agent_state.show_output = Some(config.kind);
                                }
                            }
                        }
                    }
                    // Request repaint while agents are running
                    if self
                        .agent_state
                        .statuses
                        .values()
                        .any(|s| *s == AgentStatus::Running)
                    {
                        ctx.request_repaint_after(std::time::Duration::from_millis(500));
                    }
                }

                // Show transient status message (auto-dismiss after 6s, fade during last 2s)
                // Don't auto-dismiss while async operations are still in progress
                let busy = self.git.importing_pr
                    || self.git.pushing
                    || self.git.pulling
                    || self.git.creating_pr
                    || self.git.notifying_pr;
                let expired = !busy && matches!(&self.status_message, Some((_, when)) if when.elapsed().as_secs() >= 6);
                if expired {
                    self.status_message = None;
                }
                if let Some((ref msg, ref when)) = self.status_message {
                    let elapsed = when.elapsed().as_secs_f32();
                    let alpha = if elapsed > 4.0 {
                        // Fade out over the last 2 seconds
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
                    // Keep repainting during fade
                    if elapsed > 4.0 {
                        ctx.request_repaint();
                    }
                }

            });
        });
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

                // Show attached images above the input line
                if !self.global_prompt_images.is_empty() {
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
                    ui.vertical_centered(|ui| {
                        let input_h = input_response.rect.height();
                        let btn_size = self.settings.font_size + 12.0;
                        ui.add_space((input_h - btn_size) / 2.0);
                        let send_btn = egui::Button::new(
                            icon("\u{2191}", self.settings.font_size)
                                .color(self.semantic.accent_text()),
                        )
                        .fill(self.semantic.accent)
                        .corner_radius(btn_size as u8 / 2)
                        .min_size(egui::vec2(btn_size, btn_size));
                        let btn_clicked = ui
                            .add(send_btn)
                            .on_hover_text("Create cue  (⌘Enter to run)")
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
                        if (btn_clicked || enter_submitted || cmd_enter)
                            && !self.global_prompt_input.is_empty()
                        {
                            // Strip the trailing newline that Enter inserts before we consume
                            let text = self.global_prompt_input.trim().to_string();
                            let images: Vec<String> = self
                                .global_prompt_images
                                .drain(..)
                                .map(|p| p.to_string_lossy().to_string())
                                .collect();
                            if !text.is_empty() {
                                if let Ok(id) = self.db.insert_cue(&text, "", 0, None, &images) {
                                    if cmd_enter {
                                        let _ = self.db.update_cue_status(id, CueStatus::Ready);
                                        self.claude.expand_running = true;
                                        self.reload_cues();
                                        self.trigger_claude(id);
                                    }
                                }
                            }
                            self.global_prompt_input.clear();
                            self.reload_cues();
                        }
                    });
                });
            });
    }
}
