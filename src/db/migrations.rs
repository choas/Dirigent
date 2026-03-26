use anyhow::{Context, Result};
use chrono::Local;
use rusqlite::{params, Connection};
use std::path::Path;

/// Validate that a string is a safe SQL identifier (letters, digits, underscores).
/// Panics if the identifier contains unexpected characters, preventing SQL injection
/// if these helpers are ever called with non-hardcoded values.
fn assert_valid_ident(s: &str) {
    assert!(
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
        "invalid SQL identifier: {s:?}"
    );
}

/// Propagate all errors from a schema-migration statement except
/// "duplicate column" / "already exists", which indicate the migration
/// was already applied and are safe to ignore.
fn ok_if_duplicate<T>(result: rusqlite::Result<T>, sql: &str) -> Result<()> {
    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("duplicate column") || msg.contains("already exists") {
                Ok(())
            } else {
                Err(e).with_context(|| format!("migration failed: {sql}"))
            }
        }
    }
}

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

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

        let db = Database { conn };
        db.create_tables()?;
        Ok(db)
    }

    /// Add a column to a table if it does not already exist.
    fn add_column(&self, table: &str, column: &str, col_type: &str) -> Result<()> {
        assert_valid_ident(table);
        assert_valid_ident(column);
        let probe = format!("SELECT {column} FROM {table} LIMIT 0");
        if self.conn.prepare(&probe).is_err() {
            let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {col_type}");
            ok_if_duplicate(self.conn.execute_batch(&sql), &sql)?;
        }
        Ok(())
    }

    /// Rename a column if the old name still exists.
    fn rename_column(&self, table: &str, old_col: &str, new_col: &str) -> Result<()> {
        assert_valid_ident(table);
        assert_valid_ident(old_col);
        assert_valid_ident(new_col);
        let probe = format!("SELECT {old_col} FROM {table} LIMIT 0");
        if self.conn.prepare(&probe).is_ok() {
            let sql = format!("ALTER TABLE {table} RENAME COLUMN {old_col} TO {new_col}");
            ok_if_duplicate(self.conn.execute_batch(&sql), &sql)?;
        }
        Ok(())
    }

    /// Migrate the legacy `comments` table to `cues`.
    fn migrate_comments_to_cues(&self) -> Result<()> {
        let has_old_table: bool = self.conn.prepare("SELECT 1 FROM comments LIMIT 0").is_ok();
        if !has_old_table {
            return Ok(());
        }
        let cues_count: i64 = match self
            .conn
            .prepare("SELECT COUNT(*) FROM cues")
            .and_then(|mut s| s.query_row([], |r| r.get(0)))
        {
            Ok(n) => n,
            Err(rusqlite::Error::SqliteFailure(_, Some(ref msg)))
                if msg.contains("no such table") =>
            {
                0
            }
            Err(e) => {
                return Err(e).with_context(|| "migration: checking cues table count");
            }
        };
        if cues_count == 0 {
            self.conn.execute_batch("DROP TABLE IF EXISTS cues;")?;
            self.conn
                .execute_batch("ALTER TABLE comments RENAME TO cues;")?;
        } else {
            let comments_count: i64 =
                self.conn
                    .query_row("SELECT COUNT(*) FROM comments", [], |r| r.get(0))?;
            if comments_count == 0 {
                self.conn.execute_batch("DROP TABLE comments;")?;
            } else {
                anyhow::bail!(
                    "Migration conflict: both 'cues' ({cues_count} rows) and \
                     'comments' ({comments_count} rows) contain data. \
                     Please manually merge or remove duplicates before restarting."
                );
            }
        }
        Ok(())
    }

    pub(super) fn create_tables(&self) -> Result<()> {
        self.migrate_comments_to_cues()?;

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

        // Column migrations — cues
        self.add_column("cues", "line_number_end", "INTEGER")?;
        self.add_column("cues", "source_label", "TEXT")?;
        self.add_column("cues", "source_ref", "TEXT")?;
        self.add_column("cues", "attached_images", "TEXT")?;
        self.add_column("cues", "tag", "TEXT")?;

        // Column migrations — executions
        self.rename_column("executions", "comment_id", "cue_id")?;
        self.add_column("executions", "log", "TEXT")?;
        self.add_column("executions", "provider", "TEXT DEFAULT 'Claude'")?;
        self.add_column("executions", "cost_usd", "REAL")?;
        self.add_column("executions", "duration_ms", "INTEGER")?;
        self.add_column("executions", "num_turns", "INTEGER")?;
        self.add_column("executions", "input_tokens", "INTEGER")?;
        self.add_column("executions", "output_tokens", "INTEGER")?;

        // Additional tables
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cue_activity_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                cue_id INTEGER NOT NULL REFERENCES cues(id),
                timestamp TEXT NOT NULL,
                event TEXT NOT NULL
            );",
        )?;
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS agent_runs (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_kind    TEXT NOT NULL,
                cue_id        INTEGER REFERENCES cues(id) ON DELETE SET NULL,
                command       TEXT NOT NULL,
                status        TEXT NOT NULL,
                output        TEXT,
                diagnostics   TEXT,
                duration_ms   INTEGER,
                started_at    TEXT NOT NULL,
                finished_at   TEXT
            );",
        )?;
        // Indexes
        self.conn
            .execute_batch("CREATE INDEX IF NOT EXISTS idx_cues_status ON cues(status);")?;
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_activity_cue ON cue_activity_log(cue_id);",
        )?;
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_agent_runs_kind ON agent_runs(agent_kind);",
        )?;
        // Settings migrations tracker
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
        if let Err(e) = self.conn.execute(
            "INSERT OR IGNORE INTO settings_migrations (name, applied_at) VALUES (?1, ?2)",
            params![name, now],
        ) {
            eprintln!("Failed to record settings migration '{name}': {e}");
        }
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
