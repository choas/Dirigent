use std::path::PathBuf;

use eframe::egui;

use super::{icon, DirigentApp};
use crate::db::CueStatus;
use crate::diff_view::{self, DiffViewMode};
use crate::git;
use crate::settings::{self, default_playbook, SourceConfig, SourceKind, ThemeChoice};

impl DirigentApp {
    pub(super) fn render_settings_panel(&mut self, ctx: &egui::Context) {
        let mut save = false;
        let mut close = false;
        let mut fetch_idx: Option<usize> = None;
        let fs = self.settings.font_size;

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Settings");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(icon("\u{2715}", fs)).on_hover_text("Close settings").clicked() {
                        close = true;
                    }
                    if ui.button("Save").clicked() {
                        save = true;
                    }
                });
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
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

                    ui.label("Claude CLI Path:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.claude_cli_path)
                            .desired_width(250.0)
                            .hint_text("claude (default: from PATH)")
                            .font(egui::TextStyle::Monospace),
                    );
                    ui.end_row();

                    ui.label("Extra Arguments:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.claude_extra_args)
                            .desired_width(250.0)
                            .hint_text("e.g. --max-turns 10")
                            .font(egui::TextStyle::Monospace),
                    );
                    ui.end_row();

                    ui.label("Default Flags:");
                    ui.label(
                        egui::RichText::new(
                            "-p <prompt> --verbose --output-format stream-json --dangerously-skip-permissions"
                        )
                        .monospace()
                        .weak(),
                    );
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

            // Sources section
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.strong("Sources");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("+ Add Source").clicked() {
                        self.settings.sources.push(SourceConfig::default());
                    }
                });
            });
            ui.add_space(4.0);

            if self.settings.sources.is_empty() {
                ui.label(
                    egui::RichText::new("No sources configured. Add a source to pull cues from GitHub Issues, Notion, MCP, or custom commands.")
                        .italics()
                        .color(egui::Color32::from_gray(120)),
                );
            }

            let mut remove_idx = None;
            let num_sources = self.settings.sources.len();

            for i in 0..num_sources {
                egui::Frame::none()
                    .inner_margin(6.0)
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(60)))
                    .rounding(4.0)
                    .show(ui, |ui| {
                        // Header: name + enabled + delete
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.settings.sources[i].name)
                                    .desired_width(150.0)
                                    .font(egui::TextStyle::Body),
                            );
                            ui.checkbox(&mut self.settings.sources[i].enabled, "Enabled");
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button(icon("\u{2715}", fs))
                                        .on_hover_text("Delete source")
                                        .clicked()
                                    {
                                        remove_idx = Some(i);
                                    }
                                },
                            );
                        });

                        egui::Grid::new(format!("source_grid_{}", i))
                            .num_columns(2)
                            .spacing([8.0, 4.0])
                            .show(ui, |ui| {
                                ui.label("Kind:");
                                egui::ComboBox::from_id_salt(format!("source_kind_{}", i))
                                    .selected_text(self.settings.sources[i].kind.display_name())
                                    .show_ui(ui, |ui| {
                                        for kind in SourceKind::all() {
                                            ui.selectable_value(
                                                &mut self.settings.sources[i].kind,
                                                kind.clone(),
                                                kind.display_name(),
                                            );
                                        }
                                    });
                                ui.end_row();

                                ui.label("Label:");
                                ui.add(
                                    egui::TextEdit::singleline(
                                        &mut self.settings.sources[i].label,
                                    )
                                    .desired_width(120.0)
                                    .hint_text("filter tag")
                                    .font(egui::TextStyle::Monospace),
                                );
                                ui.end_row();

                                match self.settings.sources[i].kind {
                                    SourceKind::GitHubIssues => {
                                        ui.label("GH Label:");
                                        ui.add(
                                            egui::TextEdit::singleline(
                                                &mut self.settings.sources[i].filter,
                                            )
                                            .desired_width(120.0)
                                            .hint_text("e.g. enhancement")
                                            .font(egui::TextStyle::Monospace),
                                        );
                                        ui.end_row();
                                    }
                                    _ => {
                                        ui.label("Command:");
                                        ui.add(
                                            egui::TextEdit::singleline(
                                                &mut self.settings.sources[i].command,
                                            )
                                            .desired_width(200.0)
                                            .hint_text("shell command outputting JSON")
                                            .font(egui::TextStyle::Monospace),
                                        );
                                        ui.end_row();
                                    }
                                }

                                ui.label("Poll interval:");
                                ui.horizontal(|ui| {
                                    let mut secs =
                                        self.settings.sources[i].poll_interval_secs as f64;
                                    ui.add(
                                        egui::DragValue::new(&mut secs)
                                            .range(0.0..=86400.0)
                                            .speed(10.0)
                                            .suffix("s"),
                                    );
                                    self.settings.sources[i].poll_interval_secs = secs as u64;
                                    ui.label(
                                        egui::RichText::new("(0 = manual only)")
                                            .small()
                                            .color(egui::Color32::from_gray(120)),
                                    );
                                });
                                ui.end_row();
                            });

                        ui.horizontal(|ui| {
                            if ui.small_button("Fetch Now").clicked() {
                                fetch_idx = Some(i);
                            }
                        });
                    });
                ui.add_space(4.0);
            }

            if let Some(idx) = remove_idx {
                self.settings.sources.remove(idx);
            }

            // Playbook section
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let arrow = if self.playbook_expanded { "\u{25BC}" } else { "\u{25B6}" };
                if ui.button(icon(&format!("{} Playbook", arrow), fs)).clicked() {
                    self.playbook_expanded = !self.playbook_expanded;
                }
                ui.label(
                    egui::RichText::new(format!("({} plays)", self.settings.playbook.len()))
                        .small()
                        .color(egui::Color32::from_gray(140)),
                );
                if self.playbook_expanded {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("+ Add Play").clicked() {
                            self.settings.playbook.push(settings::Play {
                                name: "New Play".to_string(),
                                prompt: String::new(),
                            });
                        }
                        if ui.small_button("Reset Defaults").clicked() {
                            self.settings.playbook = default_playbook();
                        }
                    });
                }
            });

            if self.playbook_expanded {
                ui.add_space(4.0);

                if self.settings.playbook.is_empty() {
                    ui.label(
                        egui::RichText::new("No plays configured. Add a play or reset to defaults.")
                            .italics()
                            .color(egui::Color32::from_gray(120)),
                    );
                }

                let mut remove_play_idx = None;
                let num_plays = self.settings.playbook.len();

                for i in 0..num_plays {
                    egui::Frame::none()
                        .inner_margin(6.0)
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(60)))
                        .rounding(4.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.settings.playbook[i].name)
                                        .desired_width(200.0)
                                        .font(egui::TextStyle::Body),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .small_button(icon("\u{2715}", fs))
                                            .on_hover_text("Delete play")
                                            .clicked()
                                        {
                                            remove_play_idx = Some(i);
                                        }
                                    },
                                );
                            });
                            ui.add(
                                egui::TextEdit::multiline(&mut self.settings.playbook[i].prompt)
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(3)
                                    .hint_text("Prompt text...")
                                    .font(egui::TextStyle::Monospace),
                            );
                        });
                    ui.add_space(4.0);
                }

                if let Some(idx) = remove_play_idx {
                    self.settings.playbook.remove(idx);
                }
            }

            ui.add_space(12.0);
            if ui.button("Save").clicked() {
                save = true;
            }
                }); // end ScrollArea
        });

        if close {
            self.show_settings = false;
        }
        if save {
            settings::save_settings(&self.project_root, &self.settings);
            self.needs_theme_apply = true;
        }
        if let Some(idx) = fetch_idx {
            self.trigger_source_fetch(idx);
        }
    }

    // Diff review rendered in the central panel (replaces code viewer)
    pub(super) fn render_diff_review_central(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut accept = false;
        let mut reject = false;
        let mut reply_send: Option<String> = None;
        let mut toggle_mode = None;
        let fs = self.settings.font_size;

        let review = self.diff_review.as_mut().unwrap();
        let cue_id = review.cue_id;
        let diff_text = review.diff.clone();
        let cue_text = review.cue_text.clone();
        let parsed = review.parsed.clone();
        let view_mode = review.view_mode;
        let read_only = review.read_only;
        let prompt_expanded = review.prompt_expanded;
        let reply_text = &mut review.reply_text;
        let collapsed_files = &mut review.collapsed_files;

        let mut toggle_prompt = false;

        egui::CentralPanel::default().show(ctx, |ui| {
            // Header bar
            let prefix = if read_only { "Commit" } else { "Cue" };
            ui.horizontal(|ui| {
                if ui.button(icon("\u{2190} Back", fs)).clicked() {
                    close = true;
                }
                ui.separator();
                ui.strong("Diff Review");
                ui.separator();
                let arrow = if prompt_expanded { "\u{25BC}" } else { "\u{25B6}" };
                if ui.button(icon(&format!("{} {}", arrow, prefix), fs))
                    .on_hover_text(if prompt_expanded { "Hide prompt" } else { "Show prompt" })
                    .clicked()
                {
                    toggle_prompt = true;
                }
                if !prompt_expanded {
                    let truncated = if cue_text.len() > 80 {
                        format!("{}...", &cue_text[..77])
                    } else {
                        cue_text.clone()
                    };
                    ui.label(
                        egui::RichText::new(truncated)
                            .color(egui::Color32::from_gray(180)),
                    );
                }
            });
            if prompt_expanded {
                ui.group(|ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("prompt_scroll")
                        .max_height(150.0)
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(&cue_text)
                                    .color(egui::Color32::from_gray(180)),
                            );
                        });
                });
            }
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
                                icon("\u{21BA} Revert", fs)
                                    .color(egui::Color32::from_rgb(220, 100, 100)),
                            )
                            .on_hover_text("Revert changes back to previous state")
                            .clicked()
                        {
                            reject = true;
                        }
                        if ui
                            .button(
                                icon("\u{2713} Commit", fs)
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
            // Reply field (only for actionable reviews)
            if !read_only {
                ui.horizontal(|ui| {
                    let te = ui.add(
                        egui::TextEdit::singleline(reply_text)
                            .desired_width(ui.available_width() - 80.0)
                            .hint_text("Reply with feedback..."),
                    );
                    let send = ui
                        .button(icon("\u{21A9} Reply", fs))
                        .on_hover_text("Send feedback to Claude for another iteration")
                        .clicked()
                        || (te.has_focus()
                            && ui.input(|i| {
                                i.key_pressed(egui::Key::Enter) && i.modifiers.command
                            }));
                    if send && !reply_text.trim().is_empty() {
                        reply_send = Some(reply_text.clone());
                    }
                });
            }
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
        if toggle_prompt {
            if let Some(ref mut review) = self.diff_review {
                review.prompt_expanded = !review.prompt_expanded;
            }
        }

        if accept {
            let commit_msg = git::generate_commit_message(&cue_text);
            match git::commit_diff(&self.project_root, &diff_text, &commit_msg) {
                Ok(hash) => {
                    let short = &hash[..7.min(hash.len())];
                    self.set_status_message(format!("Committed: {}", short));
                    let _ = self
                        .db
                        .update_cue_status(cue_id, CueStatus::Done);
                }
                Err(e) => {
                    self.set_status_message(format!("Commit failed: {}", e));
                }
            }
            self.reload_cues();
            self.reload_git_info();
            self.reload_commit_history();
            self.diff_review = None;
        } else if reject {
            let file_paths = git::parse_diff_file_paths_for_repo(&self.project_root, &diff_text);
            if let Err(e) = git::revert_files(&self.project_root, &file_paths) {
                self.set_status_message(format!("Revert failed: {}", e));
            }
            let _ = self
                .db
                .update_cue_status(cue_id, CueStatus::Inbox);
            if let Some(ref path) = self.viewer.current_file {
                let p = path.clone();
                self.load_file(p);
            }
            self.reload_cues();
            self.reload_git_info();
            self.diff_review = None;
        } else if let Some(reply) = reply_send {
            self.trigger_claude_reply(cue_id, &reply);
        } else if close {
            self.diff_review = None;
        }
    }

    // Feature 4: Repo picker window
    pub(super) fn render_repo_picker(&mut self, ctx: &egui::Context) {
        if !self.show_repo_picker {
            return;
        }

        let mut open = self.show_repo_picker;
        let mut switch_to: Option<PathBuf> = None;
        let mut error_msg: Option<String> = None;

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
                                error_msg = Some(format!("Not a git repository: {}", path.display()));
                            }
                        } else {
                            error_msg = Some(format!("Path not found: {}", path.display()));
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

        if let Some(msg) = error_msg {
            self.set_status_message(msg);
        }
        if let Some(new_root) = switch_to {
            self.show_repo_picker = false;
            self.switch_repo(new_root);
        }
    }

    // Feature 5: Worktree panel
    pub(super) fn render_worktree_panel(&mut self, ctx: &egui::Context) {
        if !self.git.show_worktree_panel {
            return;
        }

        let mut open = self.git.show_worktree_panel;
        let mut switch_to: Option<PathBuf> = None;
        let mut remove_path: Option<PathBuf> = None;
        let mut create_name: Option<String> = None;
        let fs = self.settings.font_size;

        egui::Window::new("Git Worktrees")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([400.0, 300.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for wt in &self.git.worktrees {
                            ui.horizontal(|ui| {
                                let label = if wt.is_current {
                                    format!("\u{25B6} {} (current)", wt.name)
                                } else if wt.is_locked {
                                    format!("\u{25A0} {}", wt.name)
                                } else {
                                    wt.name.clone()
                                };
                                ui.label(icon(&label, fs).strong());
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

                        if self.git.worktrees.is_empty() {
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
                        egui::TextEdit::singleline(&mut self.git.new_worktree_name)
                            .desired_width(200.0)
                            .hint_text("branch-name")
                            .font(egui::TextStyle::Monospace),
                    );
                    if ui.button("Create").clicked() && !self.git.new_worktree_name.is_empty() {
                        create_name = Some(self.git.new_worktree_name.clone());
                    }
                });
            });

        self.git.show_worktree_panel = open;

        if let Some(path) = switch_to {
            self.git.show_worktree_panel = false;
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
                    self.set_status_message(format!("Failed to remove worktree: {}", e));
                }
            }
        }

        if let Some(name) = create_name {
            match git::create_worktree(&self.project_root, &name) {
                Ok(_path) => {
                    self.git.new_worktree_name.clear();
                    self.reload_worktrees();
                }
                Err(e) => {
                    self.set_status_message(format!("Failed to create worktree: {}", e));
                }
            }
        }
    }

    // Claude progress rendered in the central panel (replaces code viewer)
    pub(super) fn render_running_log_central(&mut self, ctx: &egui::Context) {
        let cue_id = self.claude.show_log.unwrap();
        let fs = self.settings.font_size;

        // Drain any pending log updates before rendering
        self.drain_log_channel();

        let log_text = self
            .claude.running_logs
            .get(&cue_id)
            .cloned()
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
                if ui.button(icon("\u{2190} Back", fs)).clicked() {
                    close = true;
                }
                ui.separator();
                ui.strong("Claude Progress");
                ui.separator();
                if is_running {
                    let elapsed = self.format_elapsed(cue_id);
                    let status = if elapsed.is_empty() {
                        "\u{2022} Running".to_string()
                    } else {
                        format!("\u{2022} Running ({})", elapsed)
                    };
                    ui.label(
                        icon(&status, fs)
                            .color(egui::Color32::from_rgb(100, 180, 255)),
                    );
                    ui.ctx().request_repaint_after(super::ELAPSED_REPAINT);
                } else {
                    ui.label(
                        icon("\u{2713} Completed", fs)
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
                        let msg = if is_running {
                            "Waiting for output..."
                        } else {
                            "No output recorded."
                        };
                        ui.label(
                            egui::RichText::new(msg)
                                .italics()
                                .color(egui::Color32::from_gray(120)),
                        );
                    } else {
                        ui.label(egui::RichText::new(&log_text).monospace().small());
                    }
                });
        });

        if close {
            self.claude.show_log = None;
        }
    }
}
