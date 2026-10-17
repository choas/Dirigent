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

/// Validate that a column-type expression contains only safe characters.
/// Allows alphanumeric, spaces, single quotes, parentheses, underscores, and
/// minus signs (needed for e.g. `TEXT DEFAULT 'Claude'` or `INTEGER DEFAULT -1`)
/// while blocking injection vectors like semicolons, double-dashes, or slashes.
fn assert_valid_col_type(s: &str) {
    assert!(
        !s.is_empty()
            && !s.contains("--")
            && s.bytes().all(|b| b.is_ascii_alphanumeric()
                || matches!(b, b' ' | b'_' | b'\'' | b'(' | b')' | b'-')),
        "invalid SQL column type expression: {s:?}"
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

/// Schema version stamped into `PRAGMA user_version`.
///
/// The column migrations below are additive and idempotent, so they can run in
/// any order against any older database — the version stamp exists to answer
/// the one question probing cannot: *is this database newer than this binary?*
/// Bump it whenever a schema change is introduced that an older Dirigent build
/// would misinterpret rather than merely ignore (dropped/renamed columns,
/// changed semantics of an existing column, new NOT NULL constraints).
///
/// History:
///   0 — pre-versioning (every release up to and including v0.6.1)
///   1 — first stamped version; schema identical to 0, introduces the guard
pub(crate) const SCHEMA_VERSION: i32 = 1;

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
        db.check_schema_version()
            .with_context(|| format!("opening database at {}", db_path.display()))?;
        db.create_tables()?;
        db.stamp_schema_version()?;
        Ok(db)
    }

    /// Read `PRAGMA user_version`.
    fn schema_version(&self) -> Result<i32> {
        self.conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .context("reading PRAGMA user_version")
    }

    /// Refuse to open a database written by a newer Dirigent than this binary.
    ///
    /// Without this, an older build silently ignores columns it does not know
    /// about and writes rows the newer build then reads as incomplete — data
    /// corruption that surfaces much later. Failing loudly at open time is the
    /// only honest outcome; the caller turns this into a dialog.
    fn check_schema_version(&self) -> Result<()> {
        let found = self.schema_version()?;
        if found > SCHEMA_VERSION {
            anyhow::bail!(
                "This database was created by a newer version of Dirigent \
                 (schema v{found}; this build understands up to v{SCHEMA_VERSION}). \
                 Update Dirigent, or move .Dirigent/Dirigent.db aside to start fresh."
            );
        }
        Ok(())
    }

    /// Stamp the current schema version after migrations have run.
    ///
    /// `user_version` takes no bind parameters, so the value is formatted in —
    /// it is a compile-time constant `i32`, never user input.
    pub(super) fn stamp_schema_version(&self) -> Result<()> {
        if self.schema_version()? != SCHEMA_VERSION {
            self.conn
                .execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))
                .context("stamping PRAGMA user_version")?;
        }
        Ok(())
    }

    /// Add a column to a table if it does not already exist.
    /// Parameters require `&'static str` to guarantee at compile time that
    /// table/column/type names are hardcoded literals, not user input.
    fn add_column(
        &self,
        table: &'static str,
        column: &'static str,
        col_type: &'static str,
    ) -> Result<()> {
        assert_valid_ident(table);
        assert_valid_ident(column);
        assert_valid_col_type(col_type);
        if !self.has_column(table, column)? {
            let sql = format!("ALTER TABLE \"{table}\" ADD COLUMN \"{column}\" {col_type}");
            ok_if_duplicate(self.conn.execute_batch(&sql), &sql)?;
        }
        Ok(())
    }

    /// Rename a column if the old name still exists.
    fn rename_column(
        &self,
        table: &'static str,
        old_col: &'static str,
        new_col: &'static str,
    ) -> Result<()> {
        assert_valid_ident(table);
        assert_valid_ident(old_col);
        assert_valid_ident(new_col);
        if self.has_column(table, old_col)? {
            let sql =
                format!("ALTER TABLE \"{table}\" RENAME COLUMN \"{old_col}\" TO \"{new_col}\"");
            ok_if_duplicate(self.conn.execute_batch(&sql), &sql)?;
        }
        Ok(())
    }

    /// Check whether a table has a given column using PRAGMA table_info.
    /// Returns `Ok(false)` when the table does not exist (PRAGMA returns an
    /// empty result set); propagates other SQLite errors (locked / corrupted
    /// DB) so they are not silently masked.
    fn has_column(&self, table: &'static str, column: &'static str) -> Result<bool> {
        assert_valid_ident(table);
        assert_valid_ident(column);
        let sql = format!("PRAGMA table_info(\"{table}\")");
        let mut stmt = self
            .conn
            .prepare(&sql)
            .with_context(|| format!("checking column \"{column}\" on \"{table}\""))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .with_context(|| format!("checking column \"{column}\" on \"{table}\""))?;
        for name in rows {
            let name = name.with_context(|| format!("reading column info for \"{table}\""))?;
            if name == column {
                return Ok(true);
            }
        }
        Ok(false)
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
        self.add_column("cues", "source_id", "TEXT")?;
        self.add_column("cues", "source_ref", "TEXT")?;
        self.add_column("cues", "attached_images", "TEXT")?;
        self.add_column("cues", "tag", "TEXT")?;
        self.add_column("cues", "plan_path", "TEXT")?;
        self.add_column("cues", "has_question", "INTEGER DEFAULT 0")?;
        self.add_column("cues", "workflow", "INTEGER DEFAULT 0")?;
        self.add_column("cues", "auto_commit", "INTEGER DEFAULT 0")?;

        // Column migrations — executions
        self.rename_column("executions", "comment_id", "cue_id")?;
        self.add_column("executions", "log", "TEXT")?;
        self.add_column("executions", "provider", "TEXT DEFAULT 'Claude'")?;
        self.add_column("executions", "cost_usd", "REAL")?;
        self.add_column("executions", "duration_ms", "INTEGER")?;
        self.add_column("executions", "num_turns", "INTEGER")?;
        self.add_column("executions", "input_tokens", "INTEGER")?;
        self.add_column("executions", "output_tokens", "INTEGER")?;
        self.add_column("executions", "session_id", "TEXT")?;

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
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS execution_events (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                execution_id  INTEGER NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
                event_type    TEXT NOT NULL,
                payload       TEXT NOT NULL,
                created_at    TEXT NOT NULL
            );",
        )?;
        // PR filter patterns – reusable ignore rules for PR findings
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pr_filter_patterns (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                pattern    TEXT NOT NULL,
                match_field TEXT NOT NULL DEFAULT 'text',
                created_at TEXT NOT NULL
            );",
        )?;

        // Indexes
        self.conn
            .execute_batch("CREATE INDEX IF NOT EXISTS idx_cues_status ON cues(status);")?;
        self.conn
            .execute_batch("CREATE INDEX IF NOT EXISTS idx_cues_source_ref ON cues(source_ref);")?;
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_activity_cue ON cue_activity_log(cue_id);",
        )?;
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_agent_runs_kind ON agent_runs(agent_kind);",
        )?;
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_executions_cue ON executions(cue_id);",
        )?;
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_agent_runs_cue ON agent_runs(cue_id);",
        )?;
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_execution_events_execution ON execution_events(execution_id);",
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

    /// Check whether a settings migration has already been applied.
    /// Returns `Ok(false)` when the `settings_migrations` table does not
    /// exist yet; propagates other SQLite errors.
    fn has_settings_migration(&self, name: &str) -> Result<bool> {
        match self
            .conn
            .prepare("SELECT 1 FROM settings_migrations WHERE name = ?1")
            .and_then(|mut s| s.exists(params![name]))
        {
            Ok(v) => Ok(v),
            Err(rusqlite::Error::SqliteFailure(_, Some(ref msg)))
                if msg.contains("no such table") =>
            {
                Ok(false)
            }
            Err(e) => Err(e).with_context(|| format!("checking settings migration \"{name}\"")),
        }
    }

    fn record_settings_migration(&self, name: &str) {
        let now = Local::now().to_rfc3339();
        if let Err(e) = self.conn.execute(
            "INSERT OR IGNORE INTO settings_migrations (name, applied_at) VALUES (?1, ?2)",
            params![name, now],
        ) {
            log::error!("Failed to record settings migration '{name}': {e}");
        }
    }

    /// Apply one-time migrations to the in-memory settings (e.g. updating
    /// default playbook prompts that changed between versions).  The caller
    /// is responsible for saving the settings back to disk afterwards.
    /// Returns `true` if any migration was applied (i.e. settings changed).
    pub(crate) fn migrate_settings(
        &self,
        settings: &mut crate::settings::Settings,
    ) -> Result<bool> {
        let mut changed = false;

        // v0.2.3 – "Create release" play now uses {VERSION} variable
        if !self.has_settings_migration("create_release_version_var")? {
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
        if !self.has_settings_migration("create_release_git_push")? {
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

        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn col_type_allows_negative_default() {
        assert_valid_col_type("INTEGER DEFAULT -1");
    }

    #[test]
    fn col_type_allows_text_default() {
        assert_valid_col_type("TEXT DEFAULT 'Claude'");
    }

    #[test]
    fn col_type_allows_plain_type() {
        assert_valid_col_type("INTEGER");
        assert_valid_col_type("TEXT");
        assert_valid_col_type("REAL");
    }

    #[test]
    #[should_panic(expected = "invalid SQL column type expression")]
    fn col_type_rejects_semicolon() {
        assert_valid_col_type("TEXT; DROP TABLE cues");
    }

    #[test]
    #[should_panic(expected = "invalid SQL column type expression")]
    fn col_type_rejects_empty() {
        assert_valid_col_type("");
    }

    /// A fresh database gets stamped, and reopening it is a no-op.
    #[test]
    fn open_stamps_and_accepts_current_schema_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Database::open(dir.path()).expect("open fresh db");
        assert_eq!(db.schema_version().expect("read version"), SCHEMA_VERSION);
        drop(db);

        let db = Database::open(dir.path()).expect("reopen db");
        assert_eq!(db.schema_version().expect("read version"), SCHEMA_VERSION);
    }

    /// Pre-versioning databases (user_version == 0) still open and get stamped.
    #[test]
    fn open_upgrades_unversioned_database() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Database::open(dir.path()).expect("open fresh db");
        db.conn
            .execute_batch("PRAGMA user_version = 0;")
            .expect("reset version");
        drop(db);

        let db = Database::open(dir.path()).expect("reopen legacy db");
        assert_eq!(db.schema_version().expect("read version"), SCHEMA_VERSION);
    }

    /// A database from a future Dirigent is refused rather than silently
    /// half-understood.
    #[test]
    fn open_refuses_newer_schema_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Database::open(dir.path()).expect("open fresh db");
        db.conn
            .execute_batch(&format!("PRAGMA user_version = {};", SCHEMA_VERSION + 1))
            .expect("bump version");
        drop(db);

        let Err(err) = Database::open(dir.path()) else {
            panic!("must refuse newer schema");
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("newer version of Dirigent"),
            "unexpected error: {msg}"
        );
    }
}
