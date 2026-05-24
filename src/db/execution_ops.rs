use anyhow::Result;
use rusqlite::params;

use crate::settings::CliProvider;

use super::converters::row_to_execution;
use super::types::{Execution, ExecutionMetrics, ExecutionStatus};
use super::Database;

impl Database {
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

    pub fn complete_execution(
        &self,
        id: i64,
        response: &str,
        diff: Option<&str>,
        cost_usd: Option<f64>,
        duration_ms: Option<u64>,
        num_turns: Option<u64>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE executions SET response = ?1, diff = ?2, status = ?3, \
             cost_usd = ?5, duration_ms = ?6, num_turns = ?7 WHERE id = ?4",
            params![
                response,
                diff,
                ExecutionStatus::Completed.as_str(),
                id,
                cost_usd,
                duration_ms.map(|v| v as i64),
                num_turns.map(|v| v as i64),
            ],
        )?;
        Ok(())
    }

    /// Store run metrics (cost, duration, tokens, turns) for an execution.
    pub fn update_execution_metrics(
        &self,
        id: i64,
        cost_usd: f64,
        duration_ms: u64,
        num_turns: u64,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE executions SET cost_usd = ?1, duration_ms = ?2, num_turns = ?3, input_tokens = ?4, output_tokens = ?5 WHERE id = ?6",
            params![cost_usd, duration_ms as i64, num_turns as i64, input_tokens as i64, output_tokens as i64, id],
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

    /// Get the total cost across all executions in this project.
    pub fn total_cost(&self) -> Result<f64> {
        let cost: f64 = self.conn.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0.0) FROM executions",
            [],
            |row| row.get(0),
        )?;
        Ok(cost)
    }

    pub fn get_latest_execution(&self, cue_id: i64) -> Result<Option<Execution>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, cue_id, prompt, response, diff, log, status, provider, cost_usd, duration_ms, num_turns, session_id FROM executions WHERE cue_id = ?1 ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![cue_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_execution(row)?))
        } else {
            Ok(None)
        }
    }

    /// Get the latest execution metrics for every cue that has at least one execution.
    /// Returns a map from cue_id to its latest ExecutionMetrics.
    pub fn get_all_latest_execution_metrics(
        &self,
    ) -> Result<std::collections::HashMap<i64, ExecutionMetrics>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.cue_id, e.cost_usd, e.duration_ms, e.num_turns \
             FROM executions e \
             INNER JOIN (SELECT cue_id, MAX(id) AS max_id FROM executions GROUP BY cue_id) latest \
             ON e.id = latest.max_id",
        )?;
        let mut map = std::collections::HashMap::new();
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let cue_id: i64 = row.get(0)?;
            map.insert(
                cue_id,
                ExecutionMetrics {
                    cost_usd: row.get(1)?,
                    duration_ms: row.get::<_, Option<i64>>(2)?.map(|v| v as u64),
                    num_turns: row.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                },
            );
        }
        Ok(map)
    }

    pub fn update_execution_session_id(&self, id: i64, session_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE executions SET session_id = ?1 WHERE id = ?2",
            params![session_id, id],
        )?;
        Ok(())
    }

    /// Get the Claude session ID from the latest execution for a cue.
    pub fn get_cue_session_id(&self, cue_id: i64) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id FROM executions WHERE cue_id = ?1 AND session_id IS NOT NULL ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![cue_id])?;
        if let Some(row) = rows.next()? {
            Ok(row.get(0)?)
        } else {
            Ok(None)
        }
    }

    /// Get all executions for a cue, ordered by id (oldest first).
    pub fn get_all_executions(&self, cue_id: i64) -> Result<Vec<Execution>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, cue_id, prompt, response, diff, log, status, provider, cost_usd, duration_ms, num_turns, session_id FROM executions WHERE cue_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![cue_id], row_to_execution)?;
        let mut execs = Vec::new();
        for row in rows {
            execs.push(row?);
        }
        Ok(execs)
    }
}
