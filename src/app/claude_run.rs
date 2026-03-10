use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};
use std::time::Instant;

use crate::claude;
use crate::db::CueStatus;
use crate::git;

use super::notifications::send_macos_notification;
use super::tasks::TaskHandle;
use super::DirigentApp;

/// Result of a background Claude invocation.
struct ClaudeResult {
    cue_id: i64,
    exec_id: i64,
    diff: Option<String>,
    response: String,
    error: Option<String>,
}

/// A log message from a running Claude worker thread.
struct LogUpdate {
    cue_id: i64,
    text: String,
}

/// State for Claude execution and live log streaming.
pub(crate) struct ClaudeRunState {
    tx: mpsc::Sender<ClaudeResult>,
    rx: mpsc::Receiver<ClaudeResult>,
    log_tx: mpsc::Sender<LogUpdate>,
    log_rx: mpsc::Receiver<LogUpdate>,
    pub(super) running_logs: HashMap<i64, String>,
    pub(super) start_times: HashMap<i64, Instant>,
    pub(super) exec_ids: HashMap<i64, i64>,
    pub(super) show_log: Option<i64>,
    pub(super) last_log_flush: Instant,
    /// Expand the "Running" section on next frame (after user clicks Run).
    pub(super) expand_running: bool,
}

impl ClaudeRunState {
    pub(super) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let (log_tx, log_rx) = mpsc::channel();
        ClaudeRunState {
            tx,
            rx,
            log_tx,
            log_rx,
            running_logs: HashMap::new(),
            start_times: HashMap::new(),
            exec_ids: HashMap::new(),
            show_log: None,
            last_log_flush: Instant::now(),
            expand_running: false,
        }
    }
}

impl DirigentApp {
    pub(super) fn trigger_claude(&mut self, cue_id: i64) {
        let cue = match self.cues.iter().find(|c| c.id == cue_id) {
            Some(c) => c.clone(),
            None => return,
        };

        let prompt = claude::build_prompt(
            &cue.text,
            &cue.file_path,
            cue.line_number,
            cue.line_number_end,
        );

        // Insert execution record
        let exec_id = self.db.insert_execution(cue_id, &prompt).unwrap_or(0);

        // Initialize log buffer for this cue
        self.claude.running_logs.insert(cue_id, String::new());
        self.claude.start_times.insert(cue_id, Instant::now());
        self.claude.exec_ids.insert(cue_id, exec_id);

        let project_root = self.project_root.clone();
        let claude_tx = self.claude.tx.clone();
        let log_tx = self.claude.log_tx.clone();
        let model = self.settings.claude_model.clone();
        let cli_path = self.settings.claude_cli_path.clone();
        let extra_args = self.settings.claude_extra_args.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_thread = Arc::clone(&cancel);

        let join_handle = std::thread::spawn(move || {
            let on_log = |text: &str| {
                let _ = log_tx.send(LogUpdate {
                    cue_id,
                    text: text.to_string(),
                });
            };
            let result =
                match claude::invoke_claude_streaming(
                    &prompt, &project_root, &model, &cli_path, &extra_args, on_log, cancel_thread,
                ) {
                    Ok(response) => {
                        // Claude Code edits files directly via tools.
                        // Capture the actual changes via git diff on edited files.
                        let diff = if response.edited_files.is_empty() {
                            // Fallback: try parsing response text for a diff
                            claude::parse_diff_from_response(&response.stdout)
                        } else {
                            git::get_working_diff(&project_root, &response.edited_files)
                        };
                        ClaudeResult {
                            cue_id,
                            exec_id,
                            diff,
                            response: response.stdout,
                            error: None,
                        }
                    }
                    Err(e) => ClaudeResult {
                        cue_id,
                        exec_id,
                        diff: None,
                        response: String::new(),
                        error: Some(e.to_string()),
                    },
                };
            let _ = claude_tx.send(result);
        });

        self.task_handles.push(TaskHandle {
            join_handle,
            cancel,
            cue_id: Some(cue_id),
            exec_id: Some(exec_id),
        });
    }

    pub(super) fn trigger_claude_reply(&mut self, cue_id: i64, reply: &str) {
        let cue = match self.cues.iter().find(|c| c.id == cue_id) {
            Some(c) => c.clone(),
            None => return,
        };

        // Get the previous diff from the latest execution
        let previous_diff = self
            .db
            .get_latest_execution(cue_id)
            .ok()
            .flatten()
            .and_then(|e| e.diff)
            .unwrap_or_default();

        let prompt = claude::build_reply_prompt(
            &cue.text,
            &cue.file_path,
            cue.line_number,
            cue.line_number_end,
            &previous_diff,
            reply,
        );

        // Move cue to Ready (running)
        let _ = self.db.update_cue_status(cue_id, CueStatus::Ready);
        self.claude.expand_running = true;
        self.reload_cues();

        // Insert execution record
        let exec_id = self.db.insert_execution(cue_id, &prompt).unwrap_or(0);

        // Initialize log buffer for this cue
        self.claude.running_logs.insert(cue_id, String::new());
        self.claude.start_times.insert(cue_id, Instant::now());
        self.claude.exec_ids.insert(cue_id, exec_id);

        let project_root = self.project_root.clone();
        let claude_tx = self.claude.tx.clone();
        let log_tx = self.claude.log_tx.clone();
        let model = self.settings.claude_model.clone();
        let cli_path = self.settings.claude_cli_path.clone();
        let extra_args = self.settings.claude_extra_args.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_thread = Arc::clone(&cancel);

        let join_handle = std::thread::spawn(move || {
            let on_log = |text: &str| {
                let _ = log_tx.send(LogUpdate {
                    cue_id,
                    text: text.to_string(),
                });
            };
            let result =
                match claude::invoke_claude_streaming(
                    &prompt, &project_root, &model, &cli_path, &extra_args, on_log, cancel_thread,
                ) {
                    Ok(response) => {
                        let diff = if response.edited_files.is_empty() {
                            claude::parse_diff_from_response(&response.stdout)
                        } else {
                            git::get_working_diff(&project_root, &response.edited_files)
                        };
                        ClaudeResult {
                            cue_id,
                            exec_id,
                            diff,
                            response: response.stdout,
                            error: None,
                        }
                    }
                    Err(e) => ClaudeResult {
                        cue_id,
                        exec_id,
                        diff: None,
                        response: String::new(),
                        error: Some(e.to_string()),
                    },
                };
            let _ = claude_tx.send(result);
        });

        self.task_handles.push(TaskHandle {
            join_handle,
            cancel,
            cue_id: Some(cue_id),
            exec_id: Some(exec_id),
        });

        // Close diff review if open for this cue
        if self.diff_review.as_ref().map(|r| r.cue_id) == Some(cue_id) {
            self.diff_review = None;
        }
    }

    pub(super) fn process_claude_results(&mut self) {
        // Drain log channel into local buffers first
        self.drain_log_channel();

        let results: Vec<ClaudeResult> = self.claude.rx.try_iter().collect();

        for result in results {
            // Save the running log to DB before processing
            if let Some(log_text) = self.claude.running_logs.get(&result.cue_id) {
                let _ = self.db.update_execution_log(result.exec_id, log_text);
            }
            // Clean up runtime tracking (keep running_logs for viewing)
            self.claude.exec_ids.remove(&result.cue_id);
            self.claude.start_times.remove(&result.cue_id);

            if let Some(ref error) = result.error {
                let preview = self.cue_preview(result.cue_id);
                self.set_status_message(format!("Claude error for \"{}\": {}", preview, error));
                let _ = self.db.fail_execution(result.exec_id, error);
                let _ = self
                    .db
                    .update_cue_status(result.cue_id, CueStatus::Inbox);
            } else if let Some(ref diff) = result.diff {
                // Claude already edited files directly. Store the diff for review.
                let _ =
                    self.db
                        .complete_execution(result.exec_id, &result.response, Some(diff));
                let _ = self.db.update_cue_status(
                    result.cue_id,
                    CueStatus::Review,
                );
                self.notify_review_ready(result.cue_id);
                // Reload current file so user sees changes
                if let Some(ref path) = self.viewer.current_file {
                    let p = path.clone();
                    self.load_file(p);
                }
                self.reload_git_info();
            } else {
                // Claude ran but no files were changed
                let _ =
                    self.db
                        .complete_execution(result.exec_id, &result.response, None);
                let preview = self.cue_preview(result.cue_id);
                self.set_status_message(format!(
                    "Claude completed but no file changes detected for \"{}\"",
                    preview
                ));
                let _ = self
                    .db
                    .update_cue_status(result.cue_id, CueStatus::Done);
            }
            self.reload_cues();
        }
    }

    /// Drain the log channel, appending text to the per-cue log buffers.
    pub(super) fn drain_log_channel(&mut self) {
        for update in self.claude.log_rx.try_iter() {
            self.claude.running_logs
                .entry(update.cue_id)
                .or_default()
                .push_str(&update.text);
        }
    }

    /// Periodically flush local running logs to DB (for cross-instance visibility)
    /// and reload remote running logs from DB (for viewing another instance's run).
    pub(super) fn sync_running_logs(&mut self) {
        self.drain_log_channel();

        // Flush local running logs to DB
        for (&cue_id, log_text) in &self.claude.running_logs {
            if let Some(&exec_id) = self.claude.exec_ids.get(&cue_id) {
                let _ = self.db.update_execution_log(exec_id, log_text);
            }
        }

        // Reload log from DB for the currently viewed cue if it's a remote run
        if let Some(cue_id) = self.claude.show_log {
            if !self.claude.exec_ids.contains_key(&cue_id) {
                let is_running = self
                    .cues
                    .iter()
                    .any(|c| c.id == cue_id && c.status == CueStatus::Ready);
                if is_running {
                    if let Ok(Some(exec)) = self.db.get_latest_execution(cue_id) {
                        if let Some(log_text) = exec.log {
                            self.claude.running_logs.insert(cue_id, log_text);
                        }
                    }
                }
            }
        }

        self.claude.last_log_flush = Instant::now();
    }

    fn notify_review_ready(&self, cue_id: i64) {
        if self.settings.notify_sound {
            std::thread::spawn(|| {
                #[cfg(target_os = "macos")]
                {
                    use objc::runtime::{Class, Object, BOOL};
                    use objc::{msg_send, sel, sel_impl};
                    use std::ffi::CString;
                    unsafe {
                        let ns_cls = Class::get("NSString").unwrap();
                        let name_c = CString::new("Glass").unwrap();
                        let name: *mut Object = msg_send![ns_cls,
                            stringWithUTF8String: name_c.as_ptr()];
                        let cls = Class::get("NSSound").unwrap();
                        let sound: *mut Object =
                            msg_send![cls, soundNamed: name];
                        if !sound.is_null() {
                            let _: BOOL = msg_send![sound, play];
                        }
                    }
                }
            });
        }
        if self.settings.notify_popup {
            let preview = self.cue_preview(cue_id);
            let project_name = self
                .project_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            send_macos_notification("Dirigent", &project_name, &preview);
        }
    }
}
