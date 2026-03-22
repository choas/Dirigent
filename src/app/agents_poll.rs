use crate::agents::{self, AgentKind, AgentStatus, AgentTrigger, LastRunInfo};

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
            let kind_key = result.kind.db_key();
            let _ = self.db.insert_agent_run(
                &kind_key,
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
            self.agent_state.cancel_flags.remove(&result.kind);
            self.agent_state.last_run.insert(
                result.kind,
                LastRunInfo {
                    duration_ms: result.duration_ms,
                    finished_at: std::time::Instant::now(),
                },
            );
            self.agent_state
                .latest_output
                .insert(result.kind, result.output);
            self.agent_state
                .latest_diagnostics
                .insert(result.kind, result.diagnostics);

            // Status bar message + Activity log for associated cue
            let label = self
                .settings
                .agents
                .iter()
                .find(|a| a.kind == result.kind)
                .map(|a| a.display_name().to_string())
                .unwrap_or_else(|| result.kind.label().to_string());
            let dur = if result.duration_ms < 1000 {
                format!("{}ms", result.duration_ms)
            } else {
                format!("{:.1}s", result.duration_ms as f64 / 1000.0)
            };
            match result.status {
                AgentStatus::Passed => {
                    self.set_status_message(format!("{} passed ({})", label, dur));

                    // After format passes, reload open tabs to show reformatted code
                    if result.kind == AgentKind::Format {
                        for tab in &mut self.viewer.tabs {
                            if let Ok(content) = std::fs::read_to_string(&tab.file_path) {
                                tab.content = content.lines().map(String::from).collect();
                                let ext = tab
                                    .file_path
                                    .extension()
                                    .and_then(|e| e.to_str())
                                    .unwrap_or("");
                                tab.symbols = super::symbols::parse_symbols(&tab.content, ext);
                            }
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

            // Chain: trigger any agents configured with AfterAgent(<this agent>)
            self.trigger_agents_for(&AgentTrigger::AfterAgent(result.kind), result.cue_id, "");
        }
    }

    /// Trigger all agents matching the given trigger type.
    pub(super) fn trigger_agents_for(
        &mut self,
        trigger: &AgentTrigger,
        cue_id: Option<i64>,
        prompt: &str,
    ) {
        agents::trigger_agents(
            &self.settings.agents,
            trigger,
            &self.project_root,
            &self.settings.agent_shell_init,
            cue_id,
            prompt,
            &self.agent_state.tx,
            &mut self.agent_state.statuses,
            &mut self.agent_state.cancel_flags,
        );
    }

    /// Manually trigger a specific agent kind.
    pub(super) fn trigger_agent_manual(&mut self, kind: AgentKind) {
        if let Some(config) = self.settings.agents.iter().find(|a| a.kind == kind) {
            if self.agent_state.statuses.get(&kind) == Some(&AgentStatus::Running) {
                return; // Already running
            }
            self.agent_state.statuses.insert(kind, AgentStatus::Running);
            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            self.agent_state
                .cancel_flags
                .insert(kind, std::sync::Arc::clone(&cancel));

            let config = config.clone();
            let root = self.project_root.clone();
            let init = self.settings.agent_shell_init.clone();
            let tx = self.agent_state.tx.clone();

            std::thread::spawn(move || {
                agents::run_agent(&config, &root, &init, None, "", &tx, &cancel);
            });
        }
    }

    /// Cancel a running agent.
    pub(super) fn cancel_agent(&mut self, kind: AgentKind) {
        if let Some(flag) = self.agent_state.cancel_flags.get(&kind) {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}
