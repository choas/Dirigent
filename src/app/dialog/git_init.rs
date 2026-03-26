use std::fs;
use std::path::Path;

use eframe::egui;
use git2::Repository;

use super::super::DirigentApp;

impl DirigentApp {
    /// Append `.Dirigent` to the `.gitignore` file in the given directory,
    /// creating the file if it doesn't exist.
    fn add_dirigent_to_gitignore(repo_path: &Path) {
        let gitignore = repo_path.join(".gitignore");
        let entry = ".Dirigent";

        // Check if .gitignore already contains the entry
        if let Ok(contents) = fs::read_to_string(&gitignore) {
            if contents.lines().any(|line| line.trim() == entry) {
                return;
            }
            // Append with a leading newline if file doesn't end with one
            let prefix = if contents.ends_with('\n') || contents.is_empty() {
                ""
            } else {
                "\n"
            };
            let _ = fs::write(&gitignore, format!("{contents}{prefix}{entry}\n"));
        } else {
            let _ = fs::write(&gitignore, format!("{entry}\n"));
        }
    }

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
            .order(egui::Order::Foreground)
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
                    Self::add_dirigent_to_gitignore(&path);
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
