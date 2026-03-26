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
    /// File paths that Claude edited (from Edit/Write tool_use events).
    pub edited_files: Vec<String>,
    /// Run metrics extracted from the stream-json "result" event.
    pub metrics: RunMetrics,
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
