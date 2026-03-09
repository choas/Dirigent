use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentStatus {
    Inbox,
    Ready,
    Review,
    Done,
    Archived,
}

impl CommentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CommentStatus::Inbox => "inbox",
            CommentStatus::Ready => "ready",
            CommentStatus::Review => "review",
            CommentStatus::Done => "done",
            CommentStatus::Archived => "archived",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "inbox" => Some(CommentStatus::Inbox),
            "ready" => Some(CommentStatus::Ready),
            "review" => Some(CommentStatus::Review),
            "done" => Some(CommentStatus::Done),
            "archived" => Some(CommentStatus::Archived),
            _ => None,
        }
    }

    pub fn all() -> &'static [CommentStatus] {
        &[
            CommentStatus::Inbox,
            CommentStatus::Ready,
            CommentStatus::Review,
            CommentStatus::Done,
            CommentStatus::Archived,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            CommentStatus::Inbox => "Inbox",
            CommentStatus::Ready => "Running",
            CommentStatus::Review => "Review",
            CommentStatus::Done => "Done",
            CommentStatus::Archived => "Archived",
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
pub struct Comment {
    pub id: i64,
    pub text: String,
    pub file_path: String,
    pub line_number: usize,
    pub line_number_end: Option<usize>,
    pub status: CommentStatus,
}

#[derive(Debug, Clone)]
pub struct Execution {
    pub id: i64,
    pub comment_id: i64,
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
        let db_dir = project_root.join(".dirigent");
        std::fs::create_dir_all(&db_dir)
            .with_context(|| format!("creating .dirigent dir at {}", db_dir.display()))?;

        let db_path = db_dir.join("dirigent.db");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("opening database at {}", db_path.display()))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        let db = Database { conn };
        db.create_tables()?;
        Ok(db)
    }

    fn create_tables(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS comments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                file_path TEXT NOT NULL,
                line_number INTEGER NOT NULL,
                line_number_end INTEGER,
                status TEXT NOT NULL DEFAULT 'inbox'
            );

            CREATE TABLE IF NOT EXISTS executions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                comment_id INTEGER NOT NULL REFERENCES comments(id),
                prompt TEXT NOT NULL,
                response TEXT,
                diff TEXT,
                status TEXT NOT NULL DEFAULT 'pending'
            );",
        )?;
        // Migration: add line_number_end column if missing
        let has_col: bool = self
            .conn
            .prepare("SELECT line_number_end FROM comments LIMIT 0")
            .is_ok();
        if !has_col {
            let _ = self.conn.execute_batch(
                "ALTER TABLE comments ADD COLUMN line_number_end INTEGER;",
            );
        }
        Ok(())
    }

    // -- Comment CRUD --

    pub fn insert_comment(
        &self,
        text: &str,
        file_path: &str,
        line_number: usize,
        line_number_end: Option<usize>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO comments (text, file_path, line_number, line_number_end, status) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                text,
                file_path,
                line_number as i64,
                line_number_end.map(|n| n as i64),
                CommentStatus::Inbox.as_str()
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_comment(&self, id: i64) -> Result<Option<Comment>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, text, file_path, line_number, line_number_end, status FROM comments WHERE id = ?1")?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_comment(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn update_comment_status(&self, id: i64, status: CommentStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE comments SET status = ?1 WHERE id = ?2",
            params![status.as_str(), id],
        )?;
        Ok(())
    }

    pub fn update_comment_text(&self, id: i64, text: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE comments SET text = ?1 WHERE id = ?2",
            params![text, id],
        )?;
        Ok(())
    }

    pub fn delete_comment(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM executions WHERE comment_id = ?1", params![id])?;
        self.conn
            .execute("DELETE FROM comments WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn all_comments(&self) -> Result<Vec<Comment>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, text, file_path, line_number, line_number_end, status FROM comments ORDER BY id")?;
        let rows = stmt.query_map([], |row| row_to_comment(row))?;
        let mut comments = Vec::new();
        for row in rows {
            comments.push(row?);
        }
        Ok(comments)
    }

    pub fn comments_for_file(&self, file_path: &str) -> Result<Vec<Comment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, text, file_path, line_number, line_number_end, status FROM comments WHERE file_path = ?1 ORDER BY line_number",
        )?;
        let rows = stmt.query_map(params![file_path], |row| row_to_comment(row))?;
        let mut comments = Vec::new();
        for row in rows {
            comments.push(row?);
        }
        Ok(comments)
    }

    // -- Execution CRUD --

    pub fn insert_execution(&self, comment_id: i64, prompt: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO executions (comment_id, prompt, status) VALUES (?1, ?2, ?3)",
            params![comment_id, prompt, ExecutionStatus::Pending.as_str()],
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

    pub fn get_latest_execution(&self, comment_id: i64) -> Result<Option<Execution>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, comment_id, prompt, response, diff, status FROM executions WHERE comment_id = ?1 ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![comment_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_execution(row)?))
        } else {
            Ok(None)
        }
    }
}

fn row_to_comment(row: &rusqlite::Row) -> rusqlite::Result<Comment> {
    let status_str: String = row.get(5)?;
    let line_end: Option<i64> = row.get(4)?;
    Ok(Comment {
        id: row.get(0)?,
        text: row.get(1)?,
        file_path: row.get(2)?,
        line_number: row.get::<_, i64>(3)? as usize,
        line_number_end: line_end.map(|n| n as usize),
        status: CommentStatus::from_str(&status_str).unwrap_or(CommentStatus::Inbox),
    })
}

fn row_to_execution(row: &rusqlite::Row) -> rusqlite::Result<Execution> {
    let status_str: String = row.get(5)?;
    Ok(Execution {
        id: row.get(0)?,
        comment_id: row.get(1)?,
        prompt: row.get(2)?,
        response: row.get(3)?,
        diff: row.get(4)?,
        status: ExecutionStatus::from_str(&status_str).unwrap_or(ExecutionStatus::Pending),
    })
}
