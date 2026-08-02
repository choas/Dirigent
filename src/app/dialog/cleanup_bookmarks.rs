use eframe::egui;

use super::super::{DialogId, DirigentApp, SPACE_SM, SPACE_XS};

impl DirigentApp {
    pub(in crate::app) fn render_cleanup_bookmarks_dialog(&mut self, ctx: &egui::Context) {
        if !self.dialogs.is_open(DialogId::CleanupBookmarks) {
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
                self.render_cleanup_bookmarks_contents(ui, &mut delete_name);
            });

        if !open {
            self.close_dialog(DialogId::CleanupBookmarks);
        }

        if let Some(name) = delete_name {
            self.start_delete_suspicious_bookmark(name);
        }
    }

    fn render_cleanup_bookmarks_contents(
        &self,
        ui: &mut egui::Ui,
        delete_name: &mut Option<String>,
    ) {
        if self.git.suspicious_bookmarks.is_empty() {
            self.render_no_suspicious_bookmarks(ui);
        } else {
            self.render_suspicious_bookmarks(ui, delete_name);
        }
    }

    fn render_no_suspicious_bookmarks(&self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new("No suspicious bookmarks found.").color(self.semantic.success),
        );
        ui.add_space(SPACE_SM);
        ui.label(
            egui::RichText::new("All bookmarks look normal.")
                .small()
                .color(self.semantic.tertiary_text),
        );
    }

    fn render_suspicious_bookmarks(&self, ui: &mut egui::Ui, delete_name: &mut Option<String>) {
        let count = self.git.suspicious_bookmarks.len();
        ui.label(format!(
            "Found {} suspicious bookmark{}:",
            count,
            if count == 1 { "" } else { "s" }
        ));
        ui.add_space(SPACE_XS);

        egui::ScrollArea::vertical()
            .max_height(250.0)
            .show(ui, |ui| {
                for bookmark in &self.git.suspicious_bookmarks {
                    self.render_suspicious_bookmark_row(ui, bookmark, delete_name);
                    ui.add_space(SPACE_XS);
                }
            });
    }

    fn render_suspicious_bookmark_row(
        &self,
        ui: &mut egui::Ui,
        bookmark: &crate::jj::SuspiciousBookmark,
        delete_name: &mut Option<String>,
    ) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&bookmark.name)
                        .monospace()
                        .color(self.semantic.danger),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let enabled = !self.git.cleanup_bookmark_op.in_flight();
                    if ui
                        .add_enabled(enabled, egui::Button::new("Delete"))
                        .clicked()
                    {
                        *delete_name = Some(bookmark.name.clone());
                    }
                });
            });
            ui.label(
                egui::RichText::new(&bookmark.reason)
                    .small()
                    .color(self.semantic.secondary_text),
            );
            if let Some(ref decoded) = bookmark.decoded {
                ui.label(
                    egui::RichText::new(format!("Likely intended: \"{}\"", decoded))
                        .small()
                        .italics()
                        .color(self.semantic.tertiary_text),
                );
            }
        });
    }
}
