use eframe::egui;

use super::super::{icon, DirigentApp, SPACE_SM, SPACE_XS};

impl DirigentApp {
    pub(in crate::app) fn render_create_bookmark_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_create_bookmark {
            return;
        }

        let mut dismiss = false;
        let mut do_create = false;

        let fs = self.settings.font_size;

        egui::Window::new("Create Bookmark")
            .collapsible(false)
            .resizable(false)
            .default_width(400.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.label("Create a new bookmark pointing at your latest commit.");
                ui.add_space(SPACE_XS);

                ui.label(
                    egui::RichText::new(
                        "The bookmark will track your work so you can push and create a PR.",
                    )
                    .small()
                    .color(self.semantic.tertiary_text),
                );
                ui.add_space(SPACE_SM);

                ui.label(egui::RichText::new("Bookmark name").strong());
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.git.create_bookmark_name)
                        .desired_width(f32::INFINITY)
                        .hint_text("e.g. my-feature"),
                );

                if self.git.create_bookmark_needs_focus {
                    response.request_focus();
                    self.git.create_bookmark_needs_focus = false;
                }

                if response.has_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !self.git.create_bookmark_name.trim().is_empty()
                {
                    do_create = true;
                }

                ui.add_space(SPACE_SM);

                ui.horizontal(|ui| {
                    let can_create = !self.git.create_bookmark_name.trim().is_empty();
                    let create_btn = egui::Button::new(
                        icon("\u{1F516} Create", fs).color(self.semantic.badge_text),
                    )
                    .fill(self.semantic.accent);
                    if ui
                        .add_enabled(can_create, create_btn)
                        .on_hover_text("Create bookmark at current commit")
                        .clicked()
                    {
                        do_create = true;
                    }
                    if ui.button("Cancel").clicked() {
                        dismiss = true;
                    }
                });

                ui.add_space(SPACE_XS);
            });

        if do_create {
            self.start_create_bookmark();
        } else if dismiss {
            self.git.show_create_bookmark = false;
        }
    }
}
