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
    Trello,
    Asana,
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
    #[serde(skip)]
    pub api_key: String,
    /// Notion-specific: the type of page/database (Todo List or Kanban Board).
    #[serde(default)]
    pub notion_page_type: NotionPageType,
    /// Notion-specific: the Kanban status property name (defaults to "Status").
    #[serde(default)]
    pub notion_status_property: String,
    /// Notion-specific: the checkbox property name (`TodoList`) or target status
    /// value (`KanbanBoard`) for marking items done (see [`NotionPageType`]).
    #[serde(default)]
    pub notion_done_value: String,
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
            api_key: String::new(),
            notion_page_type: NotionPageType::default(),
            notion_status_property: String::new(),
            notion_done_value: "Done".to_string(),
        }
    }
}
