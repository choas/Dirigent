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
        WorkflowPlan {
            steps,
            current_step: 0,
        }
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
         - A step with multiple cue_ids means those cues run in parallel.\n\
         - Order steps so that dependencies are respected.\n",
    );
    prompt
}

/// Parse the LLM response JSON into a WorkflowPlan.
/// Falls back to a sequential plan if parsing fails or cue IDs are invalid.
pub(crate) fn parse_workflow_response(
    response: &str,
    expected_cue_ids: &[i64],
) -> (WorkflowPlan, Option<String>) {
    // Try to extract JSON from the response (handle markdown fences, leading text, etc.)
    let json_str = extract_json(response);

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

/// Extract a JSON object from a string that may contain markdown fences or surrounding text.
fn extract_json(s: &str) -> String {
    // Try to find JSON between code fences
    if let Some(start) = s.find("```json") {
        let after = &s[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    if let Some(start) = s.find("```") {
        let after = &s[start + 3..];
        if let Some(end) = after.find("```") {
            let inner = after[..end].trim();
            if inner.starts_with('{') {
                return inner.to_string();
            }
        }
    }
    // Try to find a raw JSON object
    if let Some(start) = s.find('{') {
        if let Some(end) = s.rfind('}') {
            if end > start {
                return s[start..=end].to_string();
            }
        }
    }
    s.trim().to_string()
}
