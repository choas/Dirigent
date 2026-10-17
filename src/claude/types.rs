/// Cost and performance metrics from a Claude run.
#[derive(Debug, Clone, Default)]
/// Metrics for one completed run.
///
/// Everything except `duration_ms` is optional, and `None` means *the backend
/// did not report it* — not zero. Collapsing the two used to make every Claude
/// run record $0.00 / 0 turns, which the totals then summed as fact: a run
/// whose cost is unknown must not be presented as a free run.
pub(crate) struct RunMetrics {
    pub cost_usd: Option<f64>,
    /// Always known: [`crate::ai_backend::AiBackend::invoke`] measures the wall
    /// clock when the backend's stream does not report one.
    pub duration_ms: u64,
    pub num_turns: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
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
