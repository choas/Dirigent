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
    let resp_steps = try_parse_workflow_json(response);
    match resp_steps {
        Some(resp_steps) => {
            let mut steps: Vec<WorkflowStep> = resp_steps
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
        None => (
            WorkflowPlan::sequential_fallback(expected_cue_ids),
            Some("Failed to parse LLM workflow JSON — using sequential fallback".to_string()),
        ),
    }
}

/// Try multiple strategies to parse workflow JSON from the LLM response.
fn try_parse_workflow_json(response: &str) -> Option<Vec<LlmWorkflowStep>> {
    if let Some(steps) = try_extract_strategies(response) {
        return Some(steps);
    }

    // Repair PTY word-wrap damage — collapse line breaks / carriage returns
    // inside JSON string values, then retry all strategies on the repaired text.
    let repaired = repair_pty_line_breaks(response);
    if repaired != response {
        if let Some(steps) = try_extract_strategies(&repaired) {
            return Some(steps);
        }
    }

    let preview_len = 500;
    let start = response.len().saturating_sub(preview_len);
    let suffix = response.get(start..).unwrap_or(response);
    log::warn!(
        "[workflow] all parse strategies failed; response tail ({} chars): {:?}",
        response.len(),
        suffix,
    );

    None
}

fn try_parse_steps(json: &str) -> Option<Vec<LlmWorkflowStep>> {
    serde_json::from_str::<LlmWorkflowResponse>(json)
        .map(|r| r.steps)
        .or_else(|_| serde_json::from_str::<Vec<LlmWorkflowStep>>(json))
        .ok()
}

/// Run extraction strategies 1–5 on the given text.
fn try_extract_strategies(response: &str) -> Option<Vec<LlmWorkflowStep>> {
    // Strategy 1: extract via balanced-brace heuristic
    let extracted = crate::util::json_extract::extract_json(response);
    if let Some(steps) = try_parse_steps(&extracted) {
        return Some(steps);
    }

    // Strategy 2: find the last `{"steps"` occurrence (compact JSON)
    if let Some(pos) = response.rfind("{\"steps\"") {
        let candidate = &response[pos..];
        let balanced = crate::util::json_extract::extract_json(candidate);
        if let Some(steps) = try_parse_steps(&balanced) {
            return Some(steps);
        }
    }

    // Strategy 3: find the last `"steps"` key with flexible whitespace
    for (i, _) in response.rmatch_indices("\"steps\"") {
        let before = response[..i].trim_end();
        if before.ends_with('{') {
            let start = before.len() - 1;
            let candidate = &response[start..];
            let balanced = crate::util::json_extract::extract_json(candidate);
            if let Some(steps) = try_parse_steps(&balanced) {
                return Some(steps);
            }
        }
    }

    // Strategy 4: find the last `[{"cue_ids"` occurrence (bare array format)
    if let Some(pos) = response.rfind("[{\"cue_ids\"") {
        let candidate = &response[pos..];
        let balanced = crate::util::json_extract::extract_json(candidate);
        if let Some(steps) = try_parse_steps(&balanced) {
            return Some(steps);
        }
    }

    // Strategy 5: find the last `"cue_ids"` with flexible whitespace (bare array)
    for (i, _) in response.rmatch_indices("\"cue_ids\"") {
        let before = response[..i].trim_end();
        if before.ends_with("[{") || before.ends_with("[ {") {
            let start = before.rfind('[').unwrap();
            let candidate = &response[start..];
            let balanced = crate::util::json_extract::extract_json(candidate);
            if let Some(steps) = try_parse_steps(&balanced) {
                return Some(steps);
            }
        }
    }

    None
}

/// Repair PTY word-wrap damage: collapse newlines that appear inside JSON
/// string values. Walks the text tracking quote state (respecting backslash
/// escapes) and replaces `\n` inside strings with a space.
fn repair_pty_line_breaks(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_string = false;
    let mut prev_backslash = false;
    for ch in s.chars() {
        if in_string {
            if ch == '\n' || ch == '\r' {
                out.push(' ');
                continue;
            }
            if ch == '"' && !prev_backslash {
                in_string = false;
            }
            prev_backslash = ch == '\\' && !prev_backslash;
        } else if ch == '"' {
            in_string = true;
            prev_backslash = false;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clean_json() {
        let response =
            r#"{"steps": [{"cue_ids": [1, 2], "label": "Fix bugs", "rationale": "Independent"}]}"#;
        let (plan, warning) = parse_workflow_response(response, &[1, 2]);
        assert!(warning.is_none());
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].cue_ids, vec![1, 2]);
    }

    #[test]
    fn parse_multiline_json() {
        let response = "{\n  \"steps\": [\n    {\n      \"cue_ids\": [10],\n      \"label\": \"Step one\",\n      \"rationale\": \"First\"\n    }\n  ]\n}";
        let (plan, warning) = parse_workflow_response(response, &[10]);
        assert!(warning.is_none());
        assert_eq!(plan.steps.len(), 1);
    }

    #[test]
    fn parse_with_markdown_fence() {
        let response = "Here is the plan:\n```json\n{\"steps\": [{\"cue_ids\": [5], \"label\": \"Do it\", \"rationale\": \"Why not\"}]}\n```\n";
        let (plan, warning) = parse_workflow_response(response, &[5]);
        assert!(warning.is_none());
        assert_eq!(plan.steps[0].cue_ids, vec![5]);
    }

    #[test]
    fn parse_with_echoed_prompt() {
        let response = "Output JSON: {\"steps\": [{\"cue_ids\": [3, 7]}]}\n\
                         Rules: ...\n\n\
                         {\"steps\": [{\"cue_ids\": [1], \"label\": \"Fix\", \"rationale\": \"Real answer\"}]}";
        let (plan, warning) = parse_workflow_response(response, &[1]);
        assert!(warning.is_none());
        assert_eq!(plan.steps[0].label, "Fix");
    }

    #[test]
    fn parse_pty_wrapped_json() {
        // Simulate PTY wrapping a long label across lines
        let response = "{\"steps\": [{\"cue_ids\": [1], \"label\": \"Fix the authenticati\non bug in the login handler\", \"rationale\": \"Needs fixing\"}]}";
        let (plan, warning) = parse_workflow_response(response, &[1]);
        assert!(warning.is_none(), "got warning: {:?}", warning);
        assert!(plan.steps[0].label.contains("authenticati"));
    }

    #[test]
    fn fallback_on_garbage() {
        let response = "Sorry, I can't do that.";
        let (plan, warning) = parse_workflow_response(response, &[1, 2]);
        assert!(warning.is_some());
        assert!(warning.unwrap().contains("Failed to parse"));
        assert_eq!(plan.steps.len(), 2);
    }

    #[test]
    fn missing_cue_ids_appended() {
        let response = r#"{"steps": [{"cue_ids": [1], "label": "First", "rationale": "R"}]}"#;
        let (plan, warning) = parse_workflow_response(response, &[1, 2, 3]);
        assert!(warning.is_some());
        assert!(warning.unwrap().contains("omitted 2 cue(s)"));
        let all_ids: Vec<i64> = plan
            .steps
            .iter()
            .flat_map(|s| &s.cue_ids)
            .copied()
            .collect();
        assert!(all_ids.contains(&2));
        assert!(all_ids.contains(&3));
    }

    #[test]
    fn repair_pty_line_breaks_fixes_strings() {
        let broken = "\"hello\nworld\"";
        assert_eq!(repair_pty_line_breaks(broken), "\"hello world\"");
    }

    #[test]
    fn repair_pty_line_breaks_preserves_outside_strings() {
        let input = "{\n\"key\": \"val\"\n}";
        assert_eq!(repair_pty_line_breaks(input), "{\n\"key\": \"val\"\n}");
    }

    #[test]
    fn repair_pty_line_breaks_handles_escaped_quotes() {
        let input = "{\"key\": \"val with \\\" escape\nstill inside\"}";
        let repaired = repair_pty_line_breaks(input);
        assert!(repaired.contains("escape still inside"), "got: {repaired}");
    }
}
