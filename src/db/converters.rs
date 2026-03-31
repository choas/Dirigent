use crate::settings::CliProvider;

use super::types::{Cue, CueStatus, Execution, ExecutionStatus};

fn try_i64_to_usize(v: i64) -> rusqlite::Result<usize> {
    usize::try_from(v).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            format!("negative integer {v} cannot convert to usize").into(),
        )
    })
}

fn try_i64_to_u64(v: i64) -> rusqlite::Result<u64> {
    u64::try_from(v).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            format!("negative integer {v} cannot convert to u64").into(),
        )
    })
}

// Cue column indices — keep in sync with CUE_COLUMNS.
const COL_ID: usize = 0;
const COL_TEXT: usize = 1;
const COL_FILE_PATH: usize = 2;
const COL_LINE_NUMBER: usize = 3;
const COL_LINE_NUMBER_END: usize = 4;
const COL_STATUS: usize = 5;
const COL_SOURCE_LABEL: usize = 6;
const COL_SOURCE_ID: usize = 7;
const COL_SOURCE_REF: usize = 8;
const COL_ATTACHED_IMAGES: usize = 9;
const COL_TAG: usize = 10;
const COL_PLAN_PATH: usize = 11;

/// Column list for SELECT queries that feed into [`row_to_cue`].
pub(super) const CUE_COLUMNS: &str =
    "id, text, file_path, line_number, line_number_end, status, source_label, source_id, source_ref, attached_images, tag, plan_path";

pub(super) fn row_to_cue(row: &rusqlite::Row) -> rusqlite::Result<Cue> {
    let status_str: String = row.get(COL_STATUS)?;
    let line_end: Option<i64> = row.get(COL_LINE_NUMBER_END)?;
    let images_json: Option<String> = row.get(COL_ATTACHED_IMAGES)?;
    let attached_images: Vec<String> = images_json
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    Ok(Cue {
        id: row.get(COL_ID)?,
        text: row.get(COL_TEXT)?,
        file_path: row.get(COL_FILE_PATH)?,
        line_number: try_i64_to_usize(row.get::<_, i64>(COL_LINE_NUMBER)?)?,
        line_number_end: line_end.map(try_i64_to_usize).transpose()?,
        status: CueStatus::from_str(&status_str).unwrap_or(CueStatus::Inbox),
        source_label: row.get(COL_SOURCE_LABEL)?,
        source_id: row.get(COL_SOURCE_ID)?,
        source_ref: row.get(COL_SOURCE_REF)?,
        attached_images,
        tag: row.get(COL_TAG)?,
        plan_path: row.get(COL_PLAN_PATH)?,
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
        duration_ms: duration_raw.map(try_i64_to_u64).transpose()?,
        num_turns: turns_raw.map(try_i64_to_u64).transpose()?,
    })
}
