use std::sync::mpsc;

use crate::db::{Cue, CueStatus};
use crate::workflow::{self, WorkflowPlan, WorkflowStepStatus};

use super::DirigentApp;

impl DirigentApp {
    /// Trigger LLM analysis of Inbox cues to create a workflow plan.
    pub(super) fn create_workflow(&mut self) {
        if self.workflow_generating {
            return;
        }

        let inbox_cues: Vec<Cue> = self.inbox_cues_for_workflow();
        if inbox_cues.len() < 2 {
            self.set_status_message("Need at least 2 Inbox cues for a workflow".into());
            return;
        }

        self.workflow_generating = true;
        let prompt = workflow::build_workflow_prompt(&inbox_cues);
        let expected_ids: Vec<i64> = inbox_cues.iter().map(|c| c.id).collect();

        let provider = self.settings.cli_provider.clone();
        let project_root = self.project_root.clone();
        let settings = self.settings.clone();

        let (tx, rx) = mpsc::channel();
        self.workflow_rx = Some(rx);

        std::thread::spawn(move || {
            let result =
                run_workflow_analysis(&prompt, &expected_ids, &provider, &project_root, &settings);
            let _ = tx.send(result);
        });

        self.set_status_message("Analyzing cues for workflow plan...".into());
    }

    /// Poll for workflow analysis results.
    pub(super) fn process_workflow_result(&mut self) {
        let rx = match self.workflow_rx {
            Some(ref rx) => rx,
            None => return,
        };
        let result = match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.workflow_generating = false;
                self.workflow_rx = None;
                self.set_status_message("Workflow analysis failed unexpectedly".into());
                return;
            }
            Ok(r) => r,
        };
        self.workflow_generating = false;
        self.workflow_rx = None;
        match result {
            Ok(plan) => {
                let step_count = plan.steps.len();
                let cue_count: usize = plan.steps.iter().map(|s| s.cue_ids.len()).sum();
                self.set_status_message(format!(
                    "Workflow plan ready: {} steps, {} cues",
                    step_count, cue_count
                ));
                self.workflow_plan = Some(plan);
            }
            Err(e) => {
                self.set_status_message(format!("Workflow analysis failed: {}", e));
            }
        }
    }

    /// Start executing the workflow plan from the first step.
    pub(super) fn start_workflow(&mut self) {
        let plan = match self.workflow_plan.as_mut() {
            Some(p) => p,
            None => return,
        };
        if plan.steps.is_empty() {
            return;
        }
        plan.current_step = 0;
        plan.steps[0].status = WorkflowStepStatus::Running;

        // Collect cue IDs to trigger (can't borrow self mutably while iterating plan)
        let cue_ids: Vec<i64> = plan.steps[0].cue_ids.clone();

        for &cue_id in &cue_ids {
            // Move cue to Ready and trigger
            let _ = self.db.update_cue_status(cue_id, CueStatus::Ready);
            let _ = self.db.log_activity(cue_id, "Workflow: started (step 0)");
            self.cue_move_flash
                .insert(cue_id, std::time::Instant::now());
        }
        self.claude.expand_running = true;
        self.reload_cues();

        // Trigger Claude for each cue in parallel (up to concurrency limit of 3)
        for &cue_id in &cue_ids {
            self.trigger_claude(cue_id);
        }
    }

    /// Called when a cue completes. Checks if the current workflow step is done.
    pub(super) fn on_workflow_cue_completed(&mut self, cue_id: i64) {
        let plan = match self.workflow_plan.as_mut() {
            Some(p) => p,
            None => return,
        };

        // Find which step this cue belongs to
        let step_idx = match plan.steps.iter().position(|s| s.cue_ids.contains(&cue_id)) {
            Some(i) => i,
            None => return,
        };

        if plan.steps[step_idx].status != WorkflowStepStatus::Running {
            return;
        }

        // Check if all cues in this step are done (no longer in Ready status)
        let all_done = plan.steps[step_idx].cue_ids.iter().all(|&id| {
            self.cues
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.status != CueStatus::Ready)
                .unwrap_or(true)
        });

        if !all_done {
            return;
        }

        // Check if any cue in this step failed (went back to Inbox)
        let any_failed = plan.steps[step_idx].cue_ids.iter().any(|&id| {
            self.cues
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.status == CueStatus::Inbox)
                .unwrap_or(false)
        });

        if any_failed {
            plan.steps[step_idx].status = WorkflowStepStatus::Failed;
            self.set_status_message(format!("Workflow step {} failed", step_idx));
            return;
        }

        // Step completed successfully
        if plan.steps[step_idx].pause_after {
            plan.steps[step_idx].status = WorkflowStepStatus::PausedAwaitingReview;
            self.set_status_message(format!(
                "Workflow paused after step {} — review before continuing",
                step_idx
            ));
            return;
        }

        plan.steps[step_idx].status = WorkflowStepStatus::Completed;
        self.advance_workflow();
    }

    /// Advance to the next pending step in the workflow.
    fn advance_workflow(&mut self) {
        let plan = match self.workflow_plan.as_mut() {
            Some(p) => p,
            None => return,
        };

        // Find the next pending step
        let next_idx = plan
            .steps
            .iter()
            .position(|s| s.status == WorkflowStepStatus::Pending);

        match next_idx {
            Some(idx) => {
                plan.current_step = idx;
                plan.steps[idx].status = WorkflowStepStatus::Running;
                let cue_ids: Vec<i64> = plan.steps[idx].cue_ids.clone();

                for &cue_id in &cue_ids {
                    let _ = self.db.update_cue_status(cue_id, CueStatus::Ready);
                    let _ = self
                        .db
                        .log_activity(cue_id, &format!("Workflow: started (step {})", idx));
                    self.cue_move_flash
                        .insert(cue_id, std::time::Instant::now());
                }
                self.claude.expand_running = true;
                self.reload_cues();

                for &cue_id in &cue_ids {
                    self.trigger_claude(cue_id);
                }
            }
            None => {
                // All steps done
                self.set_status_message("Workflow completed!".into());
            }
        }
    }

    /// Resume a paused workflow.
    pub(super) fn resume_workflow(&mut self) {
        let plan = match self.workflow_plan.as_mut() {
            Some(p) => p,
            None => return,
        };

        // Find the paused step and mark it completed
        if let Some(step) = plan
            .steps
            .iter_mut()
            .find(|s| s.status == WorkflowStepStatus::PausedAwaitingReview)
        {
            step.status = WorkflowStepStatus::Completed;
        }

        self.advance_workflow();
    }

    /// Cancel the current workflow.
    pub(super) fn cancel_workflow(&mut self) {
        // Collect cue IDs to cancel before mutating self.
        let cue_ids_to_cancel: Vec<i64> = self
            .workflow_plan
            .as_ref()
            .map(|plan| {
                plan.steps
                    .iter()
                    .filter(|s| s.status == WorkflowStepStatus::Running)
                    .flat_map(|s| s.cue_ids.iter().copied())
                    .collect()
            })
            .unwrap_or_default();

        for cue_id in cue_ids_to_cancel {
            self.cancel_cue_task(cue_id);
            let _ = self.db.update_cue_status(cue_id, CueStatus::Inbox);
            let _ = self.db.log_activity(cue_id, "Workflow cancelled");
        }
        self.workflow_plan = None;
        self.workflow_warning = None;
        self.reload_cues();
        self.set_status_message("Workflow cancelled".into());
    }

    /// Toggle pause_after on a specific step.
    pub(super) fn toggle_workflow_pause(&mut self, step_idx: usize) {
        if let Some(ref mut plan) = self.workflow_plan {
            if let Some(step) = plan.steps.get_mut(step_idx) {
                step.pause_after = !step.pause_after;
            }
        }
    }

    /// Remove a cue from the workflow plan.
    pub(super) fn remove_from_workflow(&mut self, cue_id: i64) {
        if let Some(ref mut plan) = self.workflow_plan {
            for step in &mut plan.steps {
                step.cue_ids.retain(|&id| id != cue_id);
            }
            // Remove empty steps and re-index
            plan.steps.retain(|s| !s.cue_ids.is_empty());
            for (i, step) in plan.steps.iter_mut().enumerate() {
                step.id = i;
            }
            // If plan becomes empty, clear it
            if plan.steps.is_empty() {
                self.workflow_plan = None;
                self.workflow_warning = None;
                self.set_status_message("Workflow cleared (no cues remaining)".into());
            }
        }
    }

    /// Get Inbox cues for workflow analysis, respecting source filter.
    fn inbox_cues_for_workflow(&self) -> Vec<Cue> {
        self.cues
            .iter()
            .filter(|c| {
                if c.status != CueStatus::Inbox {
                    return false;
                }
                if let Some(ref filter) = self.sources.filter {
                    c.source_label.as_deref() == Some(filter.as_str())
                } else {
                    true
                }
            })
            .cloned()
            .collect()
    }

    /// Check if the workflow is currently executing (has Running or Paused steps).
    pub(super) fn is_workflow_active(&self) -> bool {
        self.workflow_plan.as_ref().map_or(false, |p| {
            p.steps.iter().any(|s| {
                matches!(
                    s.status,
                    WorkflowStepStatus::Running | WorkflowStepStatus::PausedAwaitingReview
                )
            })
        })
    }
}

/// Run the workflow analysis using the configured provider.
fn run_workflow_analysis(
    prompt: &str,
    expected_ids: &[i64],
    provider: &crate::settings::CliProvider,
    project_root: &std::path::Path,
    settings: &crate::settings::Settings,
) -> Result<WorkflowPlan, String> {
    use crate::settings::CliProvider;

    let response_text = match provider {
        CliProvider::Claude => {
            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let result = crate::claude::invoke_claude_streaming(
                prompt,
                project_root,
                &settings.claude_model,
                &settings.claude_cli_path,
                &settings.claude_extra_args,
                &settings.claude_env_vars,
                &settings.claude_pre_run_script,
                &settings.claude_post_run_script,
                settings.allow_dangerous_skip_permissions,
                |_| {},
                cancel,
            )
            .map_err(|e| format!("Claude invocation failed: {}", e))?;
            result.stdout
        }
        CliProvider::OpenCode => {
            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let config = crate::opencode::OpenCodeRunConfig {
                model: &settings.opencode_model,
                cli_path: &settings.opencode_cli_path,
                extra_args: &settings.opencode_extra_args,
                env_vars: &settings.opencode_env_vars,
                pre_run_script: &settings.opencode_pre_run_script,
                post_run_script: &settings.opencode_post_run_script,
            };
            let result = crate::opencode::invoke_opencode_streaming(
                prompt,
                project_root,
                &config,
                |_| {},
                cancel,
            )
            .map_err(|e| format!("OpenCode invocation failed: {}", e))?;
            result.stdout
        }
    };

    let (plan, warning) = workflow::parse_workflow_response(&response_text, expected_ids);
    if let Some(ref w) = warning {
        eprintln!("[workflow] Warning: {}", w);
    }
    Ok(plan)
}
