use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CueStatus {
    Inbox,
    Ready,
    Review,
    Done,
    Archived,
}

impl CueStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CueStatus::Inbox => "inbox",
            CueStatus::Ready => "ready",
            CueStatus::Review => "review",
            CueStatus::Done => "done",
            CueStatus::Archived => "archived",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "inbox" => Some(CueStatus::Inbox),
            "ready" => Some(CueStatus::Ready),
            "review" => Some(CueStatus::Review),
            "done" => Some(CueStatus::Done),
            "archived" => Some(CueStatus::Archived),
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
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            CueStatus::Inbox => "Inbox",
            CueStatus::Ready => "Running",
            CueStatus::Review => "Review",
            CueStatus::Done => "Done",
            CueStatus::Archived => "Archived",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

#[derive(Debug, Clone)]
pub(crate) struct Execution {
    #[allow(dead_code)]
    pub id: i64,
    #[allow(dead_code)]
    pub cue_id: i64,
    #[allow(dead_code)]
    pub prompt: String,
    #[allow(dead_code)]
    pub response: Option<String>,
    pub diff: Option<String>,
    pub log: Option<String>,
    #[allow(dead_code)]
    pub status: ExecutionStatus,
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
        let has_old_table: bool = self
            .conn
            .prepare("SELECT 1 FROM comments LIMIT 0")
            .is_ok();
        if has_old_table {
            // Drop empty cues table if it was already created, so rename can succeed
            let _ = self.conn.execute_batch("DROP TABLE IF EXISTS cues;");
            self.conn.execute_batch(
                "ALTER TABLE comments RENAME TO cues;",
            )?;
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
            let _ = self.conn.execute_batch(
                "ALTER TABLE cues ADD COLUMN line_number_end INTEGER;",
            );
        }
        // Migration: rename comment_id -> cue_id in executions
        let has_old_col: bool = self
            .conn
            .prepare("SELECT comment_id FROM executions LIMIT 0")
            .is_ok();
        if has_old_col {
            let _ = self.conn.execute_batch(
                "ALTER TABLE executions RENAME COLUMN comment_id TO cue_id;",
            );
        }
        // Migration: add log column to executions
        let has_log_col: bool = self
            .conn
            .prepare("SELECT log FROM executions LIMIT 0")
            .is_ok();
        if !has_log_col {
            let _ = self.conn.execute_batch(
                "ALTER TABLE executions ADD COLUMN log TEXT;",
            );
        }
        // Migration: add source_label and source_ref columns to cues
        let has_source_label: bool = self
            .conn
            .prepare("SELECT source_label FROM cues LIMIT 0")
            .is_ok();
        if !has_source_label {
            let _ = self.conn.execute_batch(
                "ALTER TABLE cues ADD COLUMN source_label TEXT;",
            );
        }
        let has_source_ref: bool = self
            .conn
            .prepare("SELECT source_ref FROM cues LIMIT 0")
            .is_ok();
        if !has_source_ref {
            let _ = self.conn.execute_batch(
                "ALTER TABLE cues ADD COLUMN source_ref TEXT;",
            );
        }
        // Index on status for faster filtered queries
        let _ = self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_cues_status ON cues(status);",
        );
        Ok(())
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
    ) -> Result<i64> {
        let text = Self::clamp_cue_text(text);
        self.conn.execute(
            "INSERT INTO cues (text, file_path, line_number, line_number_end, status) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                text,
                file_path,
                line_number as i64,
                line_number_end.map(|n| n as i64),
                CueStatus::Inbox.as_str()
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    #[allow(dead_code)] // Used in tests
    pub fn get_cue(&self, id: i64) -> Result<Option<Cue>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, text, file_path, line_number, line_number_end, status, source_label, source_ref FROM cues WHERE id = ?1")?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_cue(row)?))
        } else {
            Ok(None)
        }
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
        self.conn.execute(
            "UPDATE cues SET text = ?1 WHERE id = ?2",
            params![text, id],
        )?;
        Ok(())
    }

    pub fn delete_cue(&self, id: i64) -> Result<()> {
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
            "SELECT id, text, file_path, line_number, line_number_end, status, source_label, source_ref FROM cues WHERE status != 'archived' ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| row_to_cue(row))?;
        for row in rows {
            cues.push(row?);
        }
        let mut stmt = self.conn.prepare(
            "SELECT id, text, file_path, line_number, line_number_end, status, source_label, source_ref FROM cues WHERE status = 'archived' ORDER BY id DESC LIMIT ?1",
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

    pub fn insert_execution(&self, cue_id: i64, prompt: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO executions (cue_id, prompt, status) VALUES (?1, ?2, ?3)",
            params![cue_id, prompt, ExecutionStatus::Pending.as_str()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn complete_execution(
        &self,
        id: i64,
        response: &str,
        diff: Option<&str>,
    ) -> Result<()> {
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
            "SELECT id, cue_id, prompt, response, diff, log, status FROM executions WHERE cue_id = ?1 ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![cue_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_execution(row)?))
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
        Ok(self.conn.last_insert_rowid())
    }

}

fn row_to_cue(row: &rusqlite::Row) -> rusqlite::Result<Cue> {
    let status_str: String = row.get(5)?;
    let line_end: Option<i64> = row.get(4)?;
    Ok(Cue {
        id: row.get(0)?,
        text: row.get(1)?,
        file_path: row.get(2)?,
        line_number: row.get::<_, i64>(3)? as usize,
        line_number_end: line_end.map(|n| n as usize),
        status: CueStatus::from_str(&status_str).unwrap_or(CueStatus::Inbox),
        source_label: row.get(6)?,
        source_ref: row.get(7)?,
    })
}

#[cfg(test)]
impl Database {
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .with_context(|| "opening in-memory database")?;
        let db = Database { conn };
        db.create_tables()?;
        Ok(db)
    }
}

fn row_to_execution(row: &rusqlite::Row) -> rusqlite::Result<Execution> {
    let status_str: String = row.get(6)?;
    Ok(Execution {
        id: row.get(0)?,
        cue_id: row.get(1)?,
        prompt: row.get(2)?,
        response: row.get(3)?,
        diff: row.get(4)?,
        log: row.get(5)?,
        status: ExecutionStatus::from_str(&status_str).unwrap_or(ExecutionStatus::Pending),
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
        let id = db.insert_cue("fix bug", "src/main.rs", 42, None).unwrap();
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
        let id = db.insert_cue("refactor", "lib.rs", 10, Some(20)).unwrap();
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
        let id = db.insert_cue("task", "f.rs", 1, None).unwrap();
        db.update_cue_status(id, CueStatus::Ready).unwrap();
        let cue = db.get_cue(id).unwrap().unwrap();
        assert_eq!(cue.status, CueStatus::Ready);
    }

    #[test]
    fn update_cue_text() {
        let db = test_db();
        let id = db.insert_cue("old", "f.rs", 1, None).unwrap();
        db.update_cue_text(id, "new text").unwrap();
        let cue = db.get_cue(id).unwrap().unwrap();
        assert_eq!(cue.text, "new text");
    }

    #[test]
    fn delete_cue_removes_cue_and_executions() {
        let db = test_db();
        let cue_id = db.insert_cue("task", "f.rs", 1, None).unwrap();
        db.insert_execution(cue_id, "do something").unwrap();
        db.delete_cue(cue_id).unwrap();
        assert!(db.get_cue(cue_id).unwrap().is_none());
        assert!(db.get_latest_execution(cue_id).unwrap().is_none());
    }

    // -- Execution CRUD --

    #[test]
    fn insert_and_get_execution() {
        let db = test_db();
        let cue_id = db.insert_cue("task", "f.rs", 1, None).unwrap();
        let exec_id = db.insert_execution(cue_id, "prompt text").unwrap();
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
        let cue_id = db.insert_cue("task", "f.rs", 1, None).unwrap();
        let exec_id = db.insert_execution(cue_id, "prompt").unwrap();
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
        let cue_id = db.insert_cue("task", "f.rs", 1, None).unwrap();
        let exec_id = db.insert_execution(cue_id, "prompt").unwrap();
        db.fail_execution(exec_id, "error msg").unwrap();
        let exec = db.get_latest_execution(cue_id).unwrap().unwrap();
        assert_eq!(exec.status, ExecutionStatus::Failed);
        assert_eq!(exec.response.as_deref(), Some("error msg"));
    }

    #[test]
    fn update_execution_log() {
        let db = test_db();
        let cue_id = db.insert_cue("task", "f.rs", 1, None).unwrap();
        let exec_id = db.insert_execution(cue_id, "prompt").unwrap();
        db.update_execution_log(exec_id, "log line 1\nlog line 2")
            .unwrap();
        let exec = db.get_latest_execution(cue_id).unwrap().unwrap();
        assert_eq!(exec.log.as_deref(), Some("log line 1\nlog line 2"));
    }

    #[test]
    fn get_latest_execution_returns_most_recent() {
        let db = test_db();
        let cue_id = db.insert_cue("task", "f.rs", 1, None).unwrap();
        db.insert_execution(cue_id, "first").unwrap();
        let second_id = db.insert_execution(cue_id, "second").unwrap();
        let exec = db.get_latest_execution(cue_id).unwrap().unwrap();
        assert_eq!(exec.id, second_id);
        assert_eq!(exec.prompt, "second");
    }

    #[test]
    fn get_latest_execution_no_executions() {
        let db = test_db();
        let cue_id = db.insert_cue("task", "f.rs", 1, None).unwrap();
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
}
