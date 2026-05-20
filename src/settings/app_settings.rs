use serde::{Deserialize, Serialize};

use crate::agents::{default_agents, AgentConfig};
use crate::lsp::{default_lsp_servers, LspServerConfig};

use super::commands::{default_commands, CueCommand};
use super::playbook::{default_playbook, Play};
use super::providers::{CliProvider, SourceConfig, SshServer};
use super::theme::{CustomTheme, ThemeChoice};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) enum DiffColorScheme {
    #[default]
    RedGreen,
    RedBlue,
    YellowBlue,
}

impl DiffColorScheme {
    pub(crate) fn display_name(&self) -> &str {
        match self {
            DiffColorScheme::RedGreen => "Red \u{2013} Green",
            DiffColorScheme::RedBlue => "Red \u{2013} Blue",
            DiffColorScheme::YellowBlue => "Yellow \u{2013} Blue",
        }
    }

    pub(crate) fn all() -> &'static [DiffColorScheme] {
        &[
            DiffColorScheme::RedGreen,
            DiffColorScheme::RedBlue,
            DiffColorScheme::YellowBlue,
        ]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) enum HeartbeatStyle {
    #[default]
    Curve,
    GabbaPeak,
    MorseCode,
    Off,
}

impl HeartbeatStyle {
    pub(crate) fn display_name(&self) -> &str {
        match self {
            HeartbeatStyle::Curve => "Curve",
            HeartbeatStyle::GabbaPeak => "Gabba Peak",
            HeartbeatStyle::MorseCode => "Morse Code",
            HeartbeatStyle::Off => "Off",
        }
    }

    pub(crate) fn all() -> &'static [HeartbeatStyle] {
        &[
            HeartbeatStyle::Curve,
            HeartbeatStyle::GabbaPeak,
            HeartbeatStyle::MorseCode,
            HeartbeatStyle::Off,
        ]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) enum RunningAnimation {
    Off,
    #[default]
    LavaLamp,
    ClaudeCodeName,
    Dino,
}

impl RunningAnimation {
    pub(crate) fn display_name(&self) -> &str {
        match self {
            RunningAnimation::Off => "Off",
            RunningAnimation::LavaLamp => "Lava Lamp",
            RunningAnimation::ClaudeCodeName => "Claude Code",
            RunningAnimation::Dino => "Desert Dino",
        }
    }

    pub(crate) fn all() -> &'static [RunningAnimation] {
        &[
            RunningAnimation::Off,
            RunningAnimation::LavaLamp,
            RunningAnimation::ClaudeCodeName,
            RunningAnimation::Dino,
        ]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) enum FontWeight {
    Light,
    #[default]
    Regular,
    Medium,
    SemiBold,
    Bold,
}

impl FontWeight {
    pub(crate) fn display_name(&self) -> &str {
        match self {
            FontWeight::Light => "Light",
            FontWeight::Regular => "Regular",
            FontWeight::Medium => "Medium",
            FontWeight::SemiBold => "SemiBold",
            FontWeight::Bold => "Bold",
        }
    }

    pub(crate) fn all() -> &'static [FontWeight] {
        &[
            FontWeight::Light,
            FontWeight::Regular,
            FontWeight::Medium,
            FontWeight::SemiBold,
            FontWeight::Bold,
        ]
    }

    /// File-name suffix used when searching for weight-specific font files
    /// (e.g. "JetBrains Mono-Bold.ttf").
    pub(crate) fn file_suffix(&self) -> &str {
        match self {
            FontWeight::Light => "Light",
            FontWeight::Regular => "Regular",
            FontWeight::Medium => "Medium",
            FontWeight::SemiBold => "SemiBold",
            FontWeight::Bold => "Bold",
        }
    }

    /// Whether this weight maps to a bold face in .ttc collections.
    pub(crate) fn is_bold(&self) -> bool {
        matches!(self, FontWeight::SemiBold | FontWeight::Bold)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub theme: ThemeChoice,
    /// User-defined custom themes.
    #[serde(default)]
    pub custom_themes: Vec<CustomTheme>,
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
    pub gemini_model: String,
    #[serde(default)]
    pub gemini_cli_path: String,
    #[serde(default)]
    pub gemini_extra_args: String,
    /// Environment variable **names** to forward to the CLI process (one per line).
    /// Values are resolved from the current environment at runtime — never stored.
    #[serde(default)]
    pub gemini_env_vars: String,
    #[serde(default)]
    pub gemini_pre_run_script: String,
    #[serde(default)]
    pub gemini_post_run_script: String,
    pub recent_repos: Vec<String>,
    #[serde(default = "default_true")]
    pub notify_sound: bool,
    #[serde(default = "default_true")]
    pub notify_popup: bool,
    #[serde(default)]
    pub running_animation: RunningAnimation,
    #[serde(default)]
    pub heartbeat_style: HeartbeatStyle,
    #[serde(default)]
    pub claude_code_display_name: String,
    #[serde(default)]
    pub diff_color_scheme: DiffColorScheme,
    #[serde(default = "default_font_family")]
    pub font_family: String,
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    #[serde(default)]
    pub font_weight: FontWeight,
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
    /// SSH remote server configurations.
    #[serde(default)]
    pub ssh_servers: Vec<SshServer>,
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
    /// Automatically commit each cue when it moves to Review.
    #[serde(default)]
    pub auto_commit: bool,
    /// Append `--dangerously-skip-permissions` to the Claude CLI invocation.
    /// Enabled by default — without this flag, non-interactive `-p` mode
    /// cannot get tool permissions and Claude will only describe changes
    /// instead of actually editing files.
    #[serde(default = "default_true")]
    pub allow_dangerous_skip_permissions: bool,
    /// Show frame timing breakdown and memory usage in the status bar.
    #[serde(default)]
    pub show_frame_timing: bool,
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
            CliProvider::Gemini => ProviderFields {
                model: &self.gemini_model,
                cli_path: &self.gemini_cli_path,
                extra_args: &self.gemini_extra_args,
                env_vars: &self.gemini_env_vars,
                pre_run_script: &self.gemini_pre_run_script,
                post_run_script: &self.gemini_post_run_script,
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
            custom_themes: Vec::new(),
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
            gemini_model: "gemini-2.5-flash".to_string(),
            gemini_cli_path: String::new(),
            gemini_extra_args: String::new(),
            gemini_env_vars: String::new(),
            gemini_pre_run_script: String::new(),
            gemini_post_run_script: String::new(),
            recent_repos: Vec::new(),
            notify_sound: true,
            notify_popup: true,
            running_animation: RunningAnimation::LavaLamp,
            heartbeat_style: HeartbeatStyle::default(),
            claude_code_display_name: String::new(),
            diff_color_scheme: DiffColorScheme::default(),
            font_family: default_font_family(),
            font_size: default_font_size(),
            font_weight: FontWeight::default(),
            sources: Vec::new(),
            playbook: default_playbook(),
            allow_home_folder_access: false,
            agent_shell_init: String::new(),
            agents: default_agents(),
            commands: default_commands(),
            ssh_servers: Vec::new(),
            lsp_servers: default_lsp_servers(),
            lsp_enabled: false,
            prompt_suggestions_enabled: false,
            auto_context_file: false,
            auto_context_git_diff: false,
            auto_commit: false,
            allow_dangerous_skip_permissions: true,
            show_frame_timing: false,
        }
    }
}
