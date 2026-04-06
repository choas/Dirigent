use serde::{Deserialize, Serialize};

use crate::settings::CliProvider;

/// (cue_id, text, file_path, line_number, line_number_end, attached_images)
pub(crate) type CueHistoryRow = (i64, String, String, usize, Option<usize>, Vec<String>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CueStatus {
    Inbox,
    Ready,
    Review,
    Done,
    Archived,
    Backlog,
}

impl CueStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CueStatus::Inbox => "inbox",
            CueStatus::Ready => "ready",
            CueStatus::Review => "review",
            CueStatus::Done => "done",
            CueStatus::Archived => "archived",
            CueStatus::Backlog => "backlog",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "inbox" => Some(CueStatus::Inbox),
            "ready" => Some(CueStatus::Ready),
            "review" => Some(CueStatus::Review),
            "done" => Some(CueStatus::Done),
            "archived" => Some(CueStatus::Archived),
            "backlog" => Some(CueStatus::Backlog),
            _ => None,
        }
    }

    pub fn all() -> &'static [CueStatus] {
        &[
            CueStatus::Inbox,
            CueStatus::Ready,
            CueStatus::Review,
            CueStatus::Done,
            CueStatus::Archived,
            CueStatus::Backlog,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            CueStatus::Inbox => "Inbox",
            CueStatus::Ready => "Running",
            CueStatus::Review => "Review",
            CueStatus::Done => "Done",
            CueStatus::Archived => "Archived",
            CueStatus::Backlog => "Backlog",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ExecutionStatus {
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
pub(crate) struct Cue {
    pub id: i64,
    pub text: String,
    pub file_path: String,
    pub line_number: usize,
    pub line_number_end: Option<usize>,
    pub status: CueStatus,
    pub source_label: Option<String>,
    pub source_id: Option<String>,
    pub source_ref: Option<String>,
    /// Attached image file paths (stored as JSON array in DB).
    pub attached_images: Vec<String>,
    /// Optional user-assigned tag for grouping/labeling cues.
    pub tag: Option<String>,
    /// Path to a Claude Code plan file (set when ExitPlanMode is detected in output).
    pub plan_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Execution {
    pub id: i64,
    #[allow(dead_code)]
    pub cue_id: i64,
    pub prompt: String,
    pub response: Option<String>,
    pub diff: Option<String>,
    pub log: Option<String>,
    #[allow(dead_code)]
    pub status: ExecutionStatus,
    pub provider: CliProvider,
    /// Cost in USD (from Claude stream-json).
    pub cost_usd: Option<f64>,
    /// Duration in milliseconds.
    pub duration_ms: Option<u64>,
    /// Number of conversation turns.
    pub num_turns: Option<u64>,
}

/// Lightweight execution metrics for display in cue cards (avoids fetching full Execution blobs).
#[derive(Debug, Clone)]
pub(crate) struct ExecutionMetrics {
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    pub num_turns: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct ActivityEntry {
    pub timestamp: String,
    pub event: String,
}
