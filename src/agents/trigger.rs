use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;

use super::execution::{run_agent, AgentResult};
use super::types::{AgentConfig, AgentKind, AgentStatus, AgentTrigger};

// ---------------------------------------------------------------------------
// Trigger agents matching a given trigger condition
// ---------------------------------------------------------------------------

/// Spawn agents that match the given trigger. Returns the number of agents started.
#[allow(clippy::too_many_arguments)]
pub(crate) fn trigger_agents(
    agents: &[AgentConfig],
    trigger: &AgentTrigger,
    project_root: &Path,
    shell_init: &str,
    cue_id: Option<i64>,
    prompt: &str,
    tx: &mpsc::Sender<AgentResult>,
    statuses: &mut HashMap<AgentKind, AgentStatus>,
    cancel_flags: &mut HashMap<AgentKind, Arc<AtomicBool>>,
) -> usize {
    let mut count = 0;
    for config in agents {
        if !config.enabled {
            continue;
        }
        let matches = match (&config.trigger, trigger) {
            (AgentTrigger::AfterAgent(k1), AgentTrigger::AfterAgent(k2)) => k1 == k2,
            (a, b) => a == b,
        };
        if !matches {
            continue;
        }
        // Don't start an agent that's already running
        if statuses.get(&config.kind) == Some(&AgentStatus::Running) {
            continue;
        }
        statuses.insert(config.kind, AgentStatus::Running);
        let cancel = Arc::new(AtomicBool::new(false));
        cancel_flags.insert(config.kind, Arc::clone(&cancel));

        let config = config.clone();
        let root = project_root.to_path_buf();
        let init = shell_init.to_string();
        let prompt = prompt.to_string();
        let tx = tx.clone();

        std::thread::spawn(move || {
            run_agent(&config, &root, &init, cue_id, &prompt, &tx, &cancel);
        });

        count += 1;
    }
    count
}
