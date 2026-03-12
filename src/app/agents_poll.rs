use crate::agents::{self, AgentKind, AgentStatus, AgentTrigger};

use super::DirigentApp;

impl DirigentApp {
    /// Drain the agent result channel and update state.
    pub(super) fn process_agent_results(&mut self) {
        let results: Vec<agents::AgentResult> = self.agent_state.rx.try_iter().collect();

        for result in results {
            // Store result in DB
            let diagnostics_json = if result.diagnostics.is_empty() {
                None
            } else {
                serde_json::to_string(&result.diagnostics).ok()
            };
            let _ = self.db.insert_agent_run(
                result.kind.as_str(),
                result.cue_id,
                &self
                    .settings
                    .agents
                    .iter()
                    .find(|a| a.kind == result.kind)
                    .map(|a| a.command.as_str())
                    .unwrap_or(""),
                result.status.as_str(),
                &result.output,
                diagnostics_json.as_deref(),
                result.duration_ms,
            );

            // Update runtime state
            self.agent_state.statuses.insert(result.kind, result.status);
            self.agent_state
                .latest_output
                .insert(result.kind, result.output);
            self.agent_state
                .latest_diagnostics
                .insert(result.kind, result.diagnostics);

            // Status bar message + Activity log for associated cue
            let label = result.kind.label();
            let dur = if result.duration_ms < 1000 {
                format!("{}ms", result.duration_ms)
            } else {
                format!("{:.1}s", result.duration_ms as f64 / 1000.0)
            };
            match result.status {
                AgentStatus::Passed => {
                    self.set_status_message(format!("{} passed ({})", label, dur));

                    // After format passes, reload the current file to show reformatted code
                    if result.kind == AgentKind::Format {
                        if let Some(ref path) = self.viewer.current_file {
                            let p = path.clone();
                            self.load_file(p);
                        }
                        self.reload_git_info();
                    }
                }
                AgentStatus::Failed => {
                    self.set_status_message(format!("{} failed", label));
                }
                AgentStatus::Error => {
                    self.set_status_message(format!("{} error", label));
                }
                _ => {}
            }

            // Log agent outcome to the cue's Activity logbook
            if let Some(cue_id) = result.cue_id {
                let event = match result.status {
                    AgentStatus::Passed => format!("{} passed ({})", label, dur),
                    AgentStatus::Failed => {
                        // Include first line of output as a hint
                        let hint = self
                            .agent_state
                            .latest_output
                            .get(&result.kind)
                            .and_then(|o| {
                                o.lines()
                                    .find(|l| !l.trim().is_empty())
                                    .map(|l| l.chars().take(80).collect::<String>())
                            })
                            .unwrap_or_default();
                        if hint.is_empty() {
                            format!("{} failed ({})", label, dur)
                        } else {
                            format!("{} failed ({}) — {}", label, dur, hint)
                        }
                    }
                    AgentStatus::Error => format!("{} error ({})", label, dur),
                    _ => format!("{} completed ({})", label, dur),
                };
                let _ = self.db.log_activity(cue_id, &event);
            }
        }
    }

    /// Trigger all agents matching the given trigger type.
    pub(super) fn trigger_agents_for(&mut self, trigger: &AgentTrigger, cue_id: Option<i64>) {
        agents::trigger_agents(
            &self.settings.agents,
            trigger,
            &self.project_root,
            cue_id,
            &self.agent_state.tx,
            &mut self.agent_state.statuses,
        );
    }

    /// Manually trigger a specific agent kind.
    pub(super) fn trigger_agent_manual(&mut self, kind: AgentKind) {
        if let Some(config) = self.settings.agents.iter().find(|a| a.kind == kind) {
            if self.agent_state.statuses.get(&kind) == Some(&AgentStatus::Running) {
                return; // Already running
            }
            self.agent_state.statuses.insert(kind, AgentStatus::Running);

            let config = config.clone();
            let root = self.project_root.clone();
            let tx = self.agent_state.tx.clone();

            std::thread::spawn(move || {
                agents::run_agent(&config, &root, None, &tx);
            });
        }
    }
}
