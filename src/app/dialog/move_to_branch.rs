use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM, SPACE_XS};

impl DirigentApp {
    pub(in crate::app) fn render_move_to_branch_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_move_to_branch {
            return;
        }

        let mut dismiss = false;
        let mut do_move = false;

        let fs = self.settings.font_size;

        egui::Window::new("Move Commits to New Branch")
            .collapsible(false)
            .resizable(false)
            .default_width(400.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let branch = self
                    .git
                    .info
                    .as_ref()
                    .map(|i| i.branch.as_str())
                    .unwrap_or("unknown");
                let ahead = self.git.ahead_of_remote;

                ui.label(format!(
                    "Move {} commit{} from '{}' to a new branch.",
                    ahead,
                    if ahead == 1 { "" } else { "s" },
                    branch,
                ));
                ui.label(
                    egui::RichText::new(format!(
                        "'{}' will be reset to origin/{}.",
                        branch, branch
                    ))
                    .small()
                    .color(self.semantic.tertiary_text),
                );
                ui.add_space(SPACE_SM);

                // Branch name input
                ui.label(egui::RichText::new("New branch name").strong());
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.git.move_to_branch_name)
                        .desired_width(f32::INFINITY)
                        .hint_text("e.g. feature/my-changes"),
                );

                // Auto-focus the text field
                if response.gained_focus() || !response.has_focus() {
                    response.request_focus();
                }

                // Enter to confirm
                if response.has_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !self.git.move_to_branch_name.trim().is_empty()
                {
                    do_move = true;
                }

                ui.add_space(SPACE_SM);

                ui.horizontal(|ui| {
                    let can_move = !self.git.move_to_branch_name.trim().is_empty()
                        && !self.git.moving_to_branch;
                    let move_btn = egui::Button::new(
                        icon("\u{2192} Move Commits", fs).color(self.semantic.badge_text),
                    )
                    .fill(self.semantic.accent);
                    if ui
                        .add_enabled(can_move, move_btn)
                        .on_hover_text(
                            "Create new branch with these commits and reset the current branch",
                        )
                        .clicked()
                    {
                        do_move = true;
                    }
                    if ui.button("Cancel").clicked() {
                        dismiss = true;
                    }
                });

                ui.add_space(SPACE_XS);
            });

        if do_move {
            self.start_move_to_branch();
        } else if dismiss {
            self.git.show_move_to_branch = false;
        }
    }
}
