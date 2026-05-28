use eframe::egui;

use super::super::{DirigentApp, SPACE_SM, SPACE_XS};

impl DirigentApp {
    pub(in crate::app) fn render_cleanup_bookmarks_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_cleanup_bookmarks {
            return;
        }

        let mut open = true;
        let mut delete_name: Option<String> = None;

        egui::Window::new("Clean Up Bookmarks")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([500.0, 300.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                if self.git.suspicious_bookmarks.is_empty() {
                    ui.label(
                        egui::RichText::new("No suspicious bookmarks found.")
                            .color(self.semantic.success),
                    );
                    ui.add_space(SPACE_SM);
                    ui.label(
                        egui::RichText::new("All bookmarks look normal.")
                            .small()
                            .color(self.semantic.tertiary_text),
                    );
                } else {
                    ui.label(format!(
                        "Found {} suspicious bookmark{}:",
                        self.git.suspicious_bookmarks.len(),
                        if self.git.suspicious_bookmarks.len() == 1 {
                            ""
                        } else {
                            "s"
                        }
                    ));
                    ui.add_space(SPACE_XS);

                    egui::ScrollArea::vertical()
                        .max_height(250.0)
                        .show(ui, |ui| {
                            for bm in &self.git.suspicious_bookmarks {
                                ui.group(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(&bm.name)
                                                .monospace()
                                                .color(self.semantic.danger),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                let enabled = !self.git.cleaning_bookmark;
                                                if ui
                                                    .add_enabled(
                                                        enabled,
                                                        egui::Button::new("Delete"),
                                                    )
                                                    .clicked()
                                                {
                                                    delete_name = Some(bm.name.clone());
                                                }
                                            },
                                        );
                                    });
                                    ui.label(
                                        egui::RichText::new(&bm.reason)
                                            .small()
                                            .color(self.semantic.secondary_text),
                                    );
                                    if let Some(ref decoded) = bm.decoded {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "Likely intended: \"{}\"",
                                                decoded
                                            ))
                                            .small()
                                            .italics()
                                            .color(self.semantic.tertiary_text),
                                        );
                                    }
                                });
                                ui.add_space(SPACE_XS);
                            }
                        });
                }
            });

        if !open {
            self.git.show_cleanup_bookmarks = false;
        }

        if let Some(name) = delete_name {
            self.start_delete_suspicious_bookmark(name);
        }
    }
}
