mod app_settings;
mod commands;
mod home_guard;
mod io;
mod playbook;
mod providers;
mod recent;
mod semantic_colors;
mod theme;
mod trust;

pub(crate) use app_settings::{
    DiffColorScheme, FontWeight, HeartbeatStyle, RunningAnimation, Settings,
};
pub(crate) use commands::{default_commands, CueCommand};
pub(crate) use home_guard::sync_home_guard_hook;
pub(crate) use io::{load_settings, resolve_in_path, save_settings, PendingTrust};
pub(crate) use playbook::{
    default_playbook, parse_play_variables, substitute_play_variables, Play, PlayVariable,
};
pub(crate) use providers::{
    CliProvider, NotionPageType, SourceConfig, SourceKind, SshAuthKind, SshServer, VcsBackend,
};
pub(crate) use recent::{add_global_recent_project, add_recent_repo, load_global_recent_projects};
pub(crate) use semantic_colors::SemanticColors;
pub(crate) use theme::{CustomTheme, ThemeChoice};
pub(crate) use trust::{
    approve_fast_llm_key_host, fast_llm_key_host_approved,
    record_approval as record_trust_approval, repo_settings_trusted,
};
