use std::path::Path;
use std::time::Instant;

use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM, SPACE_XS};

/// Width reserved for the send button and its surrounding padding.
const SEND_BUTTON_RESERVED_WIDTH: f32 = 44.0;

/// Height of the heartbeat strip drawn below the running conversation.
const HEARTBEAT_HEIGHT: f32 = 22.0;
/// Half-width of a single line-arrival pulse, in seconds.
const HEARTBEAT_PULSE_SECS: f32 = 0.18;
/// Repaint cadence while a run is active so the heartbeat scrolls smoothly.
const HEARTBEAT_REPAINT_MS: u64 = 40;

use crate::db::{CueStatus, Execution};
use crate::settings::{CliProvider, HeartbeatStyle};

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
        let fallback_provider = past_execs
            .last()
            .map(|e| e.provider.clone())
            .unwrap_or_else(|| self.settings.cli_provider.clone());
        let (current_running_log, current_provider) = self
            .claude
            .running_logs
            .get(&cue_id)
            .cloned()
            .unwrap_or((String::new(), fallback_provider));
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

        // Heartbeat strip sits above the reply panel (when present) and at
        // the bottom of the conversation otherwise.  It only renders for the
        // local in-flight run; finished runs collapse it away.
        if is_running && self.settings.heartbeat_style != HeartbeatStyle::Off {
            self.render_heartbeat_panel(ui, cue_id);
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

    /// Render a heartbeat strip at the bottom of the running conversation
    /// view.  A short peak is drawn each time a new chunk of output arrives
    /// from the CLI provider, with a flat baseline in between so the user can
    /// see at a glance that the run is still alive.
    fn render_heartbeat_panel(&self, ui: &mut egui::Ui, cue_id: i64) {
        let window_secs = DirigentApp::HEARTBEAT_WINDOW.as_secs_f32();
        let beats: Vec<f32> = self
            .claude
            .log_heartbeats
            .get(&cue_id)
            .map(|q| {
                let now = Instant::now();
                q.iter()
                    .map(|t| now.duration_since(*t).as_secs_f32())
                    .filter(|s| *s <= window_secs)
                    .collect()
            })
            .unwrap_or_default();

        let stroke_color = self.semantic.accent;
        let baseline_color = stroke_color.gamma_multiply(0.35);
        let frame = egui::Frame::NONE
            .fill(self.semantic.prompt_surface())
            .inner_margin(egui::Margin::symmetric(SPACE_SM as i8, SPACE_XS as i8));

        egui::Panel::bottom("conversation_heartbeat_panel")
            .resizable(false)
            .frame(frame)
            .show_inside(ui, |ui| {
                let top_border = ui.available_rect_before_wrap();
                ui.painter().hline(
                    top_border.x_range(),
                    top_border.top(),
                    egui::Stroke::new(1.0, self.semantic.prompt_border()),
                );

                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), HEARTBEAT_HEIGHT),
                    egui::Sense::hover(),
                );

                let baseline_y = rect.center().y + HEARTBEAT_HEIGHT * 0.25;
                let peak_height = (baseline_y - rect.top() - 2.0).max(4.0);

                // Faint baseline so the strip is visible even when idle.
                ui.painter().hline(
                    rect.x_range(),
                    baseline_y,
                    egui::Stroke::new(1.0, baseline_color),
                );

                let style = self.settings.heartbeat_style.clone();
                match style {
                    HeartbeatStyle::Heartbeat => {
                        let n = (rect.width() as usize).clamp(32, 600);
                        let mut points = Vec::with_capacity(n);
                        for i in 0..n {
                            let frac = i as f32 / (n as f32 - 1.0);
                            let x = rect.left() + frac * rect.width();
                            let t_from_now = (1.0 - frac) * window_secs;
                            let mut deflection = 0.0_f32;
                            for &age in &beats {
                                deflection += ecg_waveform(t_from_now - age);
                            }
                            deflection = deflection.clamp(-0.3, 1.0);
                            let y = baseline_y - deflection * peak_height;
                            points.push(egui::pos2(x, y));
                        }
                        ui.painter().add(egui::Shape::line(
                            points,
                            egui::Stroke::new(1.5, stroke_color),
                        ));
                    }
                    HeartbeatStyle::Wave => {
                        let n = (rect.width() as usize).clamp(32, 600);
                        let mut points = Vec::with_capacity(n);
                        for i in 0..n {
                            let frac = i as f32 / (n as f32 - 1.0);
                            let x = rect.left() + frac * rect.width();
                            let t_from_now = (1.0 - frac) * window_secs;
                            let mut intensity = 0.0_f32;
                            for &age in &beats {
                                let dt = t_from_now - age;
                                let norm = dt / HEARTBEAT_PULSE_SECS;
                                let pulse = (-(norm * norm)).exp();
                                if pulse > intensity {
                                    intensity = pulse;
                                }
                            }
                            let y = baseline_y - intensity * peak_height;
                            points.push(egui::pos2(x, y));
                        }
                        ui.painter().add(egui::Shape::line(
                            points,
                            egui::Stroke::new(1.5, stroke_color),
                        ));
                    }
                    HeartbeatStyle::GabbaPeak => {
                        // One outlined rectangle per beat: hard edges, flat top.
                        let rect_half_width = 3.0;
                        let rect_top = baseline_y - peak_height;
                        for &age in &beats {
                            let frac = 1.0 - (age / window_secs);
                            if !(0.0..=1.0).contains(&frac) {
                                continue;
                            }
                            let x = rect.left() + frac * rect.width();
                            let bar = egui::Rect::from_min_max(
                                egui::pos2(x - rect_half_width, rect_top),
                                egui::pos2(x + rect_half_width, baseline_y),
                            );
                            ui.painter().rect_stroke(
                                bar,
                                0.0,
                                egui::Stroke::new(1.5, stroke_color),
                                egui::StrokeKind::Inside,
                            );
                        }
                    }
                    HeartbeatStyle::MorseCode => {
                        // Dot at each beat, line connecting to the next beat.
                        let dot_radius = 2.5;
                        let line_y = baseline_y - peak_height * 0.5;
                        let mut sorted: Vec<f32> = beats.clone();
                        // Oldest first → drawn left-to-right.
                        sorted
                            .sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
                        for (i, &age) in sorted.iter().enumerate() {
                            let frac = 1.0 - (age / window_secs);
                            if !(0.0..=1.0).contains(&frac) {
                                continue;
                            }
                            let x = rect.left() + frac * rect.width();
                            ui.painter().circle_filled(
                                egui::pos2(x, line_y),
                                dot_radius,
                                stroke_color,
                            );
                            let next_x = if i + 1 < sorted.len() {
                                let next_age = sorted[i + 1];
                                let nfrac = 1.0 - (next_age / window_secs);
                                if (0.0..=1.0).contains(&nfrac) {
                                    rect.left() + nfrac * rect.width()
                                } else {
                                    rect.right()
                                }
                            } else {
                                rect.right()
                            };
                            let line_start_x = x + dot_radius + 1.5;
                            let line_end_x = (next_x - dot_radius - 1.5).max(line_start_x);
                            if line_end_x > line_start_x {
                                ui.painter().line_segment(
                                    [
                                        egui::pos2(line_start_x, line_y),
                                        egui::pos2(line_end_x, line_y),
                                    ],
                                    egui::Stroke::new(1.5, stroke_color),
                                );
                            }
                        }
                    }
                    HeartbeatStyle::Off => {}
                }

                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(HEARTBEAT_REPAINT_MS));
            });
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
            let mut reply_text = self
                .conversation_replies
                .get(&cue_id)
                .cloned()
                .unwrap_or_default();
            let line_count = reply_text.chars().filter(|c| *c == '\n').count() + 1;
            let desired_rows = line_count.clamp(1, 8);
            let input_response = ui.add(
                egui::TextEdit::multiline(&mut reply_text)
                    .desired_width(ui.available_width() - SEND_BUTTON_RESERVED_WIDTH)
                    .desired_rows(desired_rows)
                    .hint_text("Reply with feedback...")
                    .font(egui::TextStyle::Monospace),
            );
            let submitted = Self::render_reply_send_button(ui, fs, &input_response, &self.semantic);
            if submitted && !reply_text.trim().is_empty() {
                reply_send = Some(reply_text.trim().to_string());
            }
            if reply_text.is_empty() {
                self.conversation_replies.remove(&cue_id);
            } else {
                self.conversation_replies.insert(cue_id, reply_text);
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
                let last_msg = self.format_last_message(cue_id);
                if !last_msg.is_empty() {
                    ui.label(
                        icon(&format!("\u{2022} last ping {}", last_msg), fs)
                            .color(self.semantic.secondary_text),
                    );
                }
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
        // Once a completed Claude run reports its session id (shown as
        // "— session <id>" at the end of the PTY log), append an external-link
        // icon to that line so the conversation can be resumed in a terminal.
        let resume_session_id = if !is_current_running && exec.provider == CliProvider::Claude {
            exec.session_id.as_deref().filter(|s| !s.is_empty())
        } else {
            None
        };
        Self::render_indented_frame(ui, |ui| {
            self.render_response_content(
                ui,
                is_current_running,
                current_running_log,
                &exec.log,
                resume_session_id,
            );
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
            self.render_response_content(ui, true, current_running_log, &None, None);
        });
    }

    /// Render response content for an execution, handling running/completed/empty states.
    ///
    /// When `resume_session_id` is `Some`, an external-link icon is appended to
    /// the log line that reports the session id, opening that conversation in a
    /// terminal via `claude --resume`.
    fn render_response_content(
        &self,
        ui: &mut egui::Ui,
        is_current_running: bool,
        current_running_log: &str,
        log: &Option<String>,
        resume_session_id: Option<&str>,
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
                self.render_ansi_log(ui, current_running_log);
            }
        } else if let Some(ref log_text) = log {
            if !log_text.is_empty() {
                match resume_session_id {
                    Some(sid) => self.render_ansi_log_with_resume(ui, log_text, sid),
                    None => self.render_ansi_log(ui, log_text),
                }
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

    /// Render log text that may contain ANSI SGR sequences from Claude Code's
    /// TUI. The Claude PTY emits `lines_ansi` with embedded color escapes; the
    /// streaming callback now passes them through unstripped so colors survive.
    /// Red/green ANSI codes are remapped to the user's diff color scheme so
    /// Claude's inline diff output matches the rest of the app.
    fn render_ansi_log(&self, ui: &mut egui::Ui, text: &str) {
        let size = ui.text_style_height(&egui::TextStyle::Small);
        let font_id = egui::FontId::new(size, egui::FontFamily::Monospace);
        let default_color = ui.visuals().text_color();
        let overrides = crate::app::ansi::DiffAnsiOverrides {
            addition_fg: Some(self.semantic.addition_text()),
            deletion_fg: Some(self.semantic.deletion_text()),
            addition_bg: Some(self.semantic.addition_bg()),
            deletion_bg: Some(self.semantic.deletion_bg()),
        };
        // Lay the log out in bounded chunks rather than as a single galley.
        // egui's `Context::fonts()` holds the context write lock for the entire
        // duration of a galley layout; a multi-megabyte streaming log laid out
        // in one shot can hold that lock long enough that a background thread
        // calling `ctx.request_repaint()` (which takes a read lock) times out
        // and trips epaint's "Failed to acquire RwLock read after 10s.
        // Deadlock?" panic. Splitting at line boundaries keeps each lock hold
        // brief; ANSI state is threaded across chunks so color runs survive.
        const MAX_CHUNK_BYTES: usize = 16 * 1024;
        if text.len() <= MAX_CHUNK_BYTES {
            let job =
                crate::app::ansi::ansi_to_layout_job(text, font_id, default_color, &overrides);
            ui.label(job);
            return;
        }

        // Stack chunks with no inter-label gap so the result is visually
        // identical to a single galley (each label's rows abut the next).
        let saved_spacing = ui.spacing().item_spacing.y;
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut state: Option<egui::TextFormat> = None;
        for range in ansi_log_chunk_ranges(text, MAX_CHUNK_BYTES) {
            let (job, next) = crate::app::ansi::ansi_to_layout_job_resumable(
                &text[range],
                font_id.clone(),
                default_color,
                &overrides,
                state.take(),
            );
            ui.label(job);
            state = Some(next);
        }
        ui.spacing_mut().item_spacing.y = saved_spacing;
    }

    /// Like [`render_ansi_log`], but appends an external-link icon to the line
    /// that reports the Claude session id. Clicking it resumes the conversation
    /// in a terminal (`claude --resume <session_id>`).
    fn render_ansi_log_with_resume(&self, ui: &mut egui::Ui, text: &str, session_id: &str) {
        let marker = format!("session {session_id}");
        let lines: Vec<&str> = text.lines().collect();
        let Some(idx) = lines.iter().position(|l| l.contains(&marker)) else {
            // Session line not present (yet); render the log unchanged.
            self.render_ansi_log(ui, text);
            return;
        };

        let before = lines[..idx].join("\n");
        if !before.is_empty() {
            self.render_ansi_log(ui, &before);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_XS;
            self.render_ansi_log(ui, lines[idx]);
            self.render_resume_icon(ui, session_id);
        });
        let after = lines[(idx + 1)..].join("\n");
        if !after.is_empty() {
            self.render_ansi_log(ui, &after);
        }
    }

    /// Render the external-link icon that opens a terminal in the project
    /// directory and runs `claude --resume <session_id>`, appending
    /// `--dangerously-skip-permissions` when the Yolo setting is enabled.
    fn render_resume_icon(&self, ui: &mut egui::Ui, session_id: &str) {
        let mut resume_cmd = format!("claude --resume {}", session_id);
        if self.settings.allow_dangerous_skip_permissions {
            resume_cmd.push_str(" --dangerously-skip-permissions");
        }
        let link = ui
            .link(
                egui::RichText::new("\u{2197}")
                    .small()
                    .color(ui.visuals().hyperlink_color),
            )
            .on_hover_text(format!(
                "Resume in Terminal:\ncd {} && {}",
                self.project_root.display(),
                resume_cmd
            ));
        if link.clicked() {
            let _ = spawn_terminal_with_command(&self.project_root, &resume_cmd);
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

/// Split `text` into byte ranges of at most ~`max_bytes`, cutting only at
/// newline boundaries. The newline at each cut point is dropped (the boundary
/// between two stacked, zero-gap labels reproduces it), while interior and
/// trailing newlines are preserved, so concatenating the rendered chunks looks
/// identical to laying the whole text out as one galley.
fn ansi_log_chunk_ranges(text: &str, max_bytes: usize) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (i, _) in text.match_indices('\n') {
        // Cut once the accumulated chunk (including this newline) would exceed
        // the budget, splitting just before the newline at `i`.
        if i + 1 - start > max_bytes && i > start {
            ranges.push(start..i);
            start = i + 1;
        }
    }
    ranges.push(start..text.len());
    ranges
}

/// ECG-style waveform: maps a time offset `dt` (seconds from beat centre) to a
/// vertical deflection in [-0.25, 1.0].  The shape approximates P–QRS–T.
fn ecg_waveform(dt: f32) -> f32 {
    let t = dt;
    // P wave – gentle bump before the QRS
    let p = 0.12 * (-(((t + 0.11) / 0.035).powi(2))).exp();
    // Q dip – small downward notch
    let q = -0.15 * (-(((t + 0.025) / 0.012).powi(2))).exp();
    // R peak – tall sharp spike
    let r = 1.0 * (-(((t) / 0.014).powi(2))).exp();
    // S dip – small downward notch after R
    let s = -0.25 * (-(((t - 0.028) / 0.012).powi(2))).exp();
    // T wave – broad recovery bump
    let tw = 0.18 * (-(((t - 0.11) / 0.045).powi(2))).exp();
    p + q + r + s + tw
}

/// Quote a string for safe interpolation into a POSIX shell command line.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Open a terminal emulator in `dir` and run `command` inside it, keeping the
/// shell open afterwards so the user can interact with the resumed session.
fn spawn_terminal_with_command(dir: &Path, command: &str) -> std::io::Result<std::process::Child> {
    if cfg!(target_os = "macos") {
        // Terminal.app can't take a working directory + command directly, so
        // build a shell command and drive it via AppleScript.
        let shell_cmd = format!("cd {} && {}", shell_quote(&dir.to_string_lossy()), command);
        // Escape for embedding inside an AppleScript double-quoted string.
        let escaped = shell_cmd.replace('\\', "\\\\").replace('"', "\\\"");
        // Run `do script` *before* `activate`. If Terminal isn't already
        // running, `activate` would launch it and create a window whose shell
        // is still initializing; the subsequent `do script` then types into
        // that not-ready shell and the first character gets dropped (e.g. "cd"
        // arrives as "d"). Letting `do script` create the window first avoids
        // the race.
        let script = format!(
            "tell application \"Terminal\"\n\
             \tdo script \"{escaped}\"\n\
             \tactivate\n\
             end tell"
        );
        std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .spawn()
    } else if cfg!(target_os = "windows") {
        // Try Windows Terminal first, fall back to a detached cmd.exe window.
        std::process::Command::new("wt")
            .arg("-d")
            .arg(dir)
            .args(["cmd", "/K", command])
            .spawn()
            .or_else(|_| {
                std::process::Command::new("cmd.exe")
                    .args(["/C", "start", "cmd.exe", "/K", command])
                    .current_dir(dir)
                    .spawn()
            })
    } else {
        // Linux: keep the shell alive after the command exits via `exec $SHELL`.
        let hold = format!("{command}; exec $SHELL");
        std::process::Command::new("gnome-terminal")
            .arg(format!("--working-directory={}", dir.display()))
            .args(["--", "bash", "-c", &hold])
            .spawn()
            .or_else(|_| {
                std::process::Command::new("konsole")
                    .arg(format!("--workdir={}", dir.display()))
                    .args(["-e", "bash", "-c", &hold])
                    .spawn()
            })
            .or_else(|_| {
                std::process::Command::new("x-terminal-emulator")
                    .current_dir(dir)
                    .args(["-e", "bash", "-c", &hold])
                    .spawn()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::ansi_log_chunk_ranges;

    /// Re-join chunk ranges the way the renderer stacks them: each non-final
    /// chunk is followed by the boundary newline that was dropped at the cut.
    fn rejoin(text: &str, max: usize) -> String {
        let ranges = ansi_log_chunk_ranges(text, max);
        // Every range must land on valid char boundaries.
        for r in &ranges {
            assert!(text.is_char_boundary(r.start) && text.is_char_boundary(r.end));
        }
        let mut out = String::new();
        for (idx, r) in ranges.iter().enumerate() {
            out.push_str(&text[r.clone()]);
            if idx + 1 < ranges.len() {
                out.push('\n');
            }
        }
        out
    }

    #[test]
    fn chunks_reconstruct_original_lines() {
        let text = "alpha\nbeta\ngamma\ndelta\n";
        // Force a split: budget smaller than the whole text.
        assert_eq!(rejoin(text, 8), text);
        // A budget large enough for the whole text yields a single range.
        assert_eq!(ansi_log_chunk_ranges(text, 4096).len(), 1);
        assert_eq!(rejoin(text, 4096), text);
    }

    #[test]
    fn chunks_handle_no_trailing_newline_and_unicode() {
        let text = "héllo\nwörld\n☃ snowman\nlast line";
        assert_eq!(rejoin(text, 4), text);
    }

    #[test]
    fn oversized_single_line_is_not_dropped() {
        // A single line longer than the budget must still survive intact.
        let line = "x".repeat(100);
        let text = format!("{line}\nshort\n");
        assert_eq!(rejoin(&text, 16), text);
    }
}
