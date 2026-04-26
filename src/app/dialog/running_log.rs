use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM, SPACE_XS};

/// Width reserved for the send button and its surrounding padding.
const SEND_BUTTON_RESERVED_WIDTH: f32 = 44.0;
use crate::db::{CueStatus, Execution};
use crate::settings::CliProvider;

impl DirigentApp {
    // AI provider conversation rendered in the central panel (replaces code viewer)
    pub(in crate::app) fn render_running_log_central(&mut self, ui: &mut egui::Ui) {
        let Some(cue_id) = self.claude.show_log else {
            return;
        };
        let fs = self.settings.font_size;

        // Drain any pending log updates before rendering
        self.drain_log_channel();

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
                    format!("{}...", crate::app::truncate_str(&c.text, 77))
                } else {
                    c.text.clone()
                }
            })
            .unwrap_or_default();

        // Collect conversation data: past executions + current running log
        let past_execs = self.claude.conversation_history.clone();
        let (current_running_log, current_provider) = self
            .claude
            .running_logs
            .get(&cue_id)
            .cloned()
            .unwrap_or((String::new(), CliProvider::Claude));
        let current_exec_id = self.claude.exec_ids.get(&cue_id).copied();

        let mut close = false;
        let mut reply_send: Option<String> = None;

        let cue_status = self.cues.iter().find(|c| c.id == cue_id).map(|c| c.status);
        let can_reply =
            !is_running && matches!(cue_status, Some(CueStatus::Review) | Some(CueStatus::Done));

        // Reply field at the bottom – rendered as a bottom panel so it stays visible
        if can_reply {
            reply_send = self.render_reply_panel(ui, fs);
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Header bar
            close = self.render_conversation_header(ui, fs, is_running, cue_id, &cue_text);
            ui.separator();

            // Conversation scroll area
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    self.render_conversation_scroll_content(
                        ui,
                        &past_execs,
                        &current_running_log,
                        is_running,
                        current_exec_id,
                        &current_provider,
                    );
                });
        });

        if close {
            self.claude.show_log = None;
            self.claude.conversation_history.clear();
        }

        if let Some(reply) = reply_send {
            self.conversation_replies.remove(&cue_id);
            let images: Vec<String> = self
                .conversation_reply_images
                .remove(&cue_id)
                .unwrap_or_default()
                .drain(..)
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            self.trigger_claude_reply(cue_id, &reply, &images);
        }
    }

    /// Render the scroll area content with conversation history and running entry.
    fn render_conversation_scroll_content(
        &self,
        ui: &mut egui::Ui,
        past_execs: &[Execution],
        current_running_log: &str,
        is_running: bool,
        current_exec_id: Option<i64>,
        current_provider: &CliProvider,
    ) {
        if past_execs.is_empty() && current_running_log.is_empty() {
            let msg = if is_running {
                "Waiting for output..."
            } else {
                "No output recorded."
            };
            ui.label(
                egui::RichText::new(msg)
                    .italics()
                    .color(self.semantic.tertiary_text),
            );
        }

        for (idx, exec) in past_execs.iter().enumerate() {
            let is_current_running = current_exec_id == Some(exec.id);
            self.render_conversation_entry(ui, exec, idx, is_current_running, current_running_log);

            if idx < past_execs.len() - 1 {
                ui.separator();
            }
        }

        // If currently running but not yet in past_execs (just started)
        if is_running
            && current_exec_id.is_some()
            && !past_execs.iter().any(|e| Some(e.id) == current_exec_id)
        {
            self.render_current_running_entry(
                ui,
                past_execs,
                current_provider,
                current_running_log,
            );
        }
    }

    /// Render the reply input panel at the bottom of the conversation view.
    /// Returns `Some(reply_text)` if the user submitted a reply.
    fn render_reply_panel(&mut self, ui: &mut egui::Ui, fs: f32) -> Option<String> {
        let mut reply_send: Option<String> = None;

        let reply_frame = egui::Frame::NONE
            .fill(self.semantic.prompt_surface())
            .inner_margin(egui::Margin::symmetric(SPACE_SM as i8, SPACE_SM as i8));
        egui::Panel::bottom("conversation_reply_panel")
            .frame(reply_frame)
            .show_inside(ui, |ui| {
                // Top border line
                let rect = ui.available_rect_before_wrap();
                ui.painter().hline(
                    rect.x_range(),
                    rect.top(),
                    egui::Stroke::new(1.0, self.semantic.prompt_border()),
                );

                // Show attached files above the input line
                self.render_attached_files(ui);

                reply_send = self.render_reply_input_row(ui, fs);
            });

        reply_send
    }

    /// Render the horizontal row with attach button, text input, and send button.
    /// Returns `Some(reply_text)` if the user submitted a reply.
    fn render_reply_input_row(&mut self, ui: &mut egui::Ui, fs: f32) -> Option<String> {
        let mut reply_send: Option<String> = None;
        ui.horizontal(|ui| {
            ui.label(icon("\u{21A9}", fs).color(self.semantic.accent));
            if ui
                .button(icon("+", fs))
                .on_hover_text("Attach files (or drag & drop)")
                .clicked()
            {
                self.open_file_picker();
            }
            let Some(cue_id) = self.claude.show_log else {
                return;
            };
            let reply_text = self.conversation_replies.entry(cue_id).or_default();
            let line_count = reply_text.chars().filter(|c| *c == '\n').count() + 1;
            let desired_rows = line_count.clamp(1, 8);
            let input_response = ui.add(
                egui::TextEdit::multiline(reply_text)
                    .desired_width(ui.available_width() - SEND_BUTTON_RESERVED_WIDTH)
                    .desired_rows(desired_rows)
                    .hint_text("Reply with feedback...")
                    .font(egui::TextStyle::Monospace),
            );
            let submitted = Self::render_reply_send_button(ui, fs, &input_response, &self.semantic);
            if submitted && !reply_text.trim().is_empty() {
                reply_send = Some(reply_text.trim().to_string());
            }
        });
        reply_send
    }

    /// Render the send button and check for submit shortcuts.
    /// Returns `true` if the user triggered a send action.
    fn render_reply_send_button(
        ui: &mut egui::Ui,
        fs: f32,
        input_response: &egui::Response,
        semantic: &crate::app::SemanticColors,
    ) -> bool {
        let btn_clicked = Self::render_send_button(
            ui,
            fs,
            input_response,
            semantic,
            "Send feedback (\u{2318}Enter)",
        );
        let keyboard_submit = Self::is_submit_shortcut(ui, input_response.has_focus());
        btn_clicked || keyboard_submit
    }

    /// Render the list of attached files above the reply input, with remove buttons.
    fn render_attached_files(&mut self, ui: &mut egui::Ui) {
        let Some(cue_id) = self.claude.show_log else {
            return;
        };
        let has_images = self
            .conversation_reply_images
            .get(&cue_id)
            .is_some_and(|imgs| !imgs.is_empty());
        if !has_images {
            return;
        }
        let names: Vec<String> = self.conversation_reply_images[&cue_id]
            .iter()
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.to_string_lossy().to_string())
            })
            .collect();
        let mut remove_idx = None;
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new("Attached:")
                    .small()
                    .color(self.semantic.accent),
            );
            for (i, name) in names.iter().enumerate() {
                ui.label(egui::RichText::new(name).monospace().small());
                if ui
                    .small_button("\u{2715}")
                    .on_hover_text("Remove")
                    .clicked()
                {
                    remove_idx = Some(i);
                }
            }
        });
        if let Some(i) = remove_idx {
            if let Some(imgs) = self.conversation_reply_images.get_mut(&cue_id) {
                imgs.remove(i);
                if imgs.is_empty() {
                    self.conversation_reply_images.remove(&cue_id);
                }
            }
        }
        ui.add_space(SPACE_XS);
    }

    /// Render the conversation header bar with back button, status, and cue text.
    /// Returns `true` if the user clicked the back button.
    fn render_conversation_header(
        &self,
        ui: &mut egui::Ui,
        fs: f32,
        is_running: bool,
        cue_id: i64,
        cue_text: &str,
    ) -> bool {
        let mut close = false;
        ui.horizontal(|ui| {
            if ui.button(icon("\u{2190} Back", fs)).clicked() {
                close = true;
            }
            ui.separator();
            ui.strong("Conversation");
            ui.separator();
            if is_running {
                let elapsed = self.format_elapsed(cue_id);
                let status = if elapsed.is_empty() {
                    "\u{25CF} Running".to_string()
                } else {
                    format!("\u{25CF} Running ({})", elapsed)
                };
                ui.label(icon(&status, fs).color(self.semantic.accent));
                ui.ctx()
                    .request_repaint_after(super::super::ELAPSED_REPAINT);
            } else {
                ui.label(icon("\u{2713} Completed", fs).color(self.semantic.success));
            }
            ui.separator();
            ui.label(
                egui::RichText::new(cue_text)
                    .small()
                    .color(self.semantic.secondary_text),
            );
        });
        close
    }

    /// Render a single past conversation entry (user message + provider response).
    fn render_conversation_entry(
        &self,
        ui: &mut egui::Ui,
        exec: &Execution,
        idx: usize,
        is_current_running: bool,
        current_running_log: &str,
    ) {
        let user_color = self.semantic.accent;
        let exec_provider_name = exec.provider.display_name();
        let exec_provider_color = self.semantic.provider_color(&exec.provider);

        // -- User message --
        let user_text = crate::claude::extract_user_text_from_prompt(&exec.prompt);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("You").strong().color(user_color));
            if idx > 0 {
                ui.label(
                    egui::RichText::new(format!("(reply #{})", idx))
                        .small()
                        .color(self.semantic.tertiary_text),
                );
            }
        });
        Self::render_indented_frame(ui, |ui| {
            ui.label(&user_text);
        });

        // -- Provider response --
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(exec_provider_name)
                    .strong()
                    .color(exec_provider_color),
            );
        });
        Self::render_indented_frame(ui, |ui| {
            self.render_response_content(ui, is_current_running, current_running_log, &exec.log);
        });
    }

    /// Render the currently running execution that hasn't been saved to history yet.
    fn render_current_running_entry(
        &self,
        ui: &mut egui::Ui,
        past_execs: &[Execution],
        current_provider: &CliProvider,
        current_running_log: &str,
    ) {
        if !past_execs.is_empty() {
            ui.separator();
        }
        let current_provider_name = current_provider.display_name();
        let current_provider_color = self.semantic.provider_color(current_provider);
        // Show the user's prompt from running_logs context
        // (the execution hasn't been saved to history yet)
        ui.label(
            egui::RichText::new(current_provider_name)
                .strong()
                .color(current_provider_color),
        );
        Self::render_indented_frame(ui, |ui| {
            self.render_response_content(ui, true, current_running_log, &None);
        });
    }

    /// Render response content for an execution, handling running/completed/empty states.
    fn render_response_content(
        &self,
        ui: &mut egui::Ui,
        is_current_running: bool,
        current_running_log: &str,
        log: &Option<String>,
    ) {
        if is_current_running {
            // Show live streaming log for the currently running execution
            if current_running_log.is_empty() {
                ui.label(
                    egui::RichText::new("Waiting for output...")
                        .italics()
                        .color(self.semantic.tertiary_text),
                );
            } else {
                // ANSI already stripped upstream: streaming callback (claude_run.rs)
                // and build_completed_result (execution.rs) both strip before persistence.
                ui.label(egui::RichText::new(current_running_log).monospace().small());
            }
        // Completed execution log is already ANSI-stripped upstream
        // (build_completed_result in execution.rs), no further stripping needed.
        } else if let Some(ref log_text) = log {
            if !log_text.is_empty() {
                ui.label(egui::RichText::new(log_text.as_str()).monospace().small());
            } else {
                ui.label(
                    egui::RichText::new("(no output)")
                        .italics()
                        .color(self.semantic.tertiary_text),
                );
            }
        } else {
            ui.label(
                egui::RichText::new("(no output)")
                    .italics()
                    .color(self.semantic.tertiary_text),
            );
        }
    }

    /// Open a file picker dialog and add selected files to the reply attachments.
    fn open_file_picker(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("All files", &["*"])
            .pick_files()
        {
            let Some(cue_id) = self.claude.show_log else {
                return;
            };
            self.conversation_reply_images
                .entry(cue_id)
                .or_default()
                .extend(paths);
        }
    }

    /// Check if the user pressed Enter or Cmd+Enter to submit the reply.
    fn is_submit_shortcut(ui: &egui::Ui, has_focus: bool) -> bool {
        if !has_focus {
            return false;
        }
        ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift)
    }

    /// Render content inside a standard indented frame used for conversation messages.
    fn render_indented_frame(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
        egui::Frame::NONE
            .inner_margin(egui::Margin {
                left: SPACE_SM as i8,
                top: SPACE_XS as i8,
                right: SPACE_XS as i8,
                bottom: SPACE_SM as i8,
            })
            .show(ui, |ui| {
                add_contents(ui);
            });
    }
}
