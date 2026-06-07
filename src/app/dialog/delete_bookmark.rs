use eframe::egui;

use super::super::{DirigentApp, SPACE_XS};

impl DirigentApp {
    pub(in crate::app) fn render_delete_bookmark_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_delete_bookmark {
            return;
        }

        let mut open = true;
        let mut delete_name: Option<String> = None;

        egui::Window::new("Delete Bookmark")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([350.0, 300.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.label("Select bookmark to delete:");
                ui.add_space(SPACE_XS);

                if self.git.available_branches.is_empty() {
                    ui.label(
                        egui::RichText::new("No bookmarks available")
                            .italics()
                            .color(self.semantic.tertiary_text),
                    );
                    return;
                }

                egui::ScrollArea::vertical()
                    .max_height(250.0)
                    .show(ui, |ui| {
                        for branch in &self.git.available_branches {
                            let is_protected = branch == "main" || branch == "master";
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(branch).monospace());
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if is_protected {
                                            ui.label(
                                                egui::RichText::new("protected")
                                                    .small()
                                                    .italics()
                                                    .color(self.semantic.tertiary_text),
                                            );
                                            return;
                                        }
                                        let enabled = !self.git.deleting_bookmark;
                                        if ui
                                            .add_enabled(enabled, egui::Button::new("Delete"))
                                            .clicked()
                                        {
                                            delete_name = Some(branch.clone());
                                        }
                                    },
                                );
                            });
                            ui.add_space(SPACE_XS);
                        }
                    });
            });

        if !open {
            self.git.show_delete_bookmark = false;
        }

        if let Some(name) = delete_name {
            self.start_delete_bookmark(&name);
        }
    }
}
