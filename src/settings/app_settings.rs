use serde::{Deserialize, Serialize};

use crate::agents::{default_agents, AgentConfig};
use crate::lsp::{default_lsp_servers, LspServerConfig};

use super::commands::{default_commands, CueCommand};
use super::playbook::{default_playbook, Play};
use super::providers::{CliProvider, SourceConfig};
use super::theme::ThemeChoice;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub theme: ThemeChoice,
    pub cli_provider: CliProvider,
    pub claude_model: String,
    /// Extra model identifiers to show in the Claude model dropdown.
    /// Add new model IDs here (in settings JSON) so they appear without a code change.
    #[serde(default)]
    pub claude_custom_models: Vec<String>,
    #[serde(default)]
    pub claude_cli_path: String,
    #[serde(default)]
    pub claude_extra_args: String,
    /// Environment variable **names** to forward to the CLI process (one per line).
    /// Values are resolved from the current environment at runtime — never stored.
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
    /// Environment variable **names** to forward to the CLI process (one per line).
    /// Values are resolved from the current environment at runtime — never stored.
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
    /// Language server configurations for LSP integration.
    #[serde(default = "default_lsp_servers")]
    pub lsp_servers: Vec<LspServerConfig>,
    /// Master toggle for LSP support.
    #[serde(default)]
    pub lsp_enabled: bool,
    /// Show heuristic prompt-refinement suggestions below the prompt field.
    #[serde(default)]
    pub prompt_suggestions_enabled: bool,
    /// Automatically include file content (±50 lines) around the cue location in the prompt.
    #[serde(default)]
    pub auto_context_file: bool,
    /// Automatically include the git diff in the prompt.
    #[serde(default)]
    pub auto_context_git_diff: bool,
    /// Append `--dangerously-skip-permissions` to the Claude CLI invocation.
    /// Enabled by default — without this flag, non-interactive `-p` mode
    /// cannot get tool permissions and Claude will only describe changes
    /// instead of actually editing files.
    #[serde(default = "default_true")]
    pub allow_dangerous_skip_permissions: bool,
}

/// Common per-provider fields extracted from [`Settings`].
pub(crate) struct ProviderFields<'a> {
    pub model: &'a str,
    pub cli_path: &'a str,
    pub extra_args: &'a str,
    pub env_vars: &'a str,
    pub pre_run_script: &'a str,
    pub post_run_script: &'a str,
}

impl Settings {
    /// Return the provider-specific fields for the given CLI provider.
    pub(crate) fn provider_fields(&self, provider: &CliProvider) -> ProviderFields<'_> {
        match provider {
            CliProvider::Claude => ProviderFields {
                model: &self.claude_model,
                cli_path: &self.claude_cli_path,
                extra_args: &self.claude_extra_args,
                env_vars: &self.claude_env_vars,
                pre_run_script: &self.claude_pre_run_script,
                post_run_script: &self.claude_post_run_script,
            },
            CliProvider::OpenCode => ProviderFields {
                model: &self.opencode_model,
                cli_path: &self.opencode_cli_path,
                extra_args: &self.opencode_extra_args,
                env_vars: &self.opencode_env_vars,
                pre_run_script: &self.opencode_pre_run_script,
                post_run_script: &self.opencode_post_run_script,
            },
        }
    }
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
            claude_custom_models: Vec::new(),
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
            lsp_servers: default_lsp_servers(),
            lsp_enabled: false,
            prompt_suggestions_enabled: false,
            auto_context_file: false,
            auto_context_git_diff: false,
            allow_dangerous_skip_permissions: true,
        }
    }
}
