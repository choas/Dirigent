use anyhow::Result;
use chrono::Local;
use rusqlite::params;

use super::converters::{row_to_cue, CUE_COLUMNS};
use super::types::{Cue, CueStatus};
use super::Database;

impl Database {
    // -- Cue CRUD --

    /// Maximum allowed length for cue text (bytes). Longer text is truncated.
    const MAX_CUE_TEXT_LEN: usize = 100_000;

    /// Truncate cue text to [`MAX_CUE_TEXT_LEN`] on a char boundary.
    pub(super) fn clamp_cue_text(text: &str) -> &str {
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
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
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
        let id = tx.last_insert_rowid();
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        tx.execute(
            "INSERT INTO cue_activity_log (cue_id, timestamp, event) VALUES (?1, ?2, ?3)",
            params![id, timestamp, "Created"],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Convenience wrapper for creating a cue that is not attached to any file.
    pub fn insert_global_cue(&self, text: &str) -> Result<i64> {
        self.insert_cue(text, "", 0, None, &[])
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

    pub fn update_cue_plan_path(&self, id: i64, plan_path: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE cues SET plan_path = ?1 WHERE id = ?2",
            params![plan_path, id],
        )?;
        Ok(())
    }

    pub fn delete_cue(&self, id: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM cue_activity_log WHERE cue_id = ?1",
            params![id],
        )?;
        tx.execute("DELETE FROM executions WHERE cue_id = ?1", params![id])?;
        tx.execute("DELETE FROM cues WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(())
    }

    /// Delete all archived cues and their related records.
    pub fn delete_all_archived(&self) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute_batch(
            "DELETE FROM cue_activity_log WHERE cue_id IN (SELECT id FROM cues WHERE status = 'archived');
             DELETE FROM executions WHERE cue_id IN (SELECT id FROM cues WHERE status = 'archived');
             DELETE FROM cues WHERE status = 'archived';",
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Load all non-archived cues plus the most recent `archived_limit` archived cues.
    pub fn all_cues_limited_archived(&self, archived_limit: usize) -> Result<Vec<Cue>> {
        let mut cues = Vec::new();
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {CUE_COLUMNS} FROM cues WHERE status != 'archived' ORDER BY id"
        ))?;
        let rows = stmt.query_map([], row_to_cue)?;
        for row in rows {
            cues.push(row?);
        }
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {CUE_COLUMNS} FROM cues WHERE status = 'archived' ORDER BY id DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params![archived_limit as i64], row_to_cue)?;
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

    /// Count archived cues matching a specific source label.
    pub fn archived_cue_count_by_source(&self, source_label: &str) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM cues WHERE status = 'archived' AND source_label = ?1",
            [source_label],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }
}
