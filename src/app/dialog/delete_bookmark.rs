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
                self.render_delete_bookmark_contents(
                    ui,
                    merged_count,
                    &mut delete_merged,
                    &mut delete_name,
                );
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

    fn render_delete_bookmark_contents(
        &self,
        ui: &mut egui::Ui,
        merged_count: usize,
        delete_merged: &mut bool,
        delete_name: &mut Option<String>,
    ) {
        ui.label("Select bookmark to delete:");
        ui.add_space(SPACE_XS);
        self.render_delete_merged_button(ui, merged_count, delete_merged);
        ui.add_space(SPACE_XS);
        ui.separator();
        ui.add_space(SPACE_XS);

        if self.git.available_branches.is_empty() {
            ui.label(
                egui::RichText::new("No bookmarks available")
                    .italics()
                    .color(self.semantic.tertiary_text),
            );
        } else {
            self.render_delete_bookmark_list(ui, delete_name);
        }
    }

    fn render_delete_merged_button(
        &self,
        ui: &mut egui::Ui,
        merged_count: usize,
        delete_merged: &mut bool,
    ) {
        let enabled = !self.git.deleting_bookmark && merged_count > 0;
        let label = if merged_count > 0 {
            format!("Delete {} merged bookmark(s)", merged_count)
        } else {
            "No merged bookmarks".to_string()
        };
        if ui
            .add_enabled(enabled, egui::Button::new(label))
            .on_hover_text(
                "Delete all bookmarks whose commits are already part of trunk \
                 (protected bookmarks are kept)",
            )
            .clicked()
        {
            *delete_merged = true;
        }
    }

    fn render_delete_bookmark_list(&self, ui: &mut egui::Ui, delete_name: &mut Option<String>) {
        egui::ScrollArea::vertical()
            .max_height(250.0)
            .show(ui, |ui| {
                for branch in &self.git.available_branches {
                    self.render_delete_bookmark_row(ui, branch, delete_name);
                    ui.add_space(SPACE_XS);
                }
            });
    }

    fn render_delete_bookmark_row(
        &self,
        ui: &mut egui::Ui,
        branch: &str,
        delete_name: &mut Option<String>,
    ) {
        let is_protected = self.is_protected_bookmark(branch);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(branch).monospace());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.render_delete_bookmark_row_actions(ui, branch, is_protected, delete_name);
            });
        });
    }

    fn render_delete_bookmark_row_actions(
        &self,
        ui: &mut egui::Ui,
        branch: &str,
        is_protected: bool,
        delete_name: &mut Option<String>,
    ) {
        if is_protected {
            render_bookmark_state_label(ui, "protected", self.semantic.tertiary_text);
            return;
        }
        let enabled = !self.git.deleting_bookmark;
        if ui
            .add_enabled(enabled, egui::Button::new("Delete"))
            .clicked()
        {
            *delete_name = Some(branch.to_string());
        }
        if self
            .git
            .merged_bookmarks
            .iter()
            .any(|bookmark| bookmark == branch)
        {
            render_bookmark_state_label(ui, "merged", self.semantic.tertiary_text);
        }
    }
}

fn render_bookmark_state_label(ui: &mut egui::Ui, label: &str, color: egui::Color32) {
    ui.label(egui::RichText::new(label).small().italics().color(color));
}
