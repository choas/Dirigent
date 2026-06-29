/// Error type for Claude CLI operations.
#[derive(Debug)]
pub(crate) enum ClaudeError {
    NotFound,
    SpawnFailed(std::io::Error),
    Cancelled,
}

impl std::error::Error for ClaudeError {}

impl std::fmt::Display for ClaudeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClaudeError::NotFound => write!(f, "claude CLI not found on PATH"),
            ClaudeError::SpawnFailed(e) => write!(f, "failed to spawn claude: {e}"),
            ClaudeError::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ClaudeResponse {
    pub stdout: String,
    pub metrics: RunMetrics,
    pub metadata: Option<ClaudeRunMetadata>,
}

/// Cost and performance metrics from a Claude run.
#[derive(Debug, Clone, Default)]
pub(crate) struct RunMetrics {
    pub cost_usd: f64,
    pub duration_ms: u64,
    pub num_turns: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct ClaudeRunMetadata {
    pub completion_reason: Option<String>,
    pub stop_hook: Option<claude_pty::StopHookSummary>,
    pub permission_summaries: Vec<String>,
    pub parser_warnings: Vec<String>,
    pub pty_rows: Option<u16>,
    pub pty_cols: Option<u16>,
    pub prompt_submitted_ms: Option<u64>,
    pub first_output_ms: Option<u64>,
    pub last_output_ms: Option<u64>,
}
