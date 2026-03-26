use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Instant;

use super::diagnostics::Diagnostic;
use super::execution::AgentResult;
use super::types::{AgentKind, AgentStatus};

// ---------------------------------------------------------------------------
// Agent run state (lives inside DirigentApp)
// ---------------------------------------------------------------------------

/// Info about the most recent completed run for an agent.
pub(crate) struct LastRunInfo {
    pub duration_ms: u64,
    pub finished_at: Instant,
}

pub(crate) struct AgentRunState {
    pub tx: mpsc::Sender<AgentResult>,
    pub rx: mpsc::Receiver<AgentResult>,
    /// Latest status per agent kind (for status bar display).
    pub statuses: HashMap<AgentKind, AgentStatus>,
    /// Latest output per agent kind (for detail panel).
    pub latest_output: HashMap<AgentKind, String>,
    /// Latest diagnostics per agent kind.
    pub latest_diagnostics: HashMap<AgentKind, Vec<Diagnostic>>,
    /// Info about the last completed run per agent kind.
    pub last_run: HashMap<AgentKind, LastRunInfo>,
    /// Cancel flags for running agents.
    pub cancel_flags: HashMap<AgentKind, Arc<AtomicBool>>,
    /// Which agent's output panel is currently shown (None = hidden).
    pub show_output: Option<AgentKind>,
    /// When true, the Back button in agent log returns to settings.
    pub return_to_settings: bool,
}

impl AgentRunState {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        AgentRunState {
            tx,
            rx,
            statuses: HashMap::new(),
            latest_output: HashMap::new(),
            latest_diagnostics: HashMap::new(),
            last_run: HashMap::new(),
            cancel_flags: HashMap::new(),
            show_output: None,
            return_to_settings: false,
        }
    }
}
