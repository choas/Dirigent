mod custom;
mod external;
mod finding_text;
mod html;
mod pr_comments;
mod pr_feedback;
mod pr_findings;
mod types;

pub(crate) use custom::{collect_drained, drain_child_pipes, fetch_custom_command};
pub(crate) use external::{
    build_source_curl, fetch_asana_tasks, fetch_github_issues, fetch_notion_objects,
    fetch_notion_tasks, fetch_sentry_issues, fetch_slack_messages, fetch_sonarqube_issues,
    fetch_trello_cards, load_env_var, mark_notion_done,
};
pub(crate) use html::strip_html_tags;
pub(crate) use pr_feedback::notify_pr_finding_fixed;
pub(crate) use pr_findings::{fetch_pr_findings, strip_pr_context_hint};
pub(crate) use types::{NotionObject, PrFinding, SourceItem};

use std::path::Path;

use crate::settings::{SourceConfig, SourceKind};

/// Resolve the token for a source: use the in-memory value if set, otherwise
/// fall back to the appropriate environment variable from `.Dirigent/.env`
/// (preferred) or `.env`.
pub(crate) fn resolve_source_token(source: &SourceConfig, project_root: &Path) -> String {
    if !source.token.is_empty() {
        return source.token.clone();
    }
    let env_key = match source.kind {
        SourceKind::Slack => "SLACK_BOT_TOKEN",
        SourceKind::SonarQube => "SONAR_TOKEN",
        SourceKind::Trello => "TRELLO_TOKEN",
        SourceKind::Asana => "ASANA_TOKEN",
        SourceKind::Notion => "NOTION_TOKEN",
        SourceKind::Sentry => "SENTRY_AUTH_TOKEN",
        _ => return String::new(),
    };
    std::env::var(env_key)
        .ok()
        .or_else(|| load_env_var(project_root, env_key))
        .unwrap_or_default()
}

/// Return `true` if `env_key` resolves to a non-empty value in the process
/// environment or in `.Dirigent/.env` (preferred) / `.env` (fallback).
///
/// Used by the settings UI to show whether an auth token is already available
/// from the environment, so the user need not paste it into the field.
pub(crate) fn env_token_available(project_root: &Path, env_key: &str) -> bool {
    if std::env::var(env_key).is_ok_and(|v| !v.is_empty()) {
        return true;
    }
    load_env_var(project_root, env_key).is_some_and(|v| !v.is_empty())
}
