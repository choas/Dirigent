use std::fs;
use std::path::Path;

use eframe::egui;
use git2::{Repository, RepositoryInitOptions};

use super::super::DirigentApp;

impl DirigentApp {
    const MACOS_GITIGNORE_ENTRIES: &[&str] = &[
        ".DS_Store",
        ".AppleDouble",
        ".LSOverride",
        "Icon\r\r",
        "._*",
        ".DocumentRevisions-V100",
        ".fseventsd",
        ".Spotlight-V100",
        ".TemporaryItems",
        ".Trashes",
        ".VolumeIcon.icns",
        ".com.apple.timemachine.donotpresent",
        ".AppleDB",
        ".AppleDesktop",
        "Network Trash Folder",
        "Temporary Items",
        ".apdisk",
        "*.icloud",
    ];

    fn setup_gitignore(repo_path: &Path) -> std::io::Result<()> {
        let gitignore = repo_path.join(".gitignore");
        let mut contents = fs::read_to_string(&gitignore).unwrap_or_default();
        let existing: Vec<&str> = contents.lines().map(|l| l.trim()).collect();

        let mut entries_to_add = Vec::new();
        if !existing.contains(&".Dirigent") {
            entries_to_add.push(".Dirigent");
        }

        let mut macos_missing: Vec<&str> = Self::MACOS_GITIGNORE_ENTRIES
            .iter()
            .filter(|e| !existing.contains(&e.trim()))
            .copied()
            .collect();
        if !macos_missing.is_empty() {
            entries_to_add.append(&mut macos_missing);
        }

        if entries_to_add.is_empty() {
            return Ok(());
        }

        if !contents.is_empty() && !contents.ends_with('\n') {
            contents.push('\n');
        }
        for entry in &entries_to_add {
            contents.push_str(entry);
            contents.push('\n');
        }
        fs::write(&gitignore, contents)
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
            let mut opts = RepositoryInitOptions::new();
            opts.initial_head("main");
            match Repository::init_opts(&path, &opts) {
                Ok(_) => {
                    let gitignore_err = Self::setup_gitignore(&path).err();
                    self.git_init_confirm = None;
                    if let Some(e) = gitignore_err {
                        self.set_status_message(format!(
                            "Initialized git repo but failed to update .gitignore: {}",
                            e
                        ));
                    } else {
                        self.set_status_message(format!(
                            "Initialized git repository at {}",
                            path.display()
                        ));
                    }
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
