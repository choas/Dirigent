use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CueStatus {
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
pub enum ExecutionStatus {
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
pub struct Cue {
    pub id: i64,
    pub text: String,
    pub file_path: String,
    pub line_number: usize,
    pub line_number_end: Option<usize>,
    pub status: CueStatus,
}

#[derive(Debug, Clone)]
pub struct Execution {
    pub id: i64,
    pub cue_id: i64,
    pub prompt: String,
    pub response: Option<String>,
    pub diff: Option<String>,
    pub status: ExecutionStatus,
}

pub struct Database {
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
        Ok(())
    }

    // -- Cue CRUD --

    pub fn insert_cue(
        &self,
        text: &str,
        file_path: &str,
        line_number: usize,
        line_number_end: Option<usize>,
    ) -> Result<i64> {
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

    pub fn get_cue(&self, id: i64) -> Result<Option<Cue>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, text, file_path, line_number, line_number_end, status FROM cues WHERE id = ?1")?;
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

    pub fn all_cues(&self) -> Result<Vec<Cue>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, text, file_path, line_number, line_number_end, status FROM cues ORDER BY id")?;
        let rows = stmt.query_map([], |row| row_to_cue(row))?;
        let mut cues = Vec::new();
        for row in rows {
            cues.push(row?);
        }
        Ok(cues)
    }

    pub fn cues_for_file(&self, file_path: &str) -> Result<Vec<Cue>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, text, file_path, line_number, line_number_end, status FROM cues WHERE file_path = ?1 ORDER BY line_number",
        )?;
        let rows = stmt.query_map(params![file_path], |row| row_to_cue(row))?;
        let mut cues = Vec::new();
        for row in rows {
            cues.push(row?);
        }
        Ok(cues)
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

    pub fn get_latest_execution(&self, cue_id: i64) -> Result<Option<Execution>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, cue_id, prompt, response, diff, status FROM executions WHERE cue_id = ?1 ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![cue_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_execution(row)?))
        } else {
            Ok(None)
        }
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
    })
}

fn row_to_execution(row: &rusqlite::Row) -> rusqlite::Result<Execution> {
    let status_str: String = row.get(5)?;
    Ok(Execution {
        id: row.get(0)?,
        cue_id: row.get(1)?,
        prompt: row.get(2)?,
        response: row.get(3)?,
        diff: row.get(4)?,
        status: ExecutionStatus::from_str(&status_str).unwrap_or(ExecutionStatus::Pending),
    })
}
