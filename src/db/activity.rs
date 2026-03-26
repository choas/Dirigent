use anyhow::Result;
use chrono::Local;
use rusqlite::params;

use super::types::{ActivityEntry, CueHistoryRow};
use super::Database;

impl Database {
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

    // -- Prompt history search --

    /// Search past cue texts matching a query string (case-insensitive LIKE).
    /// Returns up to `limit` results, most recent first, as
    /// (cue_id, text, file_path, line_number, line_number_end, attached_images).
    pub fn search_cue_history(&self, query: &str, limit: usize) -> Result<Vec<CueHistoryRow>> {
        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("%{}%", escaped);
        let mut stmt = self.conn.prepare(
            "SELECT id, text, file_path, line_number, line_number_end, attached_images FROM cues WHERE text LIKE ?1 ESCAPE '\\' ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit as i64], |row| {
            let line_end: Option<i64> = row.get(4)?;
            let images_json: Option<String> = row.get(5)?;
            let images: Vec<String> = images_json
                .and_then(|j| serde_json::from_str(&j).ok())
                .unwrap_or_default();
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)? as usize,
                line_end.map(|n| n as usize),
                images,
            ))
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
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
}
