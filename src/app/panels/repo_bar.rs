use eframe::egui;

use super::super::{icon_small, DirigentApp};

impl DirigentApp {
    // Feature 4: Repo bar at top
    pub(in super::super) fn render_repo_bar(&mut self, ctx: &egui::Context) {
        egui::Panel::top("repo_bar").show_inside(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(icon_small(
                    &format!("\u{25B6} {}", self.project_root.display()),
                    self.settings.font_size,
                ));
                if ui.small_button("Change...").clicked() {
                    self.repo_path_input = self.project_root.to_string_lossy().to_string();
                    self.show_repo_picker = true;
                }
                if ui.small_button("Worktrees").clicked() {
                    self.reload_worktrees();
                    self.git.show_worktree_panel = true;
                }
            });
        });
    }
}
