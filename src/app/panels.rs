use std::collections::HashSet;
use std::path::{Path, PathBuf};

use eframe::egui;

use super::{icon, icon_small, COMMIT_MSG_TRUNCATE_LEN, DirigentApp, DiffReview, SPACE_XS, SPACE_SM, SPACE_MD};
use crate::diff_view::{self, DiffViewMode};
use crate::file_tree::FileEntry;
use crate::git;

impl DirigentApp {
    pub(super) fn render_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("Dirigent", |ui| {
                    if ui.button("About Dirigent").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Settings...").clicked() {
                        self.dismiss_central_overlays();
                        self.show_settings = true;
                        ui.close_menu();
                    }
                });
            });
        });
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
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    if let Some(ref tex) = self.logo_texture {
                        ui.add(egui::Image::new(tex).max_size(egui::vec2(128.0, 128.0)));
                    }
                    ui.add_space(SPACE_SM);
                    ui.heading("Dirigent");
                    ui.label(format!("Version {}", env!("BUILD_VERSION")));
                    ui.add_space(SPACE_XS);
                    ui.label(
                        egui::RichText::new("A read-only code viewer where humans direct and AI performs.")
                            .weak(),
                    );
                    ui.add_space(SPACE_MD);
                });
            });
        self.show_about = open;
    }

    // Feature 4: Repo bar at top
    pub(super) fn render_repo_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("repo_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    icon_small(&format!("\u{25B6} {}", self.project_root.display()), self.settings.font_size),
                );
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
                ui.heading("Files");
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
                                );
                            }
                        }
                        file_to_load
                    }).inner;
                if let Some(path) = file_to_load {
                    self.load_file(path);
                }

                ui.separator();

                // Git Log collapsible section
                let header_text = format!("Git Log ({})", self.git.commit_history.len());
                let header_resp = egui::CollapsingHeader::new(header_text)
                    .default_open(self.git.show_log)
                    .show(ui, |ui| {
                        let mut clicked_commit: Option<(String, String, String)> = None;
                        egui::ScrollArea::vertical()
                            .id_salt("git_log_scroll")
                            .show(ui, |ui| {
                                for commit in &self.git.commit_history {
                                    let msg = if commit.message.len() > COMMIT_MSG_TRUNCATE_LEN + 3 {
                                        format!("{}...", &commit.message[..COMMIT_MSG_TRUNCATE_LEN])
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
                                            Some((commit.full_hash.clone(), commit.message.clone(), commit.body.clone()));
                                    }
                                }
                            });
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
        dirty_files: &HashSet<String>,
    ) {
        let ignored_color = ui.visuals().weak_text_color();

        if entry.is_dir {
            let is_expanded = expanded.contains(&entry.path);
            let dir_has_dirty = Self::dir_has_dirty_files(entry, project_root, dirty_files);
            let header_text = if entry.is_ignored {
                egui::RichText::new(&entry.name).color(ignored_color)
            } else if dir_has_dirty {
                egui::RichText::new(&entry.name).color(egui::Color32::from_rgb(200, 160, 50))
            } else {
                egui::RichText::new(&entry.name)
            };
            let header = egui::CollapsingHeader::new(header_text)
                .default_open(is_expanded)
                .show(ui, |ui| {
                    for child in &entry.children {
                        Self::render_file_entry(
                            ui,
                            child,
                            expanded,
                            current_file,
                            file_to_load,
                            project_root,
                            dirty_files,
                        );
                    }
                });
            if header.fully_open() {
                expanded.insert(entry.path.clone());
            } else {
                expanded.remove(&entry.path);
            }
        } else {
            let is_selected = current_file.as_ref() == Some(&entry.path);
            let rel = entry
                .path
                .strip_prefix(project_root)
                .unwrap_or(&entry.path)
                .to_string_lossy()
                .to_string();
            let is_dirty = dirty_files.contains(&rel);
            let label_text = if entry.is_ignored {
                egui::RichText::new(&entry.name).color(ignored_color)
            } else if is_dirty {
                egui::RichText::new(format!("{} \u{25CF}", entry.name))
                    .color(egui::Color32::from_rgb(200, 160, 50))
            } else {
                egui::RichText::new(&entry.name)
            };
            if ui.selectable_label(is_selected, label_text).clicked() {
                *file_to_load = Some(entry.path.clone());
            }
        }
    }

    /// Check if a directory contains any dirty files (recursively).
    fn dir_has_dirty_files(
        entry: &FileEntry,
        project_root: &Path,
        dirty_files: &HashSet<String>,
    ) -> bool {
        if !entry.is_dir {
            let rel = entry
                .path
                .strip_prefix(project_root)
                .unwrap_or(&entry.path)
                .to_string_lossy()
                .to_string();
            return dirty_files.contains(&rel);
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
                    ui.label(
                        egui::RichText::new("not a git repository")
                            .monospace()
                            .small()
                            .color(egui::Color32::from_gray(100)),
                    );
                }

                // Show transient status message (auto-dismiss after 6s, fade during last 2s)
                let expired = matches!(&self.status_message, Some((_, when)) if when.elapsed().as_secs() >= 6);
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
                    let color = egui::Color32::from_rgba_unmultiplied(
                        180, 180, 140, (alpha * 255.0) as u8,
                    );
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
        egui::TopBottomPanel::bottom("prompt_field").show(ctx, |ui| {
            // Show attached images above the input line
            if !self.global_prompt_images.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("Attached:")
                            .small()
                            .color(egui::Color32::from_rgb(100, 180, 255)),
                    );
                    let mut remove_idx = None;
                    for (i, path) in self.global_prompt_images.iter().enumerate() {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.to_string_lossy().to_string());
                        ui.label(egui::RichText::new(&name).monospace().small());
                        if ui.small_button("\u{2715}").on_hover_text("Remove").clicked() {
                            remove_idx = Some(i);
                        }
                    }
                    if let Some(i) = remove_idx {
                        self.global_prompt_images.remove(i);
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.label(
                    icon("\u{25B6}", self.settings.font_size)
                        .color(egui::Color32::from_rgb(100, 180, 255)),
                );
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
                    let images: Vec<String> = self
                        .global_prompt_images
                        .drain(..)
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();
                    let _ = self.db.insert_cue(&text, "", 0, None, &images);
                    self.global_prompt_input.clear();
                    self.reload_cues();
                }
            });
        });
    }
}
