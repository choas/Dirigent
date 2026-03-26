use anyhow::Result;
use rusqlite::params;

use super::types::CueStatus;
use super::Database;

/// (id, text, status, file_path, line_number)
pub(crate) type CueSourceRow = (i64, String, String, String, usize);

impl Database {
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

    /// Get the cue id, text, status, file_path, and line_number for a given source_ref.
    pub fn get_cue_by_source_ref(&self, source_ref: &str) -> Result<Option<CueSourceRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, text, status, file_path, line_number FROM cues WHERE source_ref = ?1 LIMIT 1",
        )?;
        let result = stmt.query_row(params![source_ref], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)? as usize,
            ))
        });
        match result {
            Ok(row) => Ok(Some(row)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Update text and location of an existing cue identified by source_ref.
    pub fn update_cue_by_source_ref(
        &self,
        source_ref: &str,
        text: &str,
        file_path: &str,
        line_number: usize,
    ) -> Result<()> {
        let text = Self::clamp_cue_text(text);
        self.conn.execute(
            "UPDATE cues SET text = ?1, file_path = ?2, line_number = ?3 WHERE source_ref = ?4",
            params![text, file_path, line_number as i64, source_ref],
        )?;
        Ok(())
    }

    /// Insert a cue from an external source with optional file location.
    pub fn insert_cue_from_source(
        &self,
        text: &str,
        source_label: &str,
        source_ref: &str,
        file_path: &str,
        line_number: usize,
    ) -> Result<i64> {
        let text = Self::clamp_cue_text(text);
        self.conn.execute(
            "INSERT INTO cues (text, file_path, line_number, status, source_label, source_ref) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![text, file_path, line_number as i64, CueStatus::Inbox.as_str(), source_label, source_ref],
        )?;
        let id = self.conn.last_insert_rowid();
        let _ = self.log_activity(id, &format!("Created from {}", source_label));
        Ok(id)
    }
}
