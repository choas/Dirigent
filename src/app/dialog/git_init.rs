use eframe::egui;
use git2::Repository;

use super::super::DirigentApp;

impl DirigentApp {
    pub(in crate::app) fn render_git_init_dialog(&mut self, ctx: &egui::Context) {
        let Some(path) = self.git_init_confirm.clone() else {
            return;
        };

        let mut dismiss = false;
        let mut do_init = false;

        egui::Window::new("Initialize Git Repository?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .show(ctx, |ui| {
                ui.label(format!("\"{}\" is not a git repository.", path.display()));
                ui.add_space(8.0);
                ui.label("Would you like to run git init?");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Initialize").clicked() {
                        do_init = true;
                    }
                    if ui.button("Cancel").clicked() {
                        dismiss = true;
                    }
                });
            });

        if do_init {
            match Repository::init(&path) {
                Ok(_) => {
                    self.git_init_confirm = None;
                    self.set_status_message(format!(
                        "Initialized git repository at {}",
                        path.display()
                    ));
                    self.switch_repo(path);
                }
                Err(e) => {
                    self.git_init_confirm = None;
                    self.set_status_message(format!("git init failed: {}", e));
                }
            }
        } else if dismiss {
            self.git_init_confirm = None;
        }
    }
}
