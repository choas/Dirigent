use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) enum CliProvider {
    #[default]
    Claude,
    OpenCode,
}

impl CliProvider {
    pub fn display_name(&self) -> &'static str {
        match self {
            CliProvider::Claude => "Claude",
            CliProvider::OpenCode => "OpenCode",
        }
    }

    pub fn all() -> &'static [CliProvider] {
        &[CliProvider::Claude, CliProvider::OpenCode]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) enum SourceKind {
    GitHubIssues,
    Slack,
    SonarQube,
    Notion,
    Mcp,
    Custom,
}

impl SourceKind {
    pub fn display_name(&self) -> &'static str {
        match self {
            SourceKind::GitHubIssues => "GitHub Issues",
            SourceKind::Slack => "Slack",
            SourceKind::SonarQube => "SonarQube",
            SourceKind::Notion => "Notion",
            SourceKind::Mcp => "MCP",
            SourceKind::Custom => "Custom",
        }
    }

    pub fn default_label(&self) -> &'static str {
        match self {
            SourceKind::GitHubIssues => "github",
            SourceKind::Slack => "slack",
            SourceKind::SonarQube => "sonar",
            SourceKind::Notion => "notion",
            SourceKind::Mcp => "mcp",
            SourceKind::Custom => "custom",
        }
    }

    pub fn all() -> &'static [SourceKind] {
        &[
            SourceKind::GitHubIssues,
            SourceKind::Slack,
            SourceKind::SonarQube,
            SourceKind::Notion,
            SourceKind::Mcp,
            SourceKind::Custom,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SourceConfig {
    pub name: String,
    pub kind: SourceKind,
    pub label: String,
    pub poll_interval_secs: u64,
    pub enabled: bool,
    #[serde(default)]
    pub filter: String,
    #[serde(default)]
    pub command: String,
    #[serde(skip)]
    pub token: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub host_url: String,
    #[serde(default)]
    pub project_key: String,
}

impl Default for SourceConfig {
    fn default() -> Self {
        SourceConfig {
            name: "New Source".to_string(),
            kind: SourceKind::GitHubIssues,
            label: SourceKind::GitHubIssues.default_label().to_string(),
            poll_interval_secs: 300,
            enabled: true,
            filter: String::new(),
            command: String::new(),
            token: String::new(),
            channel: String::new(),
            host_url: String::new(),
            project_key: String::new(),
        }
    }
}
