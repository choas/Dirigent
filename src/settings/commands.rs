use serde::{Deserialize, Serialize};

/// A command mode that can be triggered by prefixing a cue with `[command_name]`.
/// Commands wrap the cue text with additional prompt instructions and can
/// override the pre/post run scripts for that particular execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CueCommand {
    /// Short identifier used in `[name]` prefix (e.g. "plan", "test").
    pub name: String,
    /// Prompt template. `{task}` is replaced with the user's cue text.
    pub prompt: String,
    /// Shell command to run before the CLI invocation (overrides provider default).
    #[serde(default)]
    pub pre_agent: String,
    /// Shell command to run after the CLI invocation (overrides provider default).
    #[serde(default)]
    pub post_agent: String,
    /// Extra CLI arguments appended to the provider command (e.g. `--permission-mode plan`).
    #[serde(default)]
    pub cli_args: String,
}

pub(crate) fn default_commands() -> Vec<CueCommand> {
    vec![
        CueCommand {
            name: "plan".into(),
            prompt: "Analyze the following task and create a detailed implementation plan. Identify the files that need to change, the approach, edge cases, and risks. Do NOT make any code changes — only output the plan.\n\nTask: {task}".into(),
            pre_agent: String::new(),
            post_agent: String::new(),
            cli_args: "--permission-mode plan".into(),
        },
        CueCommand {
            name: "test".into(),
            prompt: "Write comprehensive tests for the following. Cover happy paths, edge cases, and error conditions.\n\nWhat to test: {task}".into(),
            pre_agent: String::new(),
            post_agent: String::new(),
            cli_args: String::new(),
        },
        CueCommand {
            name: "refactor".into(),
            prompt: "Refactor the following for clarity, maintainability, and idiomatic style. Preserve all existing behavior — do not change functionality.\n\nWhat to refactor: {task}".into(),
            pre_agent: String::new(),
            post_agent: String::new(),
            cli_args: String::new(),
        },
        CueCommand {
            name: "review".into(),
            prompt: "Review the following code or area for bugs, security issues, performance problems, and style concerns. Report findings with file paths and line numbers. Do NOT make any code changes.\n\nWhat to review: {task}".into(),
            pre_agent: String::new(),
            post_agent: String::new(),
            cli_args: String::new(),
        },
        CueCommand {
            name: "fix".into(),
            prompt: "Fix the following bug or issue. Identify the root cause, apply the minimal correct fix, and verify nothing else breaks.\n\nIssue: {task}".into(),
            pre_agent: String::new(),
            post_agent: String::new(),
            cli_args: String::new(),
        },
        CueCommand {
            name: "docs".into(),
            prompt: "Write or update documentation for the following. Include clear explanations, examples where helpful, and keep it concise.\n\nWhat to document: {task}".into(),
            pre_agent: String::new(),
            post_agent: String::new(),
            cli_args: String::new(),
        },
        CueCommand {
            name: "explain".into(),
            prompt: "Explain how the following works in detail. Walk through the control flow, data structures, and key decisions. Do NOT make any code changes.\n\nWhat to explain: {task}".into(),
            pre_agent: String::new(),
            post_agent: String::new(),
            cli_args: String::new(),
        },
        CueCommand {
            name: "optimize".into(),
            prompt: "Optimize the following for performance. Profile or reason about bottlenecks, then apply targeted improvements. Preserve correctness.\n\nWhat to optimize: {task}".into(),
            pre_agent: String::new(),
            post_agent: String::new(),
            cli_args: String::new(),
        },
    ]
}
