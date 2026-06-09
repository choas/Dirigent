use eframe::egui;

use super::super::{DirigentApp, SPACE_XS};

impl DirigentApp {
    pub(in crate::app) fn render_delete_bookmark_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_delete_bookmark {
            return;
        }

        let mut open = true;
        let mut delete_name: Option<String> = None;
        let mut delete_merged = false;

        // Bookmarks fully merged into trunk, excluding the protected trunk bookmark.
        let merged_count = self
            .git
            .merged_bookmarks
            .iter()
            .filter(|b| !self.is_protected_bookmark(b))
            .count();

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

                // Bulk action: delete every bookmark already merged into trunk.
                let enabled = !self.git.deleting_bookmark && merged_count > 0;
                let label = if merged_count > 0 {
                    format!("Delete {} merged bookmark(s)", merged_count)
                } else {
                    "No merged bookmarks".to_string()
                };
                if ui
                    .add_enabled(enabled, egui::Button::new(label))
                    .on_hover_text(
                        "Delete all bookmarks whose commits are already part of \
                         trunk (protected bookmarks are kept)",
                    )
                    .clicked()
                {
                    delete_merged = true;
                }
                ui.add_space(SPACE_XS);
                ui.separator();
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
                            let is_protected = self.is_protected_bookmark(branch);
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
                                        if self.git.merged_bookmarks.contains(branch) {
                                            ui.label(
                                                egui::RichText::new("merged")
                                                    .small()
                                                    .italics()
                                                    .color(self.semantic.tertiary_text),
                                            );
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

        if delete_merged {
            self.start_delete_merged_bookmarks();
        }

        if let Some(name) = delete_name {
            self.start_delete_bookmark(&name);
        }
    }
}
