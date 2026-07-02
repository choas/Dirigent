//! Folder "Compare to…": diff a worktree folder against another chosen folder,
//! shown read-only in the side-by-side diff view (or handed to an external tool).

use std::path::PathBuf;
use std::sync::mpsc;

use crate::diff_view::DiffViewMode;
use crate::git::FolderCompare;

use super::DirigentApp;

/// `(label, comparison)` on success, or an error message.
pub(super) type FolderCompareMsg = Result<(String, FolderCompare), String>;

impl DirigentApp {
    /// Compare `left` against a folder the user picks. Uses the configured
    /// external diff tool when set, otherwise computes the diff off-thread and
    /// opens it read-only in the side-by-side view.
    pub(in crate::app) fn start_folder_compare(&mut self, left: PathBuf) {
        let Some(right) = rfd::FileDialog::new()
            .set_title("Compare against folder")
            .pick_folder()
        else {
            return; // cancelled — leave the current view untouched
        };

        // Optional external diff tool: launch it and return (non-blocking).
        let tool = self.settings.external_diff_tool.trim().to_string();
        if !tool.is_empty() {
            match std::process::Command::new(&tool).arg(&left).arg(&right).spawn() {
                Ok(_) => {
                    self.set_status_message(format!("Opened comparison in {tool}"));
                    return;
                }
                Err(e) => {
                    // Fall back to the built-in view on launch failure.
                    self.set_status_message(format!(
                        "{tool} failed ({e}); using the built-in view"
                    ));
                }
            }
        }

        self.git.show_worktree_panel = false;
        let label = format!("{} \u{2194} {}", left.display(), right.display());
        let (tx, rx) = mpsc::channel();
        self.folder_compare_rx = Some(rx);
        let ctx = self.egui_ctx.clone();
        self.set_status_message("Comparing folders\u{2026}".into());
        std::thread::spawn(move || {
            let result = crate::git::compare_folders(&left, &right)
                .map(|c| (label, c))
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
            if let Some(c) = ctx.get() {
                c.request_repaint();
            }
        });
    }

    /// Poll for a completed folder comparison.
    pub(in crate::app) fn process_folder_compare_result(&mut self) {
        let rx = match self.folder_compare_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let msg = match rx.try_recv() {
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.folder_compare_rx = None;
                return;
            }
            Ok(m) => m,
        };
        self.folder_compare_rx = None;
        match msg {
            Ok((label, FolderCompare::Diff(diff))) => {
                self.open_readonly_diff(&diff, label, DiffViewMode::SideBySide);
            }
            Ok((_, FolderCompare::Identical)) => {
                self.set_status_message("No differences between the folders".into());
            }
            Err(e) => self.set_status_message(format!("Compare failed: {e}")),
        }
    }
}
