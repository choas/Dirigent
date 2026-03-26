use anyhow::Result;
use chrono::Local;
use rusqlite::params;

use super::Database;

/// Parameters for inserting a new agent run via [`Database::insert_agent_run`].
pub struct AgentRunRecord<'a> {
    pub agent_kind: &'a str,
    pub cue_id: Option<i64>,
    pub command: &'a str,
    pub status: &'a str,
    pub output: &'a str,
    pub diagnostics_json: Option<&'a str>,
    pub duration_ms: u64,
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

impl Database {
    // -- Agent runs --

    /// Record a completed agent run.
    pub fn insert_agent_run(&self, record: &AgentRunRecord<'_>) -> Result<i64> {
        let now = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        self.conn.execute(
            "INSERT INTO agent_runs (agent_kind, cue_id, command, status, output, diagnostics, duration_ms, started_at, finished_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![record.agent_kind, record.cue_id, record.command, record.status, record.output, record.diagnostics_json, record.duration_ms as i64, now],
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
