use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::db::CueStatus;

use super::DirigentApp;

/// Handle to a background task with cancellation support.
pub(super) struct TaskHandle {
    pub(super) join_handle: JoinHandle<()>,
    pub(super) cancel: Arc<AtomicBool>,
    /// Cue ID if this is a Claude execution task (None for source fetches).
    pub(super) cue_id: Option<i64>,
    /// Execution DB row ID for cleanup on panic.
    pub(super) exec_id: Option<i64>,
}

impl DirigentApp {
    /// Reap finished tasks: join completed threads, surface panics, clean up
    /// orphaned execution records.
    pub(super) fn reap_tasks(&mut self) {
        let mut i = 0;
        while i < self.task_handles.len() {
            if self.task_handles[i].join_handle.is_finished() {
                let handle = self.task_handles.swap_remove(i);
                match handle.join_handle.join() {
                    Ok(()) => {
                        // Normal completion — result already sent via channel.
                    }
                    Err(_panic) => {
                        // Thread panicked — mark orphaned execution as failed.
                        if let Some(exec_id) = handle.exec_id {
                            let _ = self.db.fail_execution(exec_id, "worker thread panicked");
                        }
                        if let Some(cue_id) = handle.cue_id {
                            let _ = self.db.update_cue_status(cue_id, CueStatus::Inbox);
                            self.claude.running_logs.remove(&cue_id);
                            self.claude.start_times.remove(&cue_id);
                            self.claude.exec_ids.remove(&cue_id);
                            let preview = self.cue_preview(cue_id);
                            self.set_status_message(format!(
                                "Worker thread panicked for \"{}\"",
                                preview
                            ));
                        }
                        self.reload_cues();
                    }
                }
            } else {
                i += 1;
            }
        }
    }

    /// Signal cancellation for a specific cue's running task.
    pub(super) fn cancel_cue_task(&mut self, cue_id: i64) {
        for handle in &self.task_handles {
            if handle.cue_id == Some(cue_id) {
                handle.cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Signal cancellation for all running tasks.
    pub(super) fn cancel_all_tasks(&mut self) {
        for handle in &self.task_handles {
            handle.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Cancel all tasks and block until every worker thread has exited.
    pub(super) fn shutdown_tasks(&mut self) {
        self.cancel_all_tasks();
        for handle in self.task_handles.drain(..) {
            let _ = handle.join_handle.join();
        }
    }
}
