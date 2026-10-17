use eframe::egui;

use super::super::{DialogId, DirigentApp};
use super::error_dialog::{ErrorDialogSpec, create_global_error_cue, render_error_dialog};

impl DirigentApp {
    pub(in crate::app) fn render_pr_error_dialog(&mut self, ctx: &egui::Context) {
        if !self.dialogs.is_open(DialogId::PrError) {
            return;
        }

        let actions = render_error_dialog(
            ctx,
            self.semantic.dialog_frame(),
            ErrorDialogSpec {
                title: "PR Creation Failed",
                intro: "Pull request creation failed with the following error:",
                error_message: &self.git.pr_error_message,
                error_max_height: 200.0,
                create_button: "Create Cue",
                create_tooltip: "Create a cue for Claude to diagnose and fix the PR creation failure",
                try_again_tooltip: None,
                suggestion: None,
                suggestion_color: self.semantic.tertiary_text,
            },
        );

        if actions.create_cue {
            let cue_text = format!(
                "PR creation failed. Diagnose and fix the issue so the PR can be created.\n\nError:\n{}",
                self.git.pr_error_message
            );
            create_global_error_cue(self, &cue_text, "Created PR error cue in Inbox");
            self.close_pr_error_dialog();
        } else if actions.dismiss {
            self.close_pr_error_dialog();
        }
    }

    fn close_pr_error_dialog(&mut self) {
        self.close_dialog(DialogId::PrError);
    }
}
