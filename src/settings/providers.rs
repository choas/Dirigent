use serde::{Deserialize, Serialize};

/// The type of Notion page/database being used as a source.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) enum NotionPageType {
    /// A database with a checkbox property (e.g. "Done") for completion.
    #[default]
    TodoList,
    /// A database with a Status/Select property used as Kanban columns.
    KanbanBoard,
}

impl NotionPageType {
    pub fn display_name(&self) -> &'static str {
        match self {
            NotionPageType::TodoList => "Todo List",
            NotionPageType::KanbanBoard => "Kanban Board",
        }
    }

    pub fn all() -> &'static [NotionPageType] {
        &[NotionPageType::TodoList, NotionPageType::KanbanBoard]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) enum CliProvider {
    #[default]
    Claude,
    OpenCode,
    Gemini,
    Acp,
}

impl CliProvider {
    pub fn display_name(&self) -> &'static str {
        match self {
            CliProvider::Claude => "Claude",
            CliProvider::OpenCode => "OpenCode",
            CliProvider::Gemini => "Gemini",
            CliProvider::Acp => "ACP Agent",
        }
    }

    pub fn all() -> &'static [CliProvider] {
        &[
            CliProvider::Claude,
            CliProvider::OpenCode,
            CliProvider::Gemini,
            CliProvider::Acp,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) enum SourceKind {
    GitHubIssues,
    Slack,
    SonarQube,
    Notion,
    Trello,
    Asana,
    Sentry,
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
            SourceKind::Trello => "Trello",
            SourceKind::Asana => "Asana",
            SourceKind::Sentry => "Sentry",
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
            SourceKind::Trello => "trello",
            SourceKind::Asana => "asana",
            SourceKind::Sentry => "sentry",
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
            SourceKind::Trello,
            SourceKind::Asana,
            SourceKind::Sentry,
            SourceKind::Mcp,
            SourceKind::Custom,
        ]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) enum SshAuthKind {
    #[default]
    Agent,
    KeyFile,
    Password,
}

impl SshAuthKind {
    pub fn display_name(&self) -> &'static str {
        match self {
            SshAuthKind::Agent => "SSH Agent",
            SshAuthKind::KeyFile => "Key File",
            SshAuthKind::Password => "Password",
        }
    }

    pub fn all() -> &'static [SshAuthKind] {
        &[
            SshAuthKind::Agent,
            SshAuthKind::KeyFile,
            SshAuthKind::Password,
        ]
    }
}

fn generate_ssh_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SshServer {
    #[serde(default = "generate_ssh_id")]
    pub id: String,
    pub name: String,
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub auth_kind: SshAuthKind,
    #[serde(default)]
    pub key_path: String,
    #[serde(skip)]
    pub password: String,
    #[serde(default = "default_remote_path")]
    pub remote_path: String,
}

fn default_ssh_port() -> u16 {
    22
}

fn default_remote_path() -> String {
    "~".into()
}

impl Default for SshServer {
    fn default() -> Self {
        SshServer {
            id: generate_ssh_id(),
            name: "New Server".into(),
            host: String::new(),
            port: 22,
            username: String::new(),
            auth_kind: SshAuthKind::default(),
            key_path: String::new(),
            password: String::new(),
            remote_path: default_remote_path(),
        }
    }
}

fn generate_source_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn default_notion_status_property() -> String {
    "Status".into()
}

fn default_notion_done_value() -> String {
    "Done".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SourceConfig {
    /// Stable unique identifier (UUID); survives label/name renames.
    /// `None` for legacy configs that were saved without an id.
    #[serde(default)]
    pub id: Option<String>,
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
    #[serde(skip)]
    pub api_key: String,
    /// Notion-specific: the type of page/database (Todo List or Kanban Board).
    #[serde(default)]
    pub notion_page_type: NotionPageType,
    /// Notion-specific: the Kanban status property name (defaults to "Status").
    #[serde(default = "default_notion_status_property")]
    pub notion_status_property: String,
    /// Notion-specific: the checkbox property name (`TodoList`) or target status
    /// value (`KanbanBoard`) for marking items done (see [`NotionPageType`]).
    #[serde(default = "default_notion_done_value")]
    pub notion_done_value: String,
    /// When true, cues whose text starts with `{runnable}` are automatically
    /// moved to Ready and executed.  Disabled by default — enable only for
    /// trusted sources, as it grants the source the ability to trigger
    /// arbitrary AI runs.
    #[serde(default)]
    pub allow_runnable: bool,
}

impl Default for SourceConfig {
    fn default() -> Self {
        SourceConfig {
            id: Some(generate_source_id()),
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
            api_key: String::new(),
            notion_page_type: NotionPageType::default(),
            notion_status_property: default_notion_status_property(),
            notion_done_value: default_notion_done_value(),
            allow_runnable: false,
        }
    }
}
