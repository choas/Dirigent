mod cli;
mod diff_parser;
mod done_hook;
mod import_cues;
mod invoke;
mod prompt;
mod stream;
mod types;

pub(crate) use cli::{apply_dirigent_env, apply_env_vars, load_env_var_with_dirigent_fallback};
pub(crate) use diff_parser::{parse_diff_from_response, parse_hunk_header};
pub(crate) use import_cues::{
    build_import_prompt, extract_pr_label, is_import_request, parse_import_cues, ImportedCue,
};
pub(crate) use invoke::{invoke_claude_streaming, summarize_commit_message_via_cli};
pub(crate) use prompt::{
    build_prompt_with_auto_context, build_reply_prompt, extract_commit_message,
    extract_user_text_from_prompt, gather_auto_context, parse_command_prefix,
};
pub(crate) use stream::{
    detect_stopped_early, detect_usage_limit, extract_plan_path, filter_opencode_log_line,
    response_has_question,
};
pub(crate) use types::{ClaudeError, RunMetrics};
