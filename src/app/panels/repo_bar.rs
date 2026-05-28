use eframe::egui;

use super::super::{icon_small, vcs_dispatch, DirigentApp};

impl DirigentApp {
    // Feature 4: Repo bar at top
    pub(in super::super) fn render_repo_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("repo_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                let path_text = format!("\u{25B6} {}", self.project_root.display());
                let response = ui.add(
                    egui::Label::new(icon_small(&path_text, self.settings.font_size))
                        .sense(egui::Sense::click()),
                );
                if response.clicked() {
                    ui.output_mut(|o| {
                        o.copied_text = self.project_root.to_string_lossy().into_owned();
                    });
                    self.set_status_message("Path copied to clipboard".into());
                }
                response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Click to copy path");
                if ui.small_button("Change...").clicked() {
                    self.repo_path_input = self.project_root.to_string_lossy().to_string();
                    self.show_repo_picker = true;
                }
                if ui.small_button("Worktrees").clicked() {
                    self.reload_worktrees();
                    match vcs_dispatch::list_branches_with_status(
                        &self.settings.vcs_backend,
                        &self.settings.jj_cli_path,
                        &self.project_root,
                    ) {
                        Ok(infos) => {
                            self.git.bookmark_push_statuses = infos
                                .iter()
                                .map(|b| (b.name.clone(), b.push_status))
                                .collect();
                            self.git.available_branches =
                                infos.into_iter().map(|b| b.name).collect();
                        }
                        Err(e) => {
                            log::error!(
                                "Failed to list branches for {}: {e}",
                                self.project_root.display()
                            );
                            self.git.available_branches = Default::default();
                            self.git.bookmark_push_statuses.clear();
                        }
                    }
                    self.git.show_worktree_panel = true;
                }
            });
        });
    }
}
