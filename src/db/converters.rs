use crate::settings::CliProvider;

use super::types::{Cue, CueStatus, Execution, ExecutionStatus};

pub(super) fn row_to_cue(row: &rusqlite::Row) -> rusqlite::Result<Cue> {
    let status_str: String = row.get(5)?;
    let line_end: Option<i64> = row.get(4)?;
    let images_json: Option<String> = row.get(8)?;
    let attached_images: Vec<String> = images_json
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    Ok(Cue {
        id: row.get(0)?,
        text: row.get(1)?,
        file_path: row.get(2)?,
        line_number: row.get::<_, i64>(3)? as usize,
        line_number_end: line_end.map(|n| n as usize),
        status: CueStatus::from_str(&status_str).unwrap_or(CueStatus::Inbox),
        source_label: row.get(6)?,
        source_ref: row.get(7)?,
        attached_images,
        tag: row.get(9)?,
    })
}

pub(super) fn row_to_execution(row: &rusqlite::Row) -> rusqlite::Result<Execution> {
    let status_str: String = row.get(6)?;
    let provider_str: String = row.get(7)?;
    let provider = match provider_str.as_str() {
        "OpenCode" => CliProvider::OpenCode,
        _ => CliProvider::Claude,
    };
    let duration_raw: Option<i64> = row.get(9)?;
    let turns_raw: Option<i64> = row.get(10)?;
    Ok(Execution {
        id: row.get(0)?,
        cue_id: row.get(1)?,
        prompt: row.get(2)?,
        response: row.get(3)?,
        diff: row.get(4)?,
        log: row.get(5)?,
        status: ExecutionStatus::from_str(&status_str).unwrap_or(ExecutionStatus::Pending),
        provider,
        cost_usd: row.get(8)?,
        duration_ms: duration_raw.map(|v| v as u64),
        num_turns: turns_raw.map(|v| v as u64),
    })
}
