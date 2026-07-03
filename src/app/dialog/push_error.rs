use eframe::egui;

use super::super::DirigentApp;
use super::error_dialog::{create_global_error_cue, render_error_dialog, ErrorDialogSpec};

impl DirigentApp {
    pub(in crate::app) fn render_push_error_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_push_error {
            return;
        }

        let actions = render_error_dialog(
            ctx,
            self.semantic.dialog_frame(),
            ErrorDialogSpec {
                title: "Push Failed",
                intro: "Git push failed with the following error:",
                error_message: &self.git.push_error_message,
                error_max_height: 200.0,
                create_button: "Create Push Cue",
                create_tooltip: "Create a cue for Claude to push and fix any problems",
                try_again_tooltip: None,
                suggestion: None,
                suggestion_color: self.semantic.tertiary_text,
            },
        );

        if actions.create_cue {
            let cue_text = format!(
                "Git push failed. Fix the issue and push again.\n\nError:\n{}",
                self.git.push_error_message
            );
            create_global_error_cue(self, &cue_text, "Created push cue in Inbox");
            self.close_push_error_dialog();
        } else if actions.dismiss {
            self.close_push_error_dialog();
        }
    }

    fn close_push_error_dialog(&mut self) {
        self.git.show_push_error = false;
        self.git.push_error_message.clear();
    }
}
