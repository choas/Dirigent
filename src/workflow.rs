/// Maximum number of cues that may run in parallel within a single step.
const MAX_PARALLEL_CUES: usize = 5;

/// A workflow plan produced by LLM analysis of Inbox cues.
#[derive(Clone)]
pub(crate) struct WorkflowPlan {
    pub steps: Vec<WorkflowStep>,
    /// Index of the step currently executing (or about to execute).
    pub current_step: usize,
}

/// A single step in the workflow plan. Cues within a step run in parallel.
#[derive(Clone)]
pub(crate) struct WorkflowStep {
    pub id: usize,
    pub cue_ids: Vec<i64>,
    pub label: String,
    pub rationale: String,
    pub pause_after: bool,
    pub status: WorkflowStepStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkflowStepStatus {
    Pending,
    Running,
    PausedAwaitingReview,
    Completed,
    Failed,
}

impl WorkflowPlan {
    /// Build a workflow plan from parsed LLM JSON output.
    pub fn from_steps(steps: Vec<WorkflowStep>) -> Self {
        let mut plan = WorkflowPlan {
            steps,
            current_step: 0,
        };
        plan.enforce_parallel_limit();
        plan
    }

    /// Build a simple sequential fallback plan (one cue per step).
    pub fn sequential_fallback(cue_ids: &[i64]) -> Self {
        let steps = cue_ids
            .iter()
            .enumerate()
            .map(|(i, &cue_id)| WorkflowStep {
                id: i,
                cue_ids: vec![cue_id],
                label: String::new(),
                rationale: "Sequential fallback (LLM output was invalid)".to_string(),
                pause_after: false,
                status: WorkflowStepStatus::Pending,
            })
            .collect();
        WorkflowPlan {
            steps,
            current_step: 0,
        }
    }

    /// Split any step with more than `MAX_PARALLEL_CUES` cues into multiple
    /// consecutive steps of at most that size, preserving order and metadata.
    fn enforce_parallel_limit(&mut self) {
        let mut new_steps = Vec::with_capacity(self.steps.len());
        for step in self.steps.drain(..) {
            if step.cue_ids.len() <= MAX_PARALLEL_CUES {
                new_steps.push(step);
            } else {
                for chunk in step.cue_ids.chunks(MAX_PARALLEL_CUES) {
                    new_steps.push(WorkflowStep {
                        id: 0, // re-indexed below
                        cue_ids: chunk.to_vec(),
                        label: step.label.clone(),
                        rationale: step.rationale.clone(),
                        pause_after: false,
                        status: step.status,
                    });
                }
                // Preserve pause_after only on the last sub-step.
                if step.pause_after {
                    if let Some(last) = new_steps.last_mut() {
                        last.pause_after = true;
                    }
                }
            }
        }
        for (i, s) in new_steps.iter_mut().enumerate() {
            s.id = i;
        }
        self.steps = new_steps;
    }

    /// Check if all steps are completed.
    pub fn is_complete(&self) -> bool {
        self.steps
            .iter()
            .all(|s| s.status == WorkflowStepStatus::Completed)
    }
}

/// JSON structure returned by the LLM for workflow analysis.
#[derive(serde::Deserialize)]
struct LlmWorkflowResponse {
    steps: Vec<LlmWorkflowStep>,
}

#[derive(serde::Deserialize)]
struct LlmWorkflowStep {
    cue_ids: Vec<i64>,
    label: String,
    rationale: String,
}

/// Build the prompt that asks the LLM to analyze cues and produce an execution plan.
pub(crate) fn build_workflow_prompt(cues: &[crate::db::Cue]) -> String {
    let mut prompt = String::from(
        "Analyze these cues and determine the optimal execution order.\n\
         Group cues that can safely run in parallel (no file conflicts, independent changes).\n\
         Order groups that have dependencies (e.g., cue B modifies a file that cue A creates).\n\n\
         Cues:\n",
    );
    for cue in cues {
        prompt.push_str(&format!(
            "- ID {}: \"{}\" (file: {}, line: {})\n",
            cue.id,
            cue.text.replace('"', "'"),
            if cue.file_path.is_empty() {
                "(global)"
            } else {
                &cue.file_path
            },
            cue.line_number,
        ));
    }
    prompt.push_str(
        "\nOutput ONLY valid JSON (no markdown fences, no commentary) with this structure:\n\
         {\n  \"steps\": [\n    {\n      \"cue_ids\": [3, 7],\n      \
         \"label\": \"Independent fixes: auth bug + typo in README\",\n      \
         \"rationale\": \"These touch different files with no dependencies\"\n    },\n    {\n      \
         \"cue_ids\": [12],\n      \"label\": \"Refactor API handler\",\n      \
         \"rationale\": \"Depends on the auth fix from step 1\"\n    }\n  ]\n}\n\n\
         Rules:\n\
         - Every cue ID listed above MUST appear exactly once in the output.\n\
         - Minimize the number of sequential steps (maximize parallelism where safe).\n\
         - A step with multiple cue_ids means those cues run in parallel.\n",
    );
    prompt.push_str(&format!(
        "         - Each step may contain at most {} cue_ids. Split larger groups into separate steps.\n\
         - Order steps so that dependencies are respected.\n",
        MAX_PARALLEL_CUES,
    ));
    prompt
}

/// Parse the LLM response JSON into a WorkflowPlan.
/// Falls back to a sequential plan if parsing fails or cue IDs are invalid.
pub(crate) fn parse_workflow_response(
    response: &str,
    expected_cue_ids: &[i64],
) -> (WorkflowPlan, Option<String>) {
    // Try to extract JSON from the response (handle markdown fences, leading text, etc.)
    let json_str = crate::util::json_extract::extract_json(response);

    let parsed: Result<LlmWorkflowResponse, _> = serde_json::from_str(&json_str);
    match parsed {
        Ok(resp) => {
            let mut steps: Vec<WorkflowStep> = resp
                .steps
                .into_iter()
                .enumerate()
                .map(|(i, s)| WorkflowStep {
                    id: i,
                    cue_ids: s.cue_ids,
                    label: s.label,
                    rationale: s.rationale,
                    pause_after: false,
                    status: WorkflowStepStatus::Pending,
                })
                .collect();

            // Deduplicate: each cue ID should appear only once across all steps.
            let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
            for step in &mut steps {
                step.cue_ids.retain(|id| seen.insert(*id));
            }
            let missing: Vec<i64> = expected_cue_ids
                .iter()
                .filter(|id| !seen.contains(id))
                .copied()
                .collect();
            let warning = if !missing.is_empty() {
                // Append missing cues as a final sequential step
                let step_id = steps.len();
                steps.push(WorkflowStep {
                    id: step_id,
                    cue_ids: missing.clone(),
                    label: "Remaining cues (appended)".to_string(),
                    rationale: "These cues were missing from the LLM plan".to_string(),
                    pause_after: false,
                    status: WorkflowStepStatus::Pending,
                });
                Some(format!(
                    "LLM omitted {} cue(s) — appended as final step",
                    missing.len()
                ))
            } else {
                None
            };

            // Remove cue IDs that don't exist in expected list
            let expected_set: std::collections::HashSet<i64> =
                expected_cue_ids.iter().copied().collect();
            for step in &mut steps {
                step.cue_ids.retain(|id| expected_set.contains(id));
            }
            // Remove empty steps
            steps.retain(|s| !s.cue_ids.is_empty());
            // Re-index
            for (i, step) in steps.iter_mut().enumerate() {
                step.id = i;
            }

            if steps.is_empty() {
                return (
                    WorkflowPlan::sequential_fallback(expected_cue_ids),
                    Some("LLM returned empty plan — using sequential fallback".to_string()),
                );
            }

            (WorkflowPlan::from_steps(steps), warning)
        }
        Err(e) => (
            WorkflowPlan::sequential_fallback(expected_cue_ids),
            Some(format!(
                "Failed to parse LLM workflow JSON: {} — using sequential fallback",
                e
            )),
        ),
    }
}
