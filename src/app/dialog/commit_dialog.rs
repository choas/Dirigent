use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};

use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM, SPACE_XS};

impl DirigentApp {
    pub(in crate::app) fn render_commit_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_commit_dialog {
            return;
        }

        // The suggestion result is drained every frame in
        // `process_commit_suggestion` so a backgrounded commit still completes
        // after the dialog has been closed.

        let mut dismiss = false;
        let mut do_commit = false;
        let mut generate = false;
        let mut background = false;

        let fs = self.settings.font_size;

        egui::Window::new("Commit")
            .collapsible(false)
            .resizable(false)
            .default_width(450.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                if self.git.commit_review_cue_id.is_some() {
                    ui.label("Commit the reviewed changes with a message.");
                } else if !self.git.commit_files.is_empty() {
                    let n = self.git.commit_files.len();
                    ui.label(format!(
                        "Commit {n} selected file{} with a message.",
                        if n == 1 { "" } else { "s" }
                    ));
                } else if self.settings.vcs_backend == crate::settings::VcsBackend::Jj {
                    ui.label("Describe the current working-copy commit and start a new change.");
                } else {
                    ui.label("Commit all working-copy changes with a message.");
                }
                ui.add_space(SPACE_XS);

                if let Some(ref info) = self.git.info {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Branch:")
                                .small()
                                .color(self.semantic.tertiary_text),
                        );
                        ui.label(
                            egui::RichText::new(&info.branch)
                                .small()
                                .strong()
                                .color(self.semantic.secondary_text),
                        );
                    });
                    let summary = crate::git::format_status_summary(info);
                    if !summary.is_empty() {
                        ui.label(
                            egui::RichText::new(summary)
                                .small()
                                .color(self.semantic.tertiary_text),
                        );
                    }
                    ui.add_space(SPACE_XS);
                }

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Commit message").strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.git.commit_suggesting {
                            ui.spinner();
                            ui.label(
                                egui::RichText::new("Generating\u{2026}")
                                    .small()
                                    .color(self.semantic.tertiary_text),
                            );
                        } else {
                            // The button is always available: it drafts a message
                            // from the diff using the Fast LLM when configured, and
                            // otherwise falls back to the coding-agent CLI.
                            let hover = if self.settings.fast_llm_enabled {
                                "Draft a commit message from the diff using the Fast LLM"
                            } else {
                                "Draft a commit message from the diff using the CLI"
                            };
                            if ui
                                .small_button(icon("\u{2728} Summarize", fs))
                                .on_hover_text(hover)
                                .clicked()
                            {
                                generate = true;
                            }
                        }
                    });
                });
                let response = ui.add(
                    egui::TextEdit::multiline(&mut self.git.commit_message_input)
                        .desired_width(f32::INFINITY)
                        .desired_rows(3)
                        .hint_text("Describe what changed"),
                );

                if self.git.commit_needs_focus {
                    response.request_focus();
                    self.git.commit_needs_focus = false;
                }

                if response.has_focus()
                    && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter))
                    && !self.git.commit_message_input.trim().is_empty()
                {
                    do_commit = true;
                }

                ui.add_space(SPACE_XS);

                ui.label(
                    egui::RichText::new("Cmd+Enter to commit")
                        .small()
                        .color(self.semantic.tertiary_text),
                );

                ui.add_space(SPACE_SM);

                ui.horizontal(|ui| {
                    let can_commit = !self.git.commit_message_input.trim().is_empty();
                    let commit_btn = egui::Button::new(
                        icon("\u{2714} Commit", fs).color(self.semantic.badge_text),
                    )
                    .fill(self.semantic.accent);
                    if ui
                        .add_enabled(can_commit, commit_btn)
                        .on_hover_text("Commit working-copy changes")
                        .clicked()
                    {
                        do_commit = true;
                    }
                    if ui.button("Cancel").clicked() {
                        dismiss = true;
                    }
                    // Backgrounding only applies to direct working-copy commits,
                    // not to committing a reviewed cue's diff.
                    if self.git.commit_review_cue_id.is_none()
                        && ui
                            .button("Background")
                            .on_hover_text(
                                "Close and commit in the background once the message is ready",
                            )
                            .clicked()
                    {
                        background = true;
                    }
                });

                ui.add_space(SPACE_XS);
            });

        if do_commit {
            self.start_commit();
        } else if background {
            self.background_commit();
        } else if dismiss {
            self.cancel_commit_dialog();
        }
        if generate {
            self.spawn_commit_message_suggestion();
        }
    }

    /// Close the Commit dialog and arrange for the commit to happen in the
    /// background. If a message has already been drafted the commit starts
    /// immediately; otherwise analysis keeps running and the commit fires
    /// automatically once the message is ready (see
    /// [`process_commit_suggestion`](Self::process_commit_suggestion)).
    fn background_commit(&mut self) {
        self.git.show_commit_dialog = false;
        if !self.git.commit_message_input.trim().is_empty() {
            self.start_commit();
        } else if self.git.commit_suggesting || self.git.commit_suggest_rx.is_some() {
            self.git.commit_in_background = true;
            self.set_status_message("Analyzing changes; will commit in background\u{2026}".into());
        } else {
            // Nothing drafted and nothing running: kick off analysis first.
            self.git.commit_in_background = true;
            self.spawn_commit_message_suggestion();
            self.set_status_message("Analyzing changes; will commit in background\u{2026}".into());
        }
    }

    /// Cancel the Commit dialog, aborting any in-flight message analysis.
    fn cancel_commit_dialog(&mut self) {
        self.git
            .commit_suggest_cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.git.commit_suggesting = false;
        self.git.commit_suggest_rx = None;
        self.git.commit_in_background = false;
        self.git.commit_files.clear();
        self.git.show_commit_dialog = false;
        self.git.commit_review_cue_id = None;
    }

    /// Drain a completed commit-message suggestion. Runs every frame (not only
    /// while the dialog is open) so a backgrounded commit completes after the
    /// dialog has been closed.
    pub(in crate::app) fn process_commit_suggestion(&mut self) {
        let result = match &self.git.commit_suggest_rx {
            Some(rx) => match rx.try_recv() {
                Ok(r) => r,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.git.commit_suggesting = false;
                    self.git.commit_suggest_rx = None;
                    self.git.commit_in_background = false;
                    return;
                }
            },
            None => return,
        };
        self.git.commit_suggesting = false;
        self.git.commit_suggest_rx = None;
        match result {
            Ok(msg) => {
                self.git.commit_message_input = msg;
                self.git.commit_needs_focus = true;
                if self.git.commit_in_background {
                    self.git.commit_in_background = false;
                    self.start_commit();
                }
            }
            Err(e) => {
                self.git.commit_in_background = false;
                self.set_status_message(format!("Summarize: {e}"));
            }
        }
    }

    /// Spawn a background call that drafts a commit message from the current
    /// working-copy diff. When the Fast LLM is configured it is used; otherwise
    /// the coding-agent CLI is invoked headlessly with a summarization prompt.
    /// The result is delivered via
    /// [`GitState::commit_suggest_rx`](super::super::types::GitState) and applied
    /// to the message field on the next frame.
    pub(in crate::app) fn spawn_commit_message_suggestion(&mut self) {
        let files: Vec<String> = if self.git.commit_files.is_empty() {
            self.git.dirty_files.keys().cloned().collect()
        } else {
            self.git.commit_files.clone()
        };
        let diff = crate::app::vcs_dispatch::get_working_diff(
            &self.settings.vcs_backend,
            &self.settings.jj_cli_path,
            &self.project_root,
            &files,
        );
        let Some(diff) = diff.filter(|d| !d.trim().is_empty()) else {
            self.set_status_message("No changes to summarize.".to_string());
            return;
        };

        let (tx, rx) = mpsc::channel();
        self.git.commit_suggest_rx = Some(rx);
        self.git.commit_suggesting = true;
        // Fresh cancellation flag so a prior Cancel doesn't abort this run.
        let cancel = Arc::new(AtomicBool::new(false));
        self.git.commit_suggest_cancel = cancel.clone();
        let ctx = self.egui_ctx.clone();

        if let Some(config) = crate::fast_llm::FastLlmConfig::from_settings(&self.settings) {
            // Fast LLM path: a quick, local OpenAI-compatible completion.
            std::thread::spawn(move || {
                let result = crate::fast_llm::summarize_commit_message(&config, &diff);
                let _ = tx.send(result);
                if let Some(c) = ctx.get() {
                    c.request_repaint();
                }
            });
        } else {
            // CLI fallback: run the Claude CLI headlessly with a summary prompt.
            let project_root = self.project_root.clone();
            let model = self.settings.claude_model.clone();
            let cli_path = self.settings.claude_cli_path.clone();
            let extra_args = self.settings.claude_extra_args.clone();
            let env_vars = self.settings.claude_env_vars.clone();
            std::thread::spawn(move || {
                let result = crate::claude::summarize_commit_message_via_cli(
                    &diff,
                    &project_root,
                    &model,
                    &cli_path,
                    &extra_args,
                    &env_vars,
                    cancel,
                );
                let _ = tx.send(result);
                if let Some(c) = ctx.get() {
                    c.request_repaint();
                }
            });
        }
    }
}
