use serde::{Deserialize, Serialize};

use crate::agents::{default_agents, AgentConfig};

use super::commands::{default_commands, CueCommand};
use super::playbook::{default_playbook, Play};
use super::providers::{CliProvider, SourceConfig};
use super::theme::ThemeChoice;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Settings {
    pub theme: ThemeChoice,
    pub cli_provider: CliProvider,
    pub claude_model: String,
    #[serde(default)]
    pub claude_cli_path: String,
    #[serde(default)]
    pub claude_extra_args: String,
    #[serde(default)]
    pub claude_env_vars: String,
    #[serde(default)]
    pub claude_pre_run_script: String,
    #[serde(default)]
    pub claude_post_run_script: String,
    pub opencode_model: String,
    #[serde(default)]
    pub opencode_cli_path: String,
    #[serde(default)]
    pub opencode_extra_args: String,
    #[serde(default)]
    pub opencode_env_vars: String,
    #[serde(default)]
    pub opencode_pre_run_script: String,
    #[serde(default)]
    pub opencode_post_run_script: String,
    pub recent_repos: Vec<String>,
    #[serde(default = "default_true")]
    pub notify_sound: bool,
    #[serde(default = "default_true")]
    pub notify_popup: bool,
    #[serde(default = "default_true")]
    pub lava_lamp_enabled: bool,
    #[serde(default = "default_font_family")]
    pub font_family: String,
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    #[serde(default = "default_playbook")]
    pub playbook: Vec<Play>,
    /// Allow running Claude/OpenCode when the project root is the user's home folder.
    /// Disabled by default to prevent the AI from reading personal folders like
    /// Documents, Desktop, Photos, etc.
    #[serde(default)]
    pub allow_home_folder_access: bool,
    /// Shell init snippet prepended to every agent command (e.g. `source ~/.zprofile`).
    /// Solves the macOS GUI-app problem where PATH / JAVA_HOME etc. are not set.
    #[serde(default)]
    pub agent_shell_init: String,
    #[serde(default = "default_agents")]
    pub agents: Vec<AgentConfig>,
    /// Command modes triggered by `[name]` prefix in cue text.
    #[serde(default = "default_commands")]
    pub commands: Vec<CueCommand>,
    /// Show heuristic prompt-refinement suggestions below the prompt field.
    #[serde(default)]
    pub prompt_suggestions_enabled: bool,
    /// Automatically include file content (±50 lines) around the cue location in the prompt.
    #[serde(default)]
    pub auto_context_file: bool,
    /// Automatically include the git diff in the prompt.
    #[serde(default)]
    pub auto_context_git_diff: bool,
}

fn default_true() -> bool {
    true
}

fn default_font_family() -> String {
    "Menlo".to_string()
}

fn default_font_size() -> f32 {
    13.0
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeChoice::Dark,
            cli_provider: CliProvider::default(),
            claude_model: "claude-opus-4-6".to_string(),
            claude_cli_path: String::new(),
            claude_extra_args: String::new(),
            claude_env_vars: String::new(),
            claude_pre_run_script: String::new(),
            claude_post_run_script: String::new(),
            opencode_model: "openai/o1".to_string(),
            opencode_cli_path: String::new(),
            opencode_extra_args: String::new(),
            opencode_env_vars: String::new(),
            opencode_pre_run_script: String::new(),
            opencode_post_run_script: String::new(),
            recent_repos: Vec::new(),
            notify_sound: true,
            notify_popup: true,
            lava_lamp_enabled: true,
            font_family: default_font_family(),
            font_size: default_font_size(),
            sources: Vec::new(),
            playbook: default_playbook(),
            allow_home_folder_access: false,
            agent_shell_init: String::new(),
            agents: default_agents(),
            commands: default_commands(),
            prompt_suggestions_enabled: false,
            auto_context_file: false,
            auto_context_git_diff: false,
        }
    }
}
