//! Approval dialog for execution-capable `.Dirigent/settings.json` fields.
//!
//! Shown when a repository's settings file carries fields that can execute
//! programs or send data (LSP servers, source commands, pre/post-run scripts,
//! extra CLI args, ...) and the user has not yet approved exactly this field
//! set (remembered by content hash in the user profile). Until approved, those
//! fields stay at their defaults and `--dangerously-skip-permissions` is off.

use std::path::Path;

use eframe::egui;

use super::super::DirigentApp;

impl DirigentApp {
    pub(in crate::app) fn render_trust_prompt_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_trust else {
            return;
        };
        let fields = pending.fields.clone();
        let root = self.project_root.clone();
        let mut approve = false;
        let mut decline = false;

        egui::Window::new("Trust This Repository?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                render_trust_prompt_body(ui, &root, &fields, &mut approve, &mut decline);
            });

        if approve {
            self.approve_pending_trust();
        } else if decline {
            self.decline_pending_trust();
        }
    }

    pub(in crate::app) fn approve_pending_trust(&mut self) {
        let Some(pending) = self.pending_trust.take() else {
            return;
        };
        crate::settings::record_trust_approval(&self.project_root, &pending.hash);
        let recent_repos = self.settings.recent_repos.clone();
        let old_backend = self.settings.vcs_backend.clone();
        self.settings = pending.trusted_settings;
        self.settings.recent_repos = recent_repos;
        self.declined_trust_hash = None;
        if let Err(e) = crate::settings::save_settings(&self.project_root, &self.settings) {
            self.set_status_message(format!("Failed to save settings: {e}"));
        } else {
            self.set_status_message("Repository trusted — settings applied".to_string());
        }
        self.needs_theme_apply = true;
        if self.settings.vcs_backend != old_backend {
            self.git.history_cache_key = (String::new(), 0);
            self.reload_git_info();
            self.reload_commit_history();
            self.reload_worktrees();
        }
    }

    pub(in crate::app) fn decline_pending_trust(&mut self) {
        let Some(pending) = self.pending_trust.take() else {
            return;
        };
        self.declined_trust_hash = Some(pending.hash);
        self.set_status_message(
            "Repository left untrusted — execution-capable settings stay disabled".to_string(),
        );
    }
}

fn render_trust_prompt_body(
    ui: &mut egui::Ui,
    repo: &Path,
    fields: &[(String, String)],
    approve: &mut bool,
    decline: &mut bool,
) {
    ui.set_max_width(560.0);
    ui.label(format!(
        "\"{}\" has not been trusted on this machine yet.",
        repo.display()
    ));
    ui.add_space(6.0);
    if fields.is_empty() {
        ui.label(
            "Its .Dirigent/settings.json sets no execution-capable fields. Trusting \
             the repository lets agents run with full tool permissions \
             (--dangerously-skip-permissions); until then they run restricted.",
        );
    } else {
        ui.label(
            "Its .Dirigent/settings.json sets fields that can execute programs or \
             send data from your machine. They stay disabled until you trust the \
             repository:",
        );
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .max_height(260.0)
            .show(ui, |ui| {
                for (name, value) in fields {
                    ui.strong(name.as_str());
                    ui.add(egui::Label::new(
                        egui::RichText::new(value.as_str()).monospace().size(11.0),
                    ));
                    ui.add_space(4.0);
                }
            });
    }
    ui.add_space(10.0);
    ui.label(
        "Only trust repositories whose authors you trust — these settings run with \
         your user account's permissions.",
    );
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.button("Trust Repository").clicked() {
            *approve = true;
        }
        if ui.button("Keep Restricted").clicked() {
            *decline = true;
        }
    });
}
