use anyhow::{Context, Result};
use chrono::Local;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::settings::CliProvider;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CueStatus {
    Inbox,
    Ready,
    Review,
    Done,
    Archived,
    Backlog,
}

impl CueStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CueStatus::Inbox => "inbox",
            CueStatus::Ready => "ready",
            CueStatus::Review => "review",
            CueStatus::Done => "done",
            CueStatus::Archived => "archived",
            CueStatus::Backlog => "backlog",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "inbox" => Some(CueStatus::Inbox),
            "ready" => Some(CueStatus::Ready),
            "review" => Some(CueStatus::Review),
            "done" => Some(CueStatus::Done),
            "archived" => Some(CueStatus::Archived),
            "backlog" => Some(CueStatus::Backlog),
            _ => None,
        }
    }

    pub fn all() -> &'static [CueStatus] {
        &[
            CueStatus::Inbox,
            CueStatus::Ready,
            CueStatus::Review,
            CueStatus::Done,
            CueStatus::Archived,
            CueStatus::Backlog,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            CueStatus::Inbox => "Inbox",
            CueStatus::Ready => "Running",
            CueStatus::Review => "Review",
            CueStatus::Done => "Done",
            CueStatus::Archived => "Archived",
            CueStatus::Backlog => "Backlog",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ExecutionStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl ExecutionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutionStatus::Pending => "pending",
            ExecutionStatus::Running => "running",
            ExecutionStatus::Completed => "completed",
            ExecutionStatus::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(ExecutionStatus::Pending),
            "running" => Some(ExecutionStatus::Running),
            "completed" => Some(ExecutionStatus::Completed),
            "failed" => Some(ExecutionStatus::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Cue {
    pub id: i64,
    pub text: String,
    pub file_path: String,
    pub line_number: usize,
    pub line_number_end: Option<usize>,
    pub status: CueStatus,
    pub source_label: Option<String>,
    #[allow(dead_code)]
    pub source_ref: Option<String>,
    /// Attached image file paths (stored as JSON array in DB).
    pub attached_images: Vec<String>,
    /// Optional user-assigned tag for grouping/labeling cues.
    pub tag: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Execution {
    #[allow(dead_code)]
    pub id: i64,
    #[allow(dead_code)]
    pub cue_id: i64,
    #[allow(dead_code)]
    pub prompt: String,
    pub response: Option<String>,
    pub diff: Option<String>,
    pub log: Option<String>,
    #[allow(dead_code)]
    pub status: ExecutionStatus,
    pub provider: CliProvider,
}

#[derive(Debug, Clone)]
pub(crate) struct ActivityEntry {
    pub timestamp: String,
    pub event: String,
}

pub(crate) struct Database {
    conn: Connection,
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

    fn create_tables(&self) -> Result<()> {
        // Migration: rename comments -> cues if old table exists
        let has_old_table: bool = self.conn.prepare("SELECT 1 FROM comments LIMIT 0").is_ok();
        if has_old_table {
            // Drop empty cues table if it was already created, so rename can succeed
            let _ = self.conn.execute_batch("DROP TABLE IF EXISTS cues;");
            self.conn
                .execute_batch("ALTER TABLE comments RENAME TO cues;")?;
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

    // -- Cue CRUD --

    /// Maximum allowed length for cue text (bytes). Longer text is truncated.
    const MAX_CUE_TEXT_LEN: usize = 100_000;

    /// Truncate cue text to [`MAX_CUE_TEXT_LEN`] on a char boundary.
    fn clamp_cue_text(text: &str) -> &str {
        if text.len() <= Self::MAX_CUE_TEXT_LEN {
            text
        } else {
            let mut end = Self::MAX_CUE_TEXT_LEN;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            &text[..end]
        }
    }

    pub fn insert_cue(
        &self,
        text: &str,
        file_path: &str,
        line_number: usize,
        line_number_end: Option<usize>,
        images: &[String],
    ) -> Result<i64> {
        let text = Self::clamp_cue_text(text);
        let images_json = if images.is_empty() {
            None
        } else {
            Some(serde_json::to_string(images).unwrap_or_default())
        };
        self.conn.execute(
            "INSERT INTO cues (text, file_path, line_number, line_number_end, status, attached_images) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                text,
                file_path,
                line_number as i64,
                line_number_end.map(|n| n as i64),
                CueStatus::Inbox.as_str(),
                images_json,
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        let _ = self.log_activity(id, "Created");
        Ok(id)
    }

    pub fn update_cue_status(&self, id: i64, status: CueStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE cues SET status = ?1 WHERE id = ?2",
            params![status.as_str(), id],
        )?;
        Ok(())
    }

    pub fn update_cue_text(&self, id: i64, text: &str) -> Result<()> {
        let text = Self::clamp_cue_text(text);
        self.conn
            .execute("UPDATE cues SET text = ?1 WHERE id = ?2", params![text, id])?;
        Ok(())
    }

    pub fn update_cue_tag(&self, id: i64, tag: Option<&str>) -> Result<()> {
        self.conn
            .execute("UPDATE cues SET tag = ?1 WHERE id = ?2", params![tag, id])?;
        Ok(())
    }

    pub fn delete_cue(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM cue_activity_log WHERE cue_id = ?1",
            params![id],
        )?;
        self.conn
            .execute("DELETE FROM executions WHERE cue_id = ?1", params![id])?;
        self.conn
            .execute("DELETE FROM cues WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Load all non-archived cues plus the most recent `archived_limit` archived cues.
    pub fn all_cues_limited_archived(&self, archived_limit: usize) -> Result<Vec<Cue>> {
        let mut cues = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT id, text, file_path, line_number, line_number_end, status, source_label, source_ref, attached_images, tag FROM cues WHERE status != 'archived' ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| row_to_cue(row))?;
        for row in rows {
            cues.push(row?);
        }
        let mut stmt = self.conn.prepare(
            "SELECT id, text, file_path, line_number, line_number_end, status, source_label, source_ref, attached_images, tag FROM cues WHERE status = 'archived' ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![archived_limit as i64], |row| row_to_cue(row))?;
        for row in rows {
            cues.push(row?);
        }
        cues.sort_by_key(|c| c.id);
        Ok(cues)
    }

    /// Count total archived cues (for UI display when limit is applied).
    pub fn archived_cue_count(&self) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM cues WHERE status = 'archived'",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    // -- Execution CRUD --

    pub fn insert_execution(
        &self,
        cue_id: i64,
        prompt: &str,
        provider: &CliProvider,
    ) -> Result<i64> {
        let provider_str = provider.display_name();
        self.conn.execute(
            "INSERT INTO executions (cue_id, prompt, status, provider) VALUES (?1, ?2, ?3, ?4)",
            params![
                cue_id,
                prompt,
                ExecutionStatus::Pending.as_str(),
                provider_str
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn complete_execution(&self, id: i64, response: &str, diff: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE executions SET response = ?1, diff = ?2, status = ?3 WHERE id = ?4",
            params![response, diff, ExecutionStatus::Completed.as_str(), id],
        )?;
        Ok(())
    }

    pub fn fail_execution(&self, id: i64, response: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE executions SET response = ?1, status = ?2 WHERE id = ?3",
            params![response, ExecutionStatus::Failed.as_str(), id],
        )?;
        Ok(())
    }

    pub fn update_execution_log(&self, id: i64, log: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE executions SET log = ?1 WHERE id = ?2",
            params![log, id],
        )?;
        Ok(())
    }

    pub fn get_latest_execution(&self, cue_id: i64) -> Result<Option<Execution>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, cue_id, prompt, response, diff, log, status, provider FROM executions WHERE cue_id = ?1 ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![cue_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_execution(row)?))
        } else {
            Ok(None)
        }
    }

    /// Get all executions for a cue, ordered by id (oldest first).
    pub fn get_all_executions(&self, cue_id: i64) -> Result<Vec<Execution>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, cue_id, prompt, response, diff, log, status, provider FROM executions WHERE cue_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![cue_id], |row| row_to_execution(row))?;
        let mut execs = Vec::new();
        for row in rows {
            execs.push(row?);
        }
        Ok(execs)
    }

    /// Get the last activity event matching a prefix for a cue.
    pub fn get_last_activity_matching(&self, cue_id: i64, prefix: &str) -> Result<Option<String>> {
        let pattern = format!("{}%", prefix);
        let mut stmt = self.conn.prepare(
            "SELECT event FROM cue_activity_log WHERE cue_id = ?1 AND event LIKE ?2 ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![cue_id, pattern])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    // -- Source integration --

    /// Check if a cue with the given source_ref already exists (for deduplication).
    pub fn cue_exists_by_source_ref(&self, source_ref: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM cues WHERE source_ref = ?1",
            params![source_ref],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get the cue id, text, and status for a given source_ref (for refresh/update logic).
    pub fn get_cue_by_source_ref(&self, source_ref: &str) -> Result<Option<(i64, String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, text, status FROM cues WHERE source_ref = ?1 LIMIT 1")?;
        let result = stmt.query_row(params![source_ref], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        });
        match result {
            Ok(row) => Ok(Some(row)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Update the text of an existing cue identified by source_ref.
    pub fn update_cue_text_by_source_ref(&self, source_ref: &str, text: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE cues SET text = ?1 WHERE source_ref = ?2",
            params![text, source_ref],
        )?;
        Ok(())
    }

    /// Insert a cue from an external source (global cue with source tracking).
    pub fn insert_cue_from_source(
        &self,
        text: &str,
        source_label: &str,
        source_ref: &str,
    ) -> Result<i64> {
        let text = Self::clamp_cue_text(text);
        self.conn.execute(
            "INSERT INTO cues (text, file_path, line_number, status, source_label, source_ref) VALUES (?1, '', 0, ?2, ?3, ?4)",
            params![text, CueStatus::Inbox.as_str(), source_label, source_ref],
        )?;
        let id = self.conn.last_insert_rowid();
        let _ = self.log_activity(id, &format!("Created from {}", source_label));
        Ok(id)
    }

    // -- Activity log --

    /// Record a timestamped activity event for a cue.
    pub fn log_activity(&self, cue_id: i64, event: &str) -> Result<()> {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        self.conn.execute(
            "INSERT INTO cue_activity_log (cue_id, timestamp, event) VALUES (?1, ?2, ?3)",
            params![cue_id, timestamp, event],
        )?;
        Ok(())
    }

    /// Get all activity entries for a cue, ordered oldest first.
    pub fn get_activities(&self, cue_id: i64) -> Result<Vec<ActivityEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, event FROM cue_activity_log WHERE cue_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![cue_id], |row| {
            Ok(ActivityEntry {
                timestamp: row.get(0)?,
                event: row.get(1)?,
            })
        })?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    // -- Agent runs --

    /// Record a completed agent run.
    pub fn insert_agent_run(
        &self,
        agent_kind: &str,
        cue_id: Option<i64>,
        command: &str,
        status: &str,
        output: &str,
        diagnostics_json: Option<&str>,
        duration_ms: u64,
    ) -> Result<i64> {
        let now = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        self.conn.execute(
            "INSERT INTO agent_runs (agent_kind, cue_id, command, status, output, diagnostics, duration_ms, started_at, finished_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![agent_kind, cue_id, command, status, output, diagnostics_json, duration_ms as i64, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Get agent run output for a specific cue, most recent first.
    pub fn get_agent_runs_for_cue(&self, cue_id: i64) -> Result<Vec<AgentRunEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT agent_kind, status, output, duration_ms, started_at, command, cue_id
             FROM agent_runs WHERE cue_id = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt.query_map(params![cue_id], |row| {
            Ok(AgentRunEntry {
                agent_kind: row.get(0)?,
                status: row.get(1)?,
                output: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                duration_ms: row.get::<_, i64>(3)? as u64,
                started_at: row.get(4)?,
                command: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                cue_id: row.get(6)?,
            })
        })?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    /// Get recent agent runs for a specific agent kind, most recent first.
    pub fn get_recent_agent_runs_by_kind(
        &self,
        kind: &str,
        limit: usize,
    ) -> Result<Vec<AgentRunEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT agent_kind, status, output, duration_ms, started_at, command, cue_id
             FROM agent_runs WHERE agent_kind = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![kind, limit as i64], |row| {
            Ok(AgentRunEntry {
                agent_kind: row.get(0)?,
                status: row.get(1)?,
                output: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                duration_ms: row.get::<_, i64>(3)? as u64,
                started_at: row.get(4)?,
                command: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                cue_id: row.get(6)?,
            })
        })?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    /// Prune old agent runs, keeping only the most recent `keep_per_kind` runs per agent kind.
    /// Also truncates large output fields to `max_output_bytes`.
    pub fn cleanup_agent_runs(
        &self,
        keep_per_kind: usize,
        max_output_bytes: usize,
    ) -> Result<usize> {
        let mut total_deleted = 0usize;
        let kinds = ["format", "lint", "build", "test"];
        for kind in &kinds {
            // Delete old runs beyond the retention limit
            let deleted = self.conn.execute(
                "DELETE FROM agent_runs WHERE agent_kind = ?1 AND id NOT IN (
                    SELECT id FROM agent_runs WHERE agent_kind = ?1 ORDER BY id DESC LIMIT ?2
                )",
                params![kind, keep_per_kind as i64],
            )?;
            total_deleted += deleted;
        }

        // Truncate large output fields
        self.conn.execute(
            "UPDATE agent_runs SET output = SUBSTR(output, 1, ?1) WHERE LENGTH(output) > ?1",
            params![max_output_bytes as i64],
        )?;

        Ok(total_deleted)
    }
}

/// An agent run entry returned by [`Database::get_agent_runs_for_cue`].
#[derive(Debug, Clone)]
pub(crate) struct AgentRunEntry {
    pub agent_kind: String,
    pub status: String,
    pub output: String,
    pub duration_ms: u64,
    pub started_at: String,
    pub command: String,
    pub cue_id: Option<i64>,
}

fn row_to_cue(row: &rusqlite::Row) -> rusqlite::Result<Cue> {
    let status_str: String = row.get(5)?;
    let line_end: Option<i64> = row.get(4)?;
    let images_json: Option<String> = row.get(8)?;
    let attached_images: Vec<String> = images_json
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    Ok(Cue {
        id: row.get(0)?,
        text: row.get(1)?,
        file_path: row.get(2)?,
        line_number: row.get::<_, i64>(3)? as usize,
        line_number_end: line_end.map(|n| n as usize),
        status: CueStatus::from_str(&status_str).unwrap_or(CueStatus::Inbox),
        source_label: row.get(6)?,
        source_ref: row.get(7)?,
        attached_images,
        tag: row.get(9)?,
    })
}

#[cfg(test)]
impl Database {
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().with_context(|| "opening in-memory database")?;
        let db = Database { conn };
        db.create_tables()?;
        Ok(db)
    }

    pub fn get_cue(&self, id: i64) -> Result<Option<Cue>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, text, file_path, line_number, line_number_end, status, source_label, source_ref, attached_images, tag FROM cues WHERE id = ?1")?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_cue(row)?))
        } else {
            Ok(None)
        }
    }
}

fn row_to_execution(row: &rusqlite::Row) -> rusqlite::Result<Execution> {
    let status_str: String = row.get(6)?;
    let provider_str: String = row.get(7)?;
    let provider = match provider_str.as_str() {
        "OpenCode" => CliProvider::OpenCode,
        _ => CliProvider::Claude,
    };
    Ok(Execution {
        id: row.get(0)?,
        cue_id: row.get(1)?,
        prompt: row.get(2)?,
        response: row.get(3)?,
        diff: row.get(4)?,
        log: row.get(5)?,
        status: ExecutionStatus::from_str(&status_str).unwrap_or(ExecutionStatus::Pending),
        provider,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::open_in_memory().expect("in-memory db")
    }

    // -- CueStatus / ExecutionStatus roundtrips --

    #[test]
    fn cue_status_roundtrip() {
        for status in CueStatus::all() {
            let s = status.as_str();
            assert_eq!(CueStatus::from_str(s), Some(*status));
        }
    }

    #[test]
    fn cue_status_from_str_unknown_returns_none() {
        assert_eq!(CueStatus::from_str("bogus"), None);
    }

    #[test]
    fn execution_status_roundtrip() {
        for status in &[
            ExecutionStatus::Pending,
            ExecutionStatus::Running,
            ExecutionStatus::Completed,
            ExecutionStatus::Failed,
        ] {
            assert_eq!(ExecutionStatus::from_str(status.as_str()), Some(*status));
        }
    }

    #[test]
    fn execution_status_from_str_unknown_returns_none() {
        assert_eq!(ExecutionStatus::from_str("xyz"), None);
    }

    // -- Cue CRUD --

    #[test]
    fn insert_and_get_cue() {
        let db = test_db();
        let id = db
            .insert_cue("fix bug", "src/main.rs", 42, None, &[])
            .unwrap();
        let cue = db.get_cue(id).unwrap().expect("cue should exist");
        assert_eq!(cue.text, "fix bug");
        assert_eq!(cue.file_path, "src/main.rs");
        assert_eq!(cue.line_number, 42);
        assert_eq!(cue.line_number_end, None);
        assert_eq!(cue.status, CueStatus::Inbox);
    }

    #[test]
    fn insert_cue_with_line_range() {
        let db = test_db();
        let id = db
            .insert_cue("refactor", "lib.rs", 10, Some(20), &[])
            .unwrap();
        let cue = db.get_cue(id).unwrap().unwrap();
        assert_eq!(cue.line_number, 10);
        assert_eq!(cue.line_number_end, Some(20));
    }

    #[test]
    fn get_nonexistent_cue_returns_none() {
        let db = test_db();
        assert!(db.get_cue(999).unwrap().is_none());
    }

    #[test]
    fn update_cue_status() {
        let db = test_db();
        let id = db.insert_cue("task", "f.rs", 1, None, &[]).unwrap();
        db.update_cue_status(id, CueStatus::Ready).unwrap();
        let cue = db.get_cue(id).unwrap().unwrap();
        assert_eq!(cue.status, CueStatus::Ready);
    }

    #[test]
    fn update_cue_text() {
        let db = test_db();
        let id = db.insert_cue("old", "f.rs", 1, None, &[]).unwrap();
        db.update_cue_text(id, "new text").unwrap();
        let cue = db.get_cue(id).unwrap().unwrap();
        assert_eq!(cue.text, "new text");
    }

    #[test]
    fn delete_cue_removes_cue_and_executions() {
        let db = test_db();
        let cue_id = db.insert_cue("task", "f.rs", 1, None, &[]).unwrap();
        db.insert_execution(cue_id, "do something", &CliProvider::Claude)
            .unwrap();
        db.delete_cue(cue_id).unwrap();
        assert!(db.get_cue(cue_id).unwrap().is_none());
        assert!(db.get_latest_execution(cue_id).unwrap().is_none());
    }

    // -- Execution CRUD --

    #[test]
    fn insert_and_get_execution() {
        let db = test_db();
        let cue_id = db.insert_cue("task", "f.rs", 1, None, &[]).unwrap();
        let exec_id = db
            .insert_execution(cue_id, "prompt text", &CliProvider::Claude)
            .unwrap();
        let exec = db.get_latest_execution(cue_id).unwrap().unwrap();
        assert_eq!(exec.id, exec_id);
        assert_eq!(exec.prompt, "prompt text");
        assert_eq!(exec.status, ExecutionStatus::Pending);
        assert!(exec.response.is_none());
        assert!(exec.diff.is_none());
    }

    #[test]
    fn complete_execution() {
        let db = test_db();
        let cue_id = db.insert_cue("task", "f.rs", 1, None, &[]).unwrap();
        let exec_id = db
            .insert_execution(cue_id, "prompt", &CliProvider::Claude)
            .unwrap();
        db.complete_execution(exec_id, "response text", Some("diff content"))
            .unwrap();
        let exec = db.get_latest_execution(cue_id).unwrap().unwrap();
        assert_eq!(exec.status, ExecutionStatus::Completed);
        assert_eq!(exec.response.as_deref(), Some("response text"));
        assert_eq!(exec.diff.as_deref(), Some("diff content"));
    }

    #[test]
    fn fail_execution() {
        let db = test_db();
        let cue_id = db.insert_cue("task", "f.rs", 1, None, &[]).unwrap();
        let exec_id = db
            .insert_execution(cue_id, "prompt", &CliProvider::Claude)
            .unwrap();
        db.fail_execution(exec_id, "error msg").unwrap();
        let exec = db.get_latest_execution(cue_id).unwrap().unwrap();
        assert_eq!(exec.status, ExecutionStatus::Failed);
        assert_eq!(exec.response.as_deref(), Some("error msg"));
    }

    #[test]
    fn update_execution_log() {
        let db = test_db();
        let cue_id = db.insert_cue("task", "f.rs", 1, None, &[]).unwrap();
        let exec_id = db
            .insert_execution(cue_id, "prompt", &CliProvider::Claude)
            .unwrap();
        db.update_execution_log(exec_id, "log line 1\nlog line 2")
            .unwrap();
        let exec = db.get_latest_execution(cue_id).unwrap().unwrap();
        assert_eq!(exec.log.as_deref(), Some("log line 1\nlog line 2"));
    }

    #[test]
    fn get_latest_execution_returns_most_recent() {
        let db = test_db();
        let cue_id = db.insert_cue("task", "f.rs", 1, None, &[]).unwrap();
        db.insert_execution(cue_id, "first", &CliProvider::Claude)
            .unwrap();
        let second_id = db
            .insert_execution(cue_id, "second", &CliProvider::Claude)
            .unwrap();
        let exec = db.get_latest_execution(cue_id).unwrap().unwrap();
        assert_eq!(exec.id, second_id);
        assert_eq!(exec.prompt, "second");
    }

    #[test]
    fn get_latest_execution_no_executions() {
        let db = test_db();
        let cue_id = db.insert_cue("task", "f.rs", 1, None, &[]).unwrap();
        assert!(db.get_latest_execution(cue_id).unwrap().is_none());
    }

    // -- Source integration --

    #[test]
    fn insert_cue_from_source_and_find_by_ref() {
        let db = test_db();
        db.insert_cue_from_source("issue title", "GitHub", "gh#42")
            .unwrap();
        assert!(db.cue_exists_by_source_ref("gh#42").unwrap());
        assert!(!db.cue_exists_by_source_ref("gh#99").unwrap());
    }

    #[test]
    fn update_cue_text_by_source_ref() {
        let db = test_db();
        let id = db
            .insert_cue_from_source("old title", "GitHub", "gh#1")
            .unwrap();
        db.update_cue_text_by_source_ref("gh#1", "new title")
            .unwrap();
        let cue = db.get_cue(id).unwrap().unwrap();
        assert_eq!(cue.text, "new title");
    }

    #[test]
    fn source_cue_has_correct_fields() {
        let db = test_db();
        let id = db
            .insert_cue_from_source("body", "Notion", "notion:abc")
            .unwrap();
        let cue = db.get_cue(id).unwrap().unwrap();
        assert_eq!(cue.file_path, "");
        assert_eq!(cue.line_number, 0);
        assert_eq!(cue.source_label.as_deref(), Some("Notion"));
        assert_eq!(cue.source_ref.as_deref(), Some("notion:abc"));
        assert_eq!(cue.status, CueStatus::Inbox);
    }

    #[test]
    fn insert_cue_with_images() {
        let db = test_db();
        let images = vec![
            "/tmp/screenshot.png".to_string(),
            "/tmp/design.jpg".to_string(),
        ];
        let id = db
            .insert_cue("implement design", "ui.rs", 10, None, &images)
            .unwrap();
        let cue = db.get_cue(id).unwrap().unwrap();
        assert_eq!(cue.attached_images.len(), 2);
        assert_eq!(cue.attached_images[0], "/tmp/screenshot.png");
        assert_eq!(cue.attached_images[1], "/tmp/design.jpg");
    }

    #[test]
    fn insert_cue_without_images() {
        let db = test_db();
        let id = db.insert_cue("no images", "f.rs", 1, None, &[]).unwrap();
        let cue = db.get_cue(id).unwrap().unwrap();
        assert!(cue.attached_images.is_empty());
    }
}
