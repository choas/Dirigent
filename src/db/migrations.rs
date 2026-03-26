use anyhow::{Context, Result};
use chrono::Local;
use rusqlite::{params, Connection};
use std::path::Path;

pub(crate) struct Database {
    pub(super) conn: Connection,
}

impl Database {
    pub fn open(project_root: &Path) -> Result<Self> {
        let db_dir = project_root.join(".Dirigent");
        std::fs::create_dir_all(&db_dir)
            .with_context(|| format!("creating .Dirigent dir at {}", db_dir.display()))?;

        let db_path = db_dir.join("Dirigent.db");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("opening database at {}", db_path.display()))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        let db = Database { conn };
        db.create_tables()?;
        Ok(db)
    }

    pub(super) fn create_tables(&self) -> Result<()> {
        // Migration: rename comments -> cues if old table exists
        let has_old_table: bool = self.conn.prepare("SELECT 1 FROM comments LIMIT 0").is_ok();
        if has_old_table {
            let cues_count: i64 = self
                .conn
                .prepare("SELECT COUNT(*) FROM cues")
                .and_then(|mut s| s.query_row([], |r| r.get(0)))
                .unwrap_or(0); // 0 if cues table doesn't exist yet
            if cues_count == 0 {
                let _ = self.conn.execute_batch("DROP TABLE IF EXISTS cues;");
                self.conn
                    .execute_batch("ALTER TABLE comments RENAME TO cues;")?;
            } else {
                // cues table has data — drop the legacy comments table instead
                let _ = self.conn.execute_batch("DROP TABLE comments;");
            }
        }

        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cues (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                file_path TEXT NOT NULL,
                line_number INTEGER NOT NULL,
                line_number_end INTEGER,
                status TEXT NOT NULL DEFAULT 'inbox'
            );

            CREATE TABLE IF NOT EXISTS executions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                cue_id INTEGER NOT NULL REFERENCES cues(id),
                prompt TEXT NOT NULL,
                response TEXT,
                diff TEXT,
                status TEXT NOT NULL DEFAULT 'pending'
            );
            ",
        )?;
        // Migration: add line_number_end column if missing
        let has_col: bool = self
            .conn
            .prepare("SELECT line_number_end FROM cues LIMIT 0")
            .is_ok();
        if !has_col {
            let _ = self
                .conn
                .execute_batch("ALTER TABLE cues ADD COLUMN line_number_end INTEGER;");
        }
        // Migration: rename comment_id -> cue_id in executions
        let has_old_col: bool = self
            .conn
            .prepare("SELECT comment_id FROM executions LIMIT 0")
            .is_ok();
        if has_old_col {
            let _ = self
                .conn
                .execute_batch("ALTER TABLE executions RENAME COLUMN comment_id TO cue_id;");
        }
        // Migration: add log column to executions
        let has_log_col: bool = self
            .conn
            .prepare("SELECT log FROM executions LIMIT 0")
            .is_ok();
        if !has_log_col {
            let _ = self
                .conn
                .execute_batch("ALTER TABLE executions ADD COLUMN log TEXT;");
        }
        // Migration: add source_label and source_ref columns to cues
        let has_source_label: bool = self
            .conn
            .prepare("SELECT source_label FROM cues LIMIT 0")
            .is_ok();
        if !has_source_label {
            let _ = self
                .conn
                .execute_batch("ALTER TABLE cues ADD COLUMN source_label TEXT;");
        }
        let has_source_ref: bool = self
            .conn
            .prepare("SELECT source_ref FROM cues LIMIT 0")
            .is_ok();
        if !has_source_ref {
            let _ = self
                .conn
                .execute_batch("ALTER TABLE cues ADD COLUMN source_ref TEXT;");
        }
        // Migration: add attached_images column to cues
        let has_attached_images: bool = self
            .conn
            .prepare("SELECT attached_images FROM cues LIMIT 0")
            .is_ok();
        if !has_attached_images {
            let _ = self
                .conn
                .execute_batch("ALTER TABLE cues ADD COLUMN attached_images TEXT;");
        }
        // Migration: add tag column to cues
        let has_tag: bool = self.conn.prepare("SELECT tag FROM cues LIMIT 0").is_ok();
        if !has_tag {
            let _ = self
                .conn
                .execute_batch("ALTER TABLE cues ADD COLUMN tag TEXT;");
        }
        // Migration: add provider column to executions
        let has_provider_col: bool = self
            .conn
            .prepare("SELECT provider FROM executions LIMIT 0")
            .is_ok();
        if !has_provider_col {
            let _ = self
                .conn
                .execute_batch("ALTER TABLE executions ADD COLUMN provider TEXT DEFAULT 'Claude';");
        }
        // Migration: add run metrics columns to executions
        {
            let mut existing_cols: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            if let Ok(mut stmt) = self.conn.prepare("PRAGMA table_info(executions)") {
                if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) {
                    for name in rows.flatten() {
                        existing_cols.insert(name);
                    }
                }
            }
            for (col, typ) in [
                ("cost_usd", "REAL"),
                ("duration_ms", "INTEGER"),
                ("num_turns", "INTEGER"),
                ("input_tokens", "INTEGER"),
                ("output_tokens", "INTEGER"),
            ] {
                if !existing_cols.contains(col) {
                    let _ = self.conn.execute(
                        &format!("ALTER TABLE executions ADD COLUMN {col} {typ}"),
                        [],
                    );
                }
            }
        }
        // Activity log table for cue lifecycle timestamps
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cue_activity_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                cue_id INTEGER NOT NULL REFERENCES cues(id),
                timestamp TEXT NOT NULL,
                event TEXT NOT NULL
            );",
        )?;
        // Agent runs table
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS agent_runs (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_kind    TEXT NOT NULL,
                cue_id        INTEGER,
                command       TEXT NOT NULL,
                status        TEXT NOT NULL,
                output        TEXT,
                diagnostics   TEXT,
                duration_ms   INTEGER,
                started_at    TEXT NOT NULL,
                finished_at   TEXT
            );",
        )?;
        // Index on status for faster filtered queries
        let _ = self
            .conn
            .execute_batch("CREATE INDEX IF NOT EXISTS idx_cues_status ON cues(status);");
        let _ = self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_activity_cue ON cue_activity_log(cue_id);",
        );
        let _ = self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_agent_runs_kind ON agent_runs(agent_kind);",
        );
        // Settings migrations tracker – records which playbook/settings
        // migrations have already been applied so they run at most once.
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings_migrations (
                name       TEXT PRIMARY KEY,
                applied_at TEXT NOT NULL
            );",
        )?;
        Ok(())
    }

    // -- Settings migrations --

    fn has_settings_migration(&self, name: &str) -> bool {
        self.conn
            .prepare("SELECT 1 FROM settings_migrations WHERE name = ?1")
            .and_then(|mut s| s.exists(params![name]))
            .unwrap_or(false)
    }

    fn record_settings_migration(&self, name: &str) {
        let now = Local::now().to_rfc3339();
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO settings_migrations (name, applied_at) VALUES (?1, ?2)",
            params![name, now],
        );
    }

    /// Apply one-time migrations to the in-memory settings (e.g. updating
    /// default playbook prompts that changed between versions).  The caller
    /// is responsible for saving the settings back to disk afterwards.
    /// Returns `true` if any migration was applied (i.e. settings changed).
    pub(crate) fn migrate_settings(&self, settings: &mut crate::settings::Settings) -> bool {
        let mut changed = false;

        // v0.2.3 – "Create release" play now uses {VERSION} variable
        if !self.has_settings_migration("create_release_version_var") {
            let old_prefix = "Prepare a release:";
            if let Some(play) = settings
                .playbook
                .iter_mut()
                .find(|p| p.name == "Create release" && p.prompt.starts_with(old_prefix))
            {
                if let Some(new_play) = crate::settings::default_playbook()
                    .into_iter()
                    .find(|p| p.name == "Create release")
                {
                    play.prompt = new_play.prompt;
                    changed = true;
                }
            }
            self.record_settings_migration("create_release_version_var");
        }

        // v0.2.5 – "Create release" play now includes git push && git push --tags
        if !self.has_settings_migration("create_release_git_push") {
            let old_suffix = "create a git tag v{VERSION}.";
            if let Some(play) = settings
                .playbook
                .iter_mut()
                .find(|p| p.name == "Create release" && p.prompt.ends_with(old_suffix))
            {
                if let Some(new_play) = crate::settings::default_playbook()
                    .into_iter()
                    .find(|p| p.name == "Create release")
                {
                    play.prompt = new_play.prompt;
                    changed = true;
                }
            }
            self.record_settings_migration("create_release_git_push");
        }

        changed
    }
}
