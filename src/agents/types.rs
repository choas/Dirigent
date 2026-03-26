use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Agent kinds
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum AgentKind {
    Format,
    Lint,
    Build,
    Test,
    Outdated,
    Custom(u32),
}

impl AgentKind {
    pub fn as_str(&self) -> &str {
        match self {
            AgentKind::Format => "format",
            AgentKind::Lint => "lint",
            AgentKind::Build => "build",
            AgentKind::Test => "test",
            AgentKind::Outdated => "outdated",
            AgentKind::Custom(_) => "custom",
        }
    }

    /// Returns a unique key suitable for database storage.
    pub fn db_key(&self) -> String {
        match self {
            AgentKind::Custom(id) => format!("custom_{}", id),
            other => other.as_str().to_string(),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            AgentKind::Format => "Format",
            AgentKind::Lint => "Lint",
            AgentKind::Build => "Build",
            AgentKind::Test => "Test",
            AgentKind::Outdated => "Outdated",
            AgentKind::Custom(_) => "Custom",
        }
    }
}

// ---------------------------------------------------------------------------
// Agent trigger
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AgentTrigger {
    /// Run automatically after a Claude/OpenCode run completes with changes.
    AfterRun,
    /// Run automatically after a diff is committed.
    AfterCommit,
    /// Run automatically after the specified agent completes.
    AfterAgent(AgentKind),
    /// Run automatically when files change on disk (debounced with the file watcher).
    OnFileChange,
    /// Only run when manually triggered.
    Manual,
}

impl AgentTrigger {
    pub fn display_name(&self) -> &str {
        match self {
            AgentTrigger::AfterRun => "After Run",
            AgentTrigger::AfterCommit => "After Commit",
            AgentTrigger::AfterAgent(_) => "After Agent",
            AgentTrigger::OnFileChange => "On File Change",
            AgentTrigger::Manual => "Manual",
        }
    }

    /// The base variants used for the trigger type selector (without inner data).
    pub fn base_variants() -> &'static [AgentTrigger] {
        &[
            AgentTrigger::AfterRun,
            AgentTrigger::AfterCommit,
            AgentTrigger::AfterAgent(AgentKind::Format), // placeholder
            AgentTrigger::OnFileChange,
            AgentTrigger::Manual,
        ]
    }

    /// Returns the discriminant index for comparison in the UI selector.
    pub fn variant_index(&self) -> usize {
        match self {
            AgentTrigger::AfterRun => 0,
            AgentTrigger::AfterCommit => 1,
            AgentTrigger::AfterAgent(_) => 2,
            AgentTrigger::OnFileChange => 3,
            AgentTrigger::Manual => 4,
        }
    }
}

// ---------------------------------------------------------------------------
// Agent status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentStatus {
    Idle,
    Running,
    Passed,
    Failed,
    Error,
}

impl AgentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentStatus::Idle => "idle",
            AgentStatus::Running => "running",
            AgentStatus::Passed => "passed",
            AgentStatus::Failed => "failed",
            AgentStatus::Error => "error",
        }
    }
}

// ---------------------------------------------------------------------------
// Agent configuration (persisted in settings)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AgentConfig {
    pub kind: AgentKind,
    /// User-visible display name. Empty string falls back to kind label.
    #[serde(default)]
    pub name: String,
    pub enabled: bool,
    pub command: String,
    pub trigger: AgentTrigger,
    pub timeout_secs: u64,
    /// Working directory relative to project root (empty = project root).
    #[serde(default)]
    pub working_dir: String,
    /// Shell command to run before the main agent command.
    /// The prompt text is available as `$PROMPT` env var.
    /// Use `$PROMPT` in the command itself to inline it.
    /// If this command exits non-zero, the agent run is skipped.
    #[serde(default)]
    pub before_run: String,
}

impl AgentConfig {
    /// Display name: custom name if set, otherwise the kind's default label.
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            self.kind.label()
        } else {
            &self.name
        }
    }
}

pub(crate) fn default_agents() -> Vec<AgentConfig> {
    Vec::new()
}

/// Generate the next unique ID for a custom agent.
pub(crate) fn next_custom_id(agents: &[AgentConfig]) -> u32 {
    agents
        .iter()
        .filter_map(|a| match a.kind {
            AgentKind::Custom(id) => Some(id),
            _ => None,
        })
        .max()
        .map(|max| max + 1)
        .unwrap_or(1)
}

#[cfg(test)]
impl AgentKind {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "format" => Some(AgentKind::Format),
            "lint" => Some(AgentKind::Lint),
            "build" => Some(AgentKind::Build),
            "test" => Some(AgentKind::Test),
            "outdated" => Some(AgentKind::Outdated),
            s if s.starts_with("custom_") => s[7..].parse().ok().map(AgentKind::Custom),
            "custom" => Some(AgentKind::Custom(0)),
            _ => None,
        }
    }
}

#[cfg(test)]
impl AgentStatus {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "idle" => Some(AgentStatus::Idle),
            "running" => Some(AgentStatus::Running),
            "passed" => Some(AgentStatus::Passed),
            "failed" => Some(AgentStatus::Failed),
            "error" => Some(AgentStatus::Error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtins() -> &'static [AgentKind] {
        &[
            AgentKind::Format,
            AgentKind::Lint,
            AgentKind::Build,
            AgentKind::Test,
            AgentKind::Outdated,
        ]
    }

    #[test]
    fn agent_kind_roundtrip() {
        for kind in builtins() {
            let s = kind.as_str();
            assert_eq!(AgentKind::from_str(s), Some(*kind));
        }
        // Custom kind uses db_key for round-trip
        let custom = AgentKind::Custom(42);
        assert_eq!(AgentKind::from_str(&custom.db_key()), Some(custom));
    }

    #[test]
    fn agent_status_roundtrip() {
        for status in &[
            AgentStatus::Idle,
            AgentStatus::Running,
            AgentStatus::Passed,
            AgentStatus::Failed,
            AgentStatus::Error,
        ] {
            assert_eq!(AgentStatus::from_str(status.as_str()), Some(*status));
        }
    }

    #[test]
    fn default_agents_is_empty() {
        let agents = default_agents();
        assert!(agents.is_empty());
    }
}
