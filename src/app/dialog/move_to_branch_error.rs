use eframe::egui;

use super::super::{DialogId, DirigentApp};
use super::error_dialog::{ErrorDialogSpec, create_global_error_cue, render_error_dialog};

impl DirigentApp {
    pub(in crate::app) fn render_move_to_branch_error_dialog(&mut self, ctx: &egui::Context) {
        if !self.dialogs.is_open(DialogId::MoveToBranchError) {
            return;
        }

        let suggestion = suggestion_for(&self.git.move_to_branch_error_message);
        let actions = render_error_dialog(
            ctx,
            self.semantic.dialog_frame(),
            ErrorDialogSpec {
                title: "Move to Branch Failed",
                intro: "Moving the commits to a new branch failed:",
                error_message: &self.git.move_to_branch_error_message,
                error_max_height: 160.0,
                create_button: "Create Cue",
                create_tooltip: "Create a cue for Claude to resolve the problem",
                try_again_tooltip: Some("Reopen the Move Commits dialog"),
                suggestion: Some(suggestion.text),
                suggestion_color: self.semantic.tertiary_text,
            },
        );

        if actions.try_again {
            self.close_move_to_branch_error_dialog();
            self.open_move_to_branch_dialog();
        } else if actions.create_cue {
            let cue_text = format!(
                "Moving commits to a new branch failed. Resolve the underlying problem so the \
                 commits can be moved.\n\nError:\n{}",
                self.git.move_to_branch_error_message
            );
            create_global_error_cue(self, &cue_text, "Created move-to-branch cue in Inbox");
            self.close_move_to_branch_error_dialog();
        } else if actions.dismiss {
            self.close_move_to_branch_error_dialog();
        }
    }

    fn close_move_to_branch_error_dialog(&mut self) {
        self.close_dialog(DialogId::MoveToBranchError);
    }
}

struct Suggestion {
    text: &'static str,
}

/// Map a move-to-branch failure message to a concrete, actionable suggestion.
fn suggestion_for(message: &str) -> Suggestion {
    let lower = message.to_lowercase();
    let text = if lower.contains("uncommitted changes") {
        "Your working tree has uncommitted changes, which would be lost when the current branch \
         is reset. Commit them (or stash them with `git stash`), then move the commits again."
    } else if lower.contains("no upstream") {
        "The current branch has no upstream (remote-tracking) branch, so there is nothing to \
         reset it back to. Push the branch first (or set its upstream), then try again."
    } else if lower.contains("already exists") {
        "A branch with that name already exists. Choose a different branch name and try again."
    } else if lower.contains("not on a branch") || lower.contains("determine head") {
        "HEAD is not on a branch (detached HEAD). Check out a branch first, then try again."
    } else {
        "Resolve the problem reported above, then try moving the commits again. You can also \
         create a cue to have Claude investigate and fix it."
    };
    Suggestion { text }
}
