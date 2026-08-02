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
