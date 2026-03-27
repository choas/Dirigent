mod cli;
mod diff_parser;
mod invoke;
mod prompt;
mod stream;
mod types;

pub(crate) use cli::{apply_dirigent_env, load_env_var_with_dirigent_fallback};
pub(crate) use diff_parser::parse_diff_from_response;
pub(crate) use invoke::invoke_claude_streaming;
pub(crate) use prompt::{
    build_prompt_with_auto_context, build_reply_prompt, extract_user_text_from_prompt,
    gather_auto_context, parse_command_prefix,
};
pub(crate) use stream::{extract_plan_path, filter_opencode_log_line};
pub(crate) use types::{ClaudeError, RunMetrics};
