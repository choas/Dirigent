//! Single source of truth for open dialogs.
//!
//! Replaces the scattered per-dialog `show_*` booleans: every dialog is
//! identified by a [`DialogId`] and its open state lives in one
//! [`DialogRegistry`] stack. Rendering checks [`DialogRegistry::is_open`],
//! modal detection derives from [`DialogRegistry::any_modal`], and
//! backdrop/Esc dismissal pops [`DialogRegistry::top_modal`] — so a newly
//! added dialog can no longer drift out of the modal or dismissal paths.

/// Every dialog the app can show. Adding a variant automatically enrolls the
/// dialog in modal detection and topmost-dismissal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogId {
    // App-level dialogs
    RepoPicker,
    About,
    Settings,
    LogViewer,
    AnalyzeOverPicker,
    // Git / VCS dialogs
    PushError,
    PrError,
    PullDiverged,
    PullUnmerged,
    MergeConflicts,
    CreatePr,
    ImportPr,
    PrFilter,
    SwitchBranch,
    MoveToBranch,
    MoveToBranchError,
    CreateBookmark,
    CleanupBookmarks,
    DeleteBookmark,
    MergeBookmark,
    Commit,
    WorktreePanel,
}

impl DialogId {
    /// Whether the dialog blocks the rest of the UI (dimmed backdrop, global
    /// shortcuts suppressed, dismissable via backdrop click / Esc).
    ///
    /// Default is modal — a new dialog has to opt *out*, so forgetting this
    /// method can no longer silently exclude a dialog from modal detection.
    pub(crate) fn is_modal(self) -> bool {
        !matches!(self, DialogId::Settings | DialogId::LogViewer)
    }
}

/// Stack of currently open dialogs, most recently opened last.
#[derive(Default)]
pub(crate) struct DialogRegistry {
    stack: Vec<DialogId>,
}

impl DialogRegistry {
    pub(crate) fn is_open(&self, id: DialogId) -> bool {
        self.stack.contains(&id)
    }

    /// Open `id`; if it is already open it moves to the top of the stack.
    pub(crate) fn open(&mut self, id: DialogId) {
        self.stack.retain(|d| *d != id);
        self.stack.push(id);
    }

    /// Close `id` if open. Prefer [`DirigentApp::close_dialog`] at call sites
    /// that must run per-dialog cleanup.
    pub(crate) fn close(&mut self, id: DialogId) {
        self.stack.retain(|d| *d != id);
    }

    /// Open or close `id` — for `egui::Window::open(&mut bool)`-style bindings.
    pub(crate) fn set_open(&mut self, id: DialogId, open: bool) {
        if open {
            if !self.is_open(id) {
                self.open(id);
            }
        } else {
            self.close(id);
        }
    }

    /// Whether any open dialog is modal.
    pub(crate) fn any_modal(&self) -> bool {
        self.stack.iter().any(|d| d.is_modal())
    }

    /// The most recently opened modal dialog, if any.
    pub(crate) fn top_modal(&self) -> Option<DialogId> {
        self.stack.iter().rev().copied().find(|d| d.is_modal())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_close_roundtrip() {
        let mut reg = DialogRegistry::default();
        assert!(!reg.is_open(DialogId::Commit));
        reg.open(DialogId::Commit);
        assert!(reg.is_open(DialogId::Commit));
        assert!(reg.any_modal());
        reg.close(DialogId::Commit);
        assert!(!reg.is_open(DialogId::Commit));
        assert!(!reg.any_modal());
    }

    #[test]
    fn top_modal_is_most_recent_and_skips_non_modal() {
        let mut reg = DialogRegistry::default();
        reg.open(DialogId::CreatePr);
        reg.open(DialogId::PrError);
        reg.open(DialogId::LogViewer); // non-modal, on top of the stack
        assert_eq!(reg.top_modal(), Some(DialogId::PrError));
        reg.close(DialogId::PrError);
        assert_eq!(reg.top_modal(), Some(DialogId::CreatePr));
    }

    #[test]
    fn reopen_moves_to_top() {
        let mut reg = DialogRegistry::default();
        reg.open(DialogId::CreatePr);
        reg.open(DialogId::Commit);
        reg.open(DialogId::CreatePr);
        assert_eq!(reg.top_modal(), Some(DialogId::CreatePr));
    }

    #[test]
    fn non_modal_dialogs_do_not_dim() {
        let mut reg = DialogRegistry::default();
        reg.open(DialogId::LogViewer);
        reg.open(DialogId::Settings);
        assert!(!reg.any_modal());
        assert_eq!(reg.top_modal(), None);
    }
}
