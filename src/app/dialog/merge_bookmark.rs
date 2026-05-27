use eframe::egui;

use crate::app::DirigentApp;

impl DirigentApp {
    pub(in crate::app) fn render_merge_bookmark_dialog(&mut self, ctx: &egui::Context) {
        if !self.git.show_merge_bookmark {
            return;
        }

        let mut open = true;
        let mut merge_source: Option<String> = None;

        egui::Window::new("Merge Bookmark")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([350.0, 300.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                if let Some(ref info) = self.git.info {
                    ui.label(
                        egui::RichText::new(format!("Merge into: {}", info.branch))
                            .small()
                            .color(self.semantic.secondary_text),
                    );
                    ui.add_space(4.0);
                }

                ui.label("Select bookmark to merge:");
                ui.add_space(4.0);

                let current_branch = self
                    .git
                    .info
                    .as_ref()
                    .map(|i| i.branch.as_str())
                    .unwrap_or("");

                let other_branches: Vec<&String> = self
                    .git
                    .available_branches
                    .iter()
                    .filter(|b| b.as_str() != current_branch)
                    .collect();

                if other_branches.is_empty() {
                    ui.label(
                        egui::RichText::new("No other bookmarks available")
                            .italics()
                            .color(self.semantic.tertiary_text),
                    );
                    return;
                }

                egui::ScrollArea::vertical()
                    .max_height(250.0)
                    .show(ui, |ui| {
                        for branch in &other_branches {
                            if ui
                                .selectable_label(false, egui::RichText::new(*branch).monospace())
                                .clicked()
                            {
                                merge_source = Some((*branch).clone());
                            }
                        }
                    });
            });

        if !open {
            self.git.show_merge_bookmark = false;
        }

        if let Some(source) = merge_source {
            self.git.show_merge_bookmark = false;
            self.start_merge_bookmark(&source);
        }
    }
}
