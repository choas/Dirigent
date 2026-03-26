mod activity;
mod agent_runs;
mod converters;
mod cue_ops;
mod execution_ops;
mod migrations;
mod source_ops;
mod types;

pub(crate) use agent_runs::{AgentRunEntry, AgentRunRecord};
pub(crate) use migrations::Database;
#[allow(unused_imports)]
pub(crate) use types::{
    ActivityEntry, Cue, CueHistoryRow, CueStatus, Execution, ExecutionMetrics, ExecutionStatus,
};

#[cfg(test)]
impl Database {
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn =
            rusqlite::Connection::open_in_memory().with_context(|| "opening in-memory database")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let db = Database { conn };
        db.create_tables()?;
        Ok(db)
    }

    pub fn get_cue(&self, id: i64) -> anyhow::Result<Option<Cue>> {
        use rusqlite::params;
        let mut stmt = self
            .conn
            .prepare("SELECT id, text, file_path, line_number, line_number_end, status, source_label, source_ref, attached_images, tag FROM cues WHERE id = ?1")?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(converters::row_to_cue(row)?))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
use anyhow::Context;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::CliProvider;

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
        db.complete_execution(
            exec_id,
            "response text",
            Some("diff content"),
            Some(0.0123),
            Some(5000),
            Some(3),
        )
        .unwrap();
        let exec = db.get_latest_execution(cue_id).unwrap().unwrap();
        assert_eq!(exec.status, ExecutionStatus::Completed);
        assert_eq!(exec.response.as_deref(), Some("response text"));
        assert_eq!(exec.diff.as_deref(), Some("diff content"));
        assert!((exec.cost_usd.unwrap() - 0.0123).abs() < 0.0001);
        assert_eq!(exec.duration_ms, Some(5000));
        assert_eq!(exec.num_turns, Some(3));
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
        db.insert_cue_from_source("issue title", "GitHub", "gh#42", "", 0)
            .unwrap();
        assert!(db.cue_exists_by_source_ref("gh#42").unwrap());
        assert!(!db.cue_exists_by_source_ref("gh#99").unwrap());
    }

    #[test]
    fn update_cue_by_source_ref() {
        let db = test_db();
        let id = db
            .insert_cue_from_source("old title", "GitHub", "gh#1", "", 0)
            .unwrap();
        db.update_cue_by_source_ref("gh#1", "new title", "src/main.rs", 42)
            .unwrap();
        let cue = db.get_cue(id).unwrap().unwrap();
        assert_eq!(cue.text, "new title");
        assert_eq!(cue.file_path, "src/main.rs");
        assert_eq!(cue.line_number, 42);
    }

    #[test]
    fn source_cue_has_correct_fields() {
        let db = test_db();
        let id = db
            .insert_cue_from_source("body", "Notion", "notion:abc", "", 0)
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
