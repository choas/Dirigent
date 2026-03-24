use crate::agents::{self, AgentKind, AgentResult, AgentStatus, AgentTrigger, LastRunInfo};
use crate::db::AgentRunRecord;

use super::DirigentApp;

impl DirigentApp {
    /// Drain the agent result channel and update state.
    pub(super) fn process_agent_results(&mut self) {
        let results: Vec<agents::AgentResult> = self.agent_state.rx.try_iter().collect();

        for result in results {
            self.process_single_agent_result(result);
        }
    }

    /// Process a single agent result: persist to DB, update state, set status
    /// messages, log activity, and trigger chained agents.
    fn process_single_agent_result(&mut self, result: AgentResult) {
        self.persist_agent_result(&result);
        self.update_agent_runtime_state(&result);

        let label = self.agent_display_label(result.kind);
        let dur = super::util::format_duration_ms(result.duration_ms);

        self.set_agent_status_message(&result, &label, &dur);
        self.log_agent_activity(&result, &label, &dur);

        // Chain: trigger any agents configured with AfterAgent(<this agent>)
        self.trigger_agents_for(&AgentTrigger::AfterAgent(result.kind), result.cue_id, "");
    }

    /// Store an agent result in the database.
    fn persist_agent_result(&self, result: &AgentResult) {
        let diagnostics_json = if result.diagnostics.is_empty() {
            None
        } else {
            serde_json::to_string(&result.diagnostics).ok()
        };
        let kind_key = result.kind.db_key();
        let command = self
            .settings
            .agents
            .iter()
            .find(|a| a.kind == result.kind)
            .map(|a| a.command.as_str())
            .unwrap_or("");
        let _ = self.db.insert_agent_run(&AgentRunRecord {
            agent_kind: &kind_key,
            cue_id: result.cue_id,
            command,
            status: result.status.as_str(),
            output: &result.output,
            diagnostics_json: diagnostics_json.as_deref(),
            duration_ms: result.duration_ms,
        });
    }

    /// Update in-memory agent runtime state from a completed result.
    fn update_agent_runtime_state(&mut self, result: &AgentResult) {
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
            .insert(result.kind, result.output.clone());
        self.agent_state
            .latest_diagnostics
            .insert(result.kind, result.diagnostics.clone());
    }

    /// Get the human-readable display label for an agent kind.
    fn agent_display_label(&self, kind: AgentKind) -> String {
        self.settings
            .agents
            .iter()
            .find(|a| a.kind == kind)
            .map(|a| a.display_name().to_string())
            .unwrap_or_else(|| kind.label().to_string())
    }

    /// Set a status bar message based on the agent result.
    fn set_agent_status_message(&mut self, result: &AgentResult, label: &str, dur: &str) {
        match result.status {
            AgentStatus::Passed => {
                self.set_status_message(format!("{} passed ({})", label, dur));
                if result.kind == AgentKind::Format {
                    self.reload_tabs_after_format();
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
    }

    /// After a format agent passes, reload open tabs to show reformatted code.
    fn reload_tabs_after_format(&mut self) {
        for tab in &mut self.viewer.tabs {
            if let Ok(content) = std::fs::read_to_string(&tab.file_path) {
                if tab.markdown_blocks.is_some() {
                    tab.markdown_blocks = Some(super::markdown_parser::parse_markdown(&content));
                }
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

    /// Log the agent outcome to the cue's Activity logbook (if a cue is associated).
    fn log_agent_activity(&self, result: &AgentResult, label: &str, dur: &str) {
        let Some(cue_id) = result.cue_id else {
            return;
        };
        let event = match result.status {
            AgentStatus::Passed => format!("{} passed ({})", label, dur),
            AgentStatus::Failed => {
                let hint = self.get_first_nonempty_output_line(result.kind);
                if hint.is_empty() {
                    format!("{} failed ({})", label, dur)
                } else {
                    format!("{} failed ({}) \u{2014} {}", label, dur, hint)
                }
            }
            AgentStatus::Error => format!("{} error ({})", label, dur),
            _ => format!("{} completed ({})", label, dur),
        };
        let _ = self.db.log_activity(cue_id, &event);
    }

    /// Get the first non-empty line of the latest output for an agent, truncated to 80 chars.
    fn get_first_nonempty_output_line(&self, kind: AgentKind) -> String {
        self.agent_state
            .latest_output
            .get(&kind)
            .and_then(|o| {
                o.lines()
                    .find(|l| !l.trim().is_empty())
                    .map(|l| l.chars().take(80).collect::<String>())
            })
            .unwrap_or_default()
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
