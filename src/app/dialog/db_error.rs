//! Recovery dialog for a database that could not be opened at startup.
//!
//! `Database::open` used to be `.expect()`-ed in `DirigentApp::new`, so a
//! locked, corrupt or permission-denied `.Dirigent/Dirigent.db` killed the
//! process before any window existed. The app now falls back to an in-memory
//! database and shows this dialog, which offers the same three recoveries the
//! user would otherwise have to perform by hand in a terminal.

use std::path::{Path, PathBuf};

use chrono::Local;
use eframe::egui;

use super::super::{DbOpenError, DialogId, DirigentApp};
use crate::db::Database;

/// What the user chose in the recovery dialog.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DbRecovery {
    /// Retry `Database::open` — the usual fix after quitting the other
    /// Dirigent instance that held the lock.
    Retry,
    /// Rename the unreadable file aside and start from an empty database.
    Recreate,
    /// Open the repo picker to point Dirigent somewhere else.
    Relocate,
    /// Keep working against the in-memory database. Nothing is persisted.
    ContinueUnsaved,
}

impl DirigentApp {
    pub(in crate::app) fn render_db_error_dialog(&mut self, ctx: &egui::Context) {
        let Some(err) = self.db_open_error.clone() else {
            return;
        };

        let db_path = database_path(&self.project_root);
        let mut choice: Option<DbRecovery> = None;

        egui::Window::new("Cannot Open Database")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(self.semantic.dialog_frame())
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                render_body(ui, &db_path, &err, self.semantic.danger, &mut choice);
            });

        if let Some(choice) = choice {
            self.apply_db_recovery(choice);
        }
    }

    fn apply_db_recovery(&mut self, choice: DbRecovery) {
        match choice {
            DbRecovery::Retry => self.retry_open_database(),
            DbRecovery::Recreate => self.recreate_database(),
            DbRecovery::Relocate => {
                // Leave `db_open_error` set: if the user cancels the picker we
                // are still running on the in-memory fallback and must keep
                // saying so. `switch_repo` clears it on success.
                self.dialogs.open(DialogId::RepoPicker);
            }
            DbRecovery::ContinueUnsaved => {
                self.db_open_error = None;
                self.set_status_message(
                    "Continuing without a database — cues and run history will not be saved."
                        .to_string(),
                );
            }
        }
    }

    /// Re-attempt the on-disk open, keeping the in-memory fallback on failure.
    fn retry_open_database(&mut self) {
        match Database::open(&self.project_root) {
            Ok(db) => {
                self.db = db;
                self.db_open_error = None;
                self.reload_cues();
                self.set_status_message("Database opened.".to_string());
            }
            Err(e) => {
                let message = format!("{e:#}");
                log::error!("Database reopen failed: {message}");
                self.db_open_error = Some(DbOpenError { message });
            }
        }
    }

    /// Move the unreadable database aside and start from an empty one.
    ///
    /// The old file is renamed, never deleted — a corrupt SQLite file is often
    /// still partially recoverable with `sqlite3 .recover`, and silently
    /// destroying someone's cue history is not a recovery.
    fn recreate_database(&mut self) {
        let db_path = database_path(&self.project_root);
        let stamp = Local::now().format("%Y%m%d-%H%M%S");
        let backup = db_path.with_extension(format!("db.bak-{stamp}"));

        if db_path.exists() {
            if let Err(e) = std::fs::rename(&db_path, &backup) {
                let message = format!(
                    "Could not move {} aside: {e}",
                    db_path.file_name().unwrap_or_default().to_string_lossy()
                );
                log::error!("{message}");
                self.db_open_error = Some(DbOpenError { message });
                return;
            }
            // WAL/SHM siblings belong to the old database; leaving them behind
            // makes SQLite reject or misread the fresh file.
            for suffix in ["db-wal", "db-shm"] {
                let sidecar = db_path.with_extension(suffix);
                if sidecar.exists() {
                    let _ = std::fs::rename(
                        &sidecar,
                        backup.with_extension(format!("{suffix}-{stamp}")),
                    );
                }
            }
        }

        match Database::open(&self.project_root) {
            Ok(db) => {
                self.db = db;
                self.db_open_error = None;
                self.reload_cues();
                self.set_status_message(format!(
                    "Started a fresh database. The previous file was kept as {}",
                    backup.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
            Err(e) => {
                let message = format!("{e:#}");
                log::error!("Recreating the database failed: {message}");
                self.db_open_error = Some(DbOpenError { message });
            }
        }
    }
}

fn database_path(project_root: &Path) -> PathBuf {
    project_root.join(".Dirigent").join("Dirigent.db")
}

fn render_body(
    ui: &mut egui::Ui,
    db_path: &Path,
    err: &DbOpenError,
    error_color: egui::Color32,
    choice: &mut Option<DbRecovery>,
) {
    ui.set_max_width(560.0);
    ui.label("Dirigent could not open its database:");
    ui.add_space(4.0);
    ui.label(egui::RichText::new(db_path.display().to_string()).monospace());
    ui.add_space(8.0);
    ui.label(egui::RichText::new(&err.message).color(error_color).small());
    ui.add_space(10.0);
    ui.label(
        egui::RichText::new(
            "Until this is resolved Dirigent is running on a temporary in-memory \
             database: everything works, but nothing is saved.",
        )
        .small(),
    );
    ui.add_space(12.0);

    ui.horizontal(|ui| {
        if ui
            .button("Try Again")
            .on_hover_text("Retry after closing another Dirigent instance or fixing permissions")
            .clicked()
        {
            *choice = Some(DbRecovery::Retry);
        }
        if ui
            .button("Open Another Repo…")
            .on_hover_text("Point Dirigent at a different project")
            .clicked()
        {
            *choice = Some(DbRecovery::Relocate);
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui
            .button("Move Aside and Recreate")
            .on_hover_text(
                "Rename the existing file to Dirigent.db.bak-<timestamp> and start empty",
            )
            .clicked()
        {
            *choice = Some(DbRecovery::Recreate);
        }
        if ui
            .button("Continue Without Saving")
            .on_hover_text("Dismiss this dialog and keep using the in-memory database")
            .clicked()
        {
            *choice = Some(DbRecovery::ContinueUnsaved);
        }
    });
}
