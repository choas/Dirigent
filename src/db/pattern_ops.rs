use anyhow::Result;
use chrono::Local;
use rusqlite::params;

use super::Database;

/// A persisted PR filter pattern.
#[derive(Debug, Clone)]
pub(crate) struct PrFilterPattern {
    pub id: i64,
    pub pattern: String,
    /// Which field to match: "text" or "file_path".
    pub match_field: String,
}

impl Database {
    /// Insert a new PR filter pattern. Returns the new row id.
    pub fn insert_pr_filter_pattern(&self, pattern: &str, match_field: &str) -> Result<i64> {
        let now = Local::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO pr_filter_patterns (pattern, match_field, created_at) VALUES (?1, ?2, ?3)",
            params![pattern, match_field, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// List all PR filter patterns.
    pub fn list_pr_filter_patterns(&self) -> Result<Vec<PrFilterPattern>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, pattern, match_field FROM pr_filter_patterns ORDER BY id")?;
        let rows = stmt.query_map([], |row| {
            Ok(PrFilterPattern {
                id: row.get(0)?,
                pattern: row.get(1)?,
                match_field: row.get(2)?,
            })
        })?;
        let mut patterns = Vec::new();
        for row in rows {
            patterns.push(row?);
        }
        Ok(patterns)
    }

    /// Update the pattern text and match field for an existing pattern.
    pub fn update_pr_filter_pattern(
        &self,
        id: i64,
        pattern: &str,
        match_field: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE pr_filter_patterns SET pattern = ?1, match_field = ?2 WHERE id = ?3",
            params![pattern, match_field, id],
        )?;
        Ok(())
    }

    /// Delete a PR filter pattern by id.
    pub fn delete_pr_filter_pattern(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM pr_filter_patterns WHERE id = ?1", params![id])?;
        Ok(())
    }
}
