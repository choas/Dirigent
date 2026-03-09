use std::collections::HashSet;
use std::path::PathBuf;

use eframe::egui;

use super::{icon, icon_small, DirigentApp, DiffReview};
use crate::db::CueStatus;
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

        // Lazily load the logo texture
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
                    ui.label(format!("Version {}", env!("BUILD_VERSION")));
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
    pub(super) fn render_repo_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("repo_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    icon_small(&format!("\u{25B8} {}", self.project_root.display()), self.settings.font_size),
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

    pub(super) fn render_file_tree_panel(&mut self, ctx: &egui::Context) {
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
                            prompt_expanded: false,
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

    pub(super) fn render_status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(ref info) = self.git_info {
                    ui.label(
                        icon_small(&format!("\u{25CF} {}", info.branch), self.settings.font_size),
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

                // Show transient status message (auto-dismiss after 10s)
                let expired = matches!(&self.status_message, Some((_, when)) if when.elapsed().as_secs() >= 10);
                if expired {
                    self.status_message = None;
                }
                if let Some((ref msg, _)) = self.status_message {
                    ui.separator();
                    ui.label(
                        egui::RichText::new(msg.as_str())
                            .monospace()
                            .small()
                            .color(egui::Color32::from_rgb(255, 200, 60)),
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Settings gear button
                    if ui
                        .small_button(icon("\u{2699}", self.settings.font_size))
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
                                prompt_expanded: false,
                            });
                        }
                    }
                });
            });
        });
    }

    // Feature 2: Global prompt input
    pub(super) fn render_prompt_field(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("prompt_field").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    icon("\u{276F}", self.settings.font_size)
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
}
