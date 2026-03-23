use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Agent kinds
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum AgentKind {
    Format,
    Lint,
    Build,
    Test,
    Custom(u32),
}

impl AgentKind {
    pub fn as_str(&self) -> &str {
        match self {
            AgentKind::Format => "format",
            AgentKind::Lint => "lint",
            AgentKind::Build => "build",
            AgentKind::Test => "test",
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
            AgentKind::Custom(_) => "Custom",
        }
    }

    /// Built-in agent kinds (used for language presets and tests).
    #[allow(dead_code)]
    pub fn builtins() -> &'static [AgentKind] {
        &[
            AgentKind::Format,
            AgentKind::Lint,
            AgentKind::Build,
            AgentKind::Test,
        ]
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

// ---------------------------------------------------------------------------
// Language presets for agent initialization
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentLanguage {
    Rust,
    TypeScript,
    Python,
    Go,
    Java,
    CSharp,
    Ruby,
    Swift,
    Kotlin,
    Cpp,
    Elixir,
    Zig,
    Lua,
}

impl AgentLanguage {
    pub fn label(&self) -> &'static str {
        match self {
            AgentLanguage::Rust => "Rust",
            AgentLanguage::TypeScript => "TypeScript",
            AgentLanguage::Python => "Python",
            AgentLanguage::Go => "Go",
            AgentLanguage::Java => "Java",
            AgentLanguage::CSharp => "C#",
            AgentLanguage::Ruby => "Ruby",
            AgentLanguage::Swift => "Swift",
            AgentLanguage::Kotlin => "Kotlin",
            AgentLanguage::Cpp => "C/C++",
            AgentLanguage::Elixir => "Elixir",
            AgentLanguage::Zig => "Zig",
            AgentLanguage::Lua => "Lua",
        }
    }

    pub fn all() -> &'static [AgentLanguage] {
        &[
            AgentLanguage::Rust,
            AgentLanguage::TypeScript,
            AgentLanguage::Python,
            AgentLanguage::Go,
            AgentLanguage::Java,
            AgentLanguage::CSharp,
            AgentLanguage::Ruby,
            AgentLanguage::Swift,
            AgentLanguage::Kotlin,
            AgentLanguage::Cpp,
            AgentLanguage::Elixir,
            AgentLanguage::Zig,
            AgentLanguage::Lua,
        ]
    }
}

pub(crate) fn agents_for_language(lang: AgentLanguage) -> Vec<AgentConfig> {
    match lang {
        AgentLanguage::Rust => vec![
            AgentConfig {
                kind: AgentKind::Format,
                name: String::new(),
                enabled: true,
                command: "cargo fmt".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Lint,
                name: String::new(),
                enabled: true,
                command: "cargo clippy --message-format=json 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Format),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Build,
                name: String::new(),
                enabled: true,
                command: "cargo build --message-format=json 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Test),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Test,
                name: String::new(),
                enabled: true,
                command: "cargo test 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Lint),
                timeout_secs: 300,
                working_dir: String::new(),
                before_run: String::new(),
            },
        ],
        AgentLanguage::TypeScript => vec![
            AgentConfig {
                kind: AgentKind::Format,
                name: String::new(),
                enabled: true,
                command: "npx prettier --write .".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Lint,
                name: String::new(),
                enabled: true,
                command: "npx eslint . 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Format),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Build,
                name: String::new(),
                enabled: true,
                command: "npx tsc --noEmit 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Test),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Test,
                name: String::new(),
                enabled: true,
                command: "npx jest 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Lint),
                timeout_secs: 300,
                working_dir: String::new(),
                before_run: String::new(),
            },
        ],
        AgentLanguage::Python => vec![
            AgentConfig {
                kind: AgentKind::Format,
                name: String::new(),
                enabled: true,
                command: "black .".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Lint,
                name: String::new(),
                enabled: true,
                command: "ruff check . 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Format),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Build,
                name: String::new(),
                enabled: true,
                command: "python -m py_compile *.py 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Test),
                timeout_secs: 60,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Test,
                name: String::new(),
                enabled: true,
                command: "pytest 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Lint),
                timeout_secs: 300,
                working_dir: String::new(),
                before_run: String::new(),
            },
        ],
        AgentLanguage::Go => vec![
            AgentConfig {
                kind: AgentKind::Format,
                name: String::new(),
                enabled: true,
                command: "gofmt -w .".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Lint,
                name: String::new(),
                enabled: true,
                command: "golangci-lint run 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Format),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Build,
                name: String::new(),
                enabled: true,
                command: "go build ./... 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Test),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Test,
                name: String::new(),
                enabled: true,
                command: "go test ./... 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Lint),
                timeout_secs: 300,
                working_dir: String::new(),
                before_run: String::new(),
            },
        ],
        AgentLanguage::Java => vec![
            AgentConfig {
                kind: AgentKind::Format,
                name: String::new(),
                enabled: true,
                command: "./mvnw com.diffplug.spotless:spotless-maven-plugin:apply 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 60,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Lint,
                name: String::new(),
                enabled: true,
                command: "mvn checkstyle:check 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Format),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Build,
                name: String::new(),
                enabled: true,
                command: "mvn compile 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Test),
                timeout_secs: 180,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Test,
                name: String::new(),
                enabled: true,
                command: "mvn test 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Lint),
                timeout_secs: 300,
                working_dir: String::new(),
                before_run: String::new(),
            },
        ],
        AgentLanguage::CSharp => vec![
            AgentConfig {
                kind: AgentKind::Format,
                name: String::new(),
                enabled: true,
                command: "dotnet format 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 60,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Lint,
                name: String::new(),
                enabled: true,
                command: "dotnet format --verify-no-changes 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Format),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Build,
                name: String::new(),
                enabled: true,
                command: "dotnet build 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Test),
                timeout_secs: 180,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Test,
                name: String::new(),
                enabled: true,
                command: "dotnet test 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Lint),
                timeout_secs: 300,
                working_dir: String::new(),
                before_run: String::new(),
            },
        ],
        AgentLanguage::Ruby => vec![
            AgentConfig {
                kind: AgentKind::Format,
                name: String::new(),
                enabled: true,
                command: "bundle exec rubocop -a 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 60,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Lint,
                name: String::new(),
                enabled: true,
                command: "bundle exec rubocop 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Format),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Build,
                name: String::new(),
                enabled: true,
                command: "ruby -c **/*.rb 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Test),
                timeout_secs: 60,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Test,
                name: String::new(),
                enabled: true,
                command: "bundle exec rspec 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Lint),
                timeout_secs: 300,
                working_dir: String::new(),
                before_run: String::new(),
            },
        ],
        AgentLanguage::Swift => vec![
            AgentConfig {
                kind: AgentKind::Format,
                name: String::new(),
                enabled: true,
                command: "swift-format format -i -r . 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Lint,
                name: String::new(),
                enabled: true,
                command: "swiftlint 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Format),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Build,
                name: String::new(),
                enabled: true,
                command: "swift build 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Test),
                timeout_secs: 180,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Test,
                name: String::new(),
                enabled: true,
                command: "swift test 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Lint),
                timeout_secs: 300,
                working_dir: String::new(),
                before_run: String::new(),
            },
        ],
        AgentLanguage::Kotlin => vec![
            AgentConfig {
                kind: AgentKind::Format,
                name: String::new(),
                enabled: true,
                command: "ktlint --format 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 60,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Lint,
                name: String::new(),
                enabled: true,
                command: "ktlint 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Format),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Build,
                name: String::new(),
                enabled: true,
                command: "./gradlew compileKotlin 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Test),
                timeout_secs: 180,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Test,
                name: String::new(),
                enabled: true,
                command: "./gradlew test 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Lint),
                timeout_secs: 300,
                working_dir: String::new(),
                before_run: String::new(),
            },
        ],
        AgentLanguage::Cpp => vec![
            AgentConfig {
                kind: AgentKind::Format,
                name: String::new(),
                enabled: true,
                command: "find . -name '*.cpp' -o -name '*.h' | xargs clang-format -i".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Lint,
                name: String::new(),
                enabled: true,
                command: "cppcheck --enable=all . 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Format),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Build,
                name: String::new(),
                enabled: true,
                command: "cmake --build build 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Test),
                timeout_secs: 180,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Test,
                name: String::new(),
                enabled: true,
                command: "ctest --test-dir build 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Lint),
                timeout_secs: 300,
                working_dir: String::new(),
                before_run: String::new(),
            },
        ],
        AgentLanguage::Elixir => vec![
            AgentConfig {
                kind: AgentKind::Format,
                name: String::new(),
                enabled: true,
                command: "mix format".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Lint,
                name: String::new(),
                enabled: true,
                command: "mix credo 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Format),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Build,
                name: String::new(),
                enabled: true,
                command: "mix compile 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Test),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Test,
                name: String::new(),
                enabled: true,
                command: "mix test 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Lint),
                timeout_secs: 300,
                working_dir: String::new(),
                before_run: String::new(),
            },
        ],
        AgentLanguage::Zig => vec![
            AgentConfig {
                kind: AgentKind::Format,
                name: String::new(),
                enabled: true,
                command: "zig fmt .".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Lint,
                name: String::new(),
                enabled: true,
                command: "zig build 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Format),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Build,
                name: String::new(),
                enabled: true,
                command: "zig build 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Test),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Test,
                name: String::new(),
                enabled: true,
                command: "zig build test 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Lint),
                timeout_secs: 300,
                working_dir: String::new(),
                before_run: String::new(),
            },
        ],
        AgentLanguage::Lua => vec![
            AgentConfig {
                kind: AgentKind::Format,
                name: String::new(),
                enabled: true,
                command: "stylua .".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Lint,
                name: String::new(),
                enabled: true,
                command: "luacheck . 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Format),
                timeout_secs: 120,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Build,
                name: String::new(),
                enabled: true,
                command: "luac -p *.lua 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Test),
                timeout_secs: 60,
                working_dir: String::new(),
                before_run: String::new(),
            },
            AgentConfig {
                kind: AgentKind::Test,
                name: String::new(),
                enabled: true,
                command: "busted 2>&1".into(),
                trigger: AgentTrigger::AfterAgent(AgentKind::Lint),
                timeout_secs: 300,
                working_dir: String::new(),
                before_run: String::new(),
            },
        ],
    }
}

// ---------------------------------------------------------------------------
// Diagnostic (parsed from cargo JSON output)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Diagnostic {
    pub file: String,
    pub line: usize,
    pub col: Option<usize>,
    pub message: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Severity {
    Error,
    Warning,
    Info,
}

// ---------------------------------------------------------------------------
// Agent result (sent from worker thread back to main)
// ---------------------------------------------------------------------------

pub(crate) struct AgentResult {
    pub kind: AgentKind,
    pub cue_id: Option<i64>,
    pub status: AgentStatus,
    pub output: String,
    pub diagnostics: Vec<Diagnostic>,
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Agent run state (lives inside DirigentApp)
// ---------------------------------------------------------------------------

/// Info about the most recent completed run for an agent.
pub(crate) struct LastRunInfo {
    pub duration_ms: u64,
    pub finished_at: Instant,
}

pub(crate) struct AgentRunState {
    pub tx: mpsc::Sender<AgentResult>,
    pub rx: mpsc::Receiver<AgentResult>,
    /// Latest status per agent kind (for status bar display).
    pub statuses: HashMap<AgentKind, AgentStatus>,
    /// Latest output per agent kind (for detail panel).
    pub latest_output: HashMap<AgentKind, String>,
    /// Latest diagnostics per agent kind.
    pub latest_diagnostics: HashMap<AgentKind, Vec<Diagnostic>>,
    /// Info about the last completed run per agent kind.
    pub last_run: HashMap<AgentKind, LastRunInfo>,
    /// Cancel flags for running agents.
    pub cancel_flags: HashMap<AgentKind, Arc<AtomicBool>>,
    /// Which agent's output panel is currently shown (None = hidden).
    pub show_output: Option<AgentKind>,
    /// When true, the Back button in agent log returns to settings.
    pub return_to_settings: bool,
}

impl AgentRunState {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        AgentRunState {
            tx,
            rx,
            statuses: HashMap::new(),
            latest_output: HashMap::new(),
            latest_diagnostics: HashMap::new(),
            last_run: HashMap::new(),
            cancel_flags: HashMap::new(),
            show_output: None,
            return_to_settings: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Run an agent (called from a worker thread)
// ---------------------------------------------------------------------------

/// Execute a single agent command. This is meant to be called from a spawned
/// thread — it blocks until the command finishes or times out.
///
/// `shell_init` is an optional shell snippet (from settings) prepended to the
/// command so that macOS GUI apps can source profiles, set PATH, JAVA_HOME, etc.
pub(crate) fn run_agent(
    config: &AgentConfig,
    project_root: &Path,
    shell_init: &str,
    cue_id: Option<i64>,
    prompt: &str,
    tx: &mpsc::Sender<AgentResult>,
    cancel: &Arc<AtomicBool>,
) {
    let start = Instant::now();
    let kind = config.kind;
    let timeout = Duration::from_secs(config.timeout_secs);

    // Build effective command: optional shell init + the agent command
    let effective_cmd = if shell_init.trim().is_empty() {
        config.command.clone()
    } else {
        format!("{}\n{}", shell_init.trim(), config.command)
    };

    // Working directory: project root + optional subdirectory.
    // Reject paths that escape the project root (e.g. via "..").
    let cwd = if config.working_dir.trim().is_empty() {
        project_root.to_path_buf()
    } else {
        let candidate = project_root.join(config.working_dir.trim());
        let resolved = candidate.canonicalize().unwrap_or(candidate.clone());
        let root_resolved = project_root
            .canonicalize()
            .unwrap_or(project_root.to_path_buf());
        if !resolved.starts_with(&root_resolved) {
            let _ = tx.send(AgentResult {
                kind,
                cue_id,
                status: AgentStatus::Error,
                output: format!("working_dir '{}' escapes project root", config.working_dir),
                diagnostics: Vec::new(),
                duration_ms: start.elapsed().as_millis() as u64,
            });
            return;
        }
        candidate
    };

    // Execute before_run hook if configured.
    // The prompt is available as $PROMPT env var and expanded in the command.
    if !config.before_run.trim().is_empty() {
        let before_cmd = config.before_run.replace("$PROMPT", prompt);
        let before_effective = if shell_init.trim().is_empty() {
            before_cmd
        } else {
            format!("{}\n{}", shell_init.trim(), before_cmd)
        };
        let before_result = Command::new("sh")
            .arg("-c")
            .arg(&before_effective)
            .current_dir(&cwd)
            .env("PROMPT", prompt)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();

        match before_result {
            Ok(output) if !output.status.success() => {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let combined = if stderr.is_empty() { stdout } else { stderr };
                let _ = tx.send(AgentResult {
                    kind,
                    cue_id,
                    status: AgentStatus::Error,
                    output: format!("before_run failed (exit {}):\n{}", output.status, combined),
                    diagnostics: Vec::new(),
                    duration_ms: start.elapsed().as_millis() as u64,
                });
                return;
            }
            Err(e) => {
                let _ = tx.send(AgentResult {
                    kind,
                    cue_id,
                    status: AgentStatus::Error,
                    output: format!("before_run failed to execute: {}", e),
                    diagnostics: Vec::new(),
                    duration_ms: start.elapsed().as_millis() as u64,
                });
                return;
            }
            _ => {} // success — continue to main command
        }
    }

    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&effective_cmd)
        .current_dir(&cwd)
        .env("PROMPT", prompt)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // On Unix, create a new process group so we can kill the entire tree on timeout
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let child = cmd.spawn();

    match child {
        Ok(mut child) => {
            // Wait with timeout enforcement
            let result = wait_with_timeout(&mut child, timeout, cancel);
            let duration_ms = start.elapsed().as_millis() as u64;

            match result {
                WaitResult::Completed(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    let combined = if stderr.is_empty() {
                        stdout.clone()
                    } else if stdout.is_empty() {
                        stderr.clone()
                    } else {
                        format!("{}\n{}", stdout, stderr)
                    };

                    let status = match kind {
                        // Lint & Format: a completed run is always "passed" —
                        // findings are reported via diagnostics, not as agent failure.
                        AgentKind::Lint | AgentKind::Format => AgentStatus::Passed,
                        // Build, Test, and Custom: exit code determines pass/fail.
                        _ => {
                            if output.status.success() {
                                AgentStatus::Passed
                            } else {
                                AgentStatus::Failed
                            }
                        }
                    };

                    // Parse diagnostics from output (cargo JSON for Rust, generic patterns for others)
                    let diagnostics = match kind {
                        AgentKind::Lint
                        | AgentKind::Build
                        | AgentKind::Test
                        | AgentKind::Custom(_) => {
                            let cargo_diags = parse_cargo_diagnostics(&stdout);
                            if cargo_diags.is_empty() {
                                parse_generic_diagnostics(&combined)
                            } else {
                                cargo_diags
                            }
                        }
                        _ => Vec::new(),
                    };

                    let _ = tx.send(AgentResult {
                        kind,
                        cue_id,
                        status,
                        output: combined,
                        diagnostics,
                        duration_ms,
                    });
                }
                WaitResult::TimedOut => {
                    // Kill the process group on timeout
                    kill_process_tree(&child);
                    let _ = child.wait(); // reap zombie
                    let _ = tx.send(AgentResult {
                        kind,
                        cue_id,
                        status: AgentStatus::Error,
                        output: format!(
                            "Agent timed out after {}s (limit: {}s)",
                            duration_ms / 1000,
                            config.timeout_secs
                        ),
                        diagnostics: Vec::new(),
                        duration_ms,
                    });
                }
                WaitResult::Cancelled => {
                    kill_process_tree(&child);
                    let _ = child.wait();
                    let _ = tx.send(AgentResult {
                        kind,
                        cue_id,
                        status: AgentStatus::Error,
                        output: "Cancelled by user".to_string(),
                        diagnostics: Vec::new(),
                        duration_ms,
                    });
                }
            }
        }
        Err(e) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let _ = tx.send(AgentResult {
                kind,
                cue_id,
                status: AgentStatus::Error,
                output: format!("Failed to execute command: {}", e),
                diagnostics: Vec::new(),
                duration_ms,
            });
        }
    }
}

enum WaitResult {
    Completed(std::process::Output),
    TimedOut,
    Cancelled,
}

/// Wait for a child process with a timeout, polling every 100ms.
/// Drains stdout/stderr in background threads to avoid pipe buffer deadlocks.
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
    cancel: &Arc<AtomicBool>,
) -> WaitResult {
    use std::io::Read;

    // Spawn threads to drain stdout/stderr so the pipe buffers don't fill up
    // and block the child process (classic pipe deadlock).
    let stdout_handle = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out.read_to_end(&mut buf);
            buf
        })
    });
    let stderr_handle = child.stderr.take().map(|mut err| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = err.read_to_end(&mut buf);
            buf
        })
    });

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = stdout_handle
                    .and_then(|h| h.join().ok())
                    .unwrap_or_default();
                let stderr = stderr_handle
                    .and_then(|h| h.join().ok())
                    .unwrap_or_default();
                return WaitResult::Completed(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if cancel.load(Ordering::Relaxed) {
                    return WaitResult::Cancelled;
                }
                if start.elapsed() >= timeout {
                    return WaitResult::TimedOut;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => {
                return WaitResult::TimedOut;
            }
        }
    }
}

/// Kill the entire process tree (process group) on Unix, or just the child on other platforms.
fn kill_process_tree(child: &std::process::Child) {
    let pid = child.id();
    #[cfg(unix)]
    {
        // Kill the entire process group (negative PID)
        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
        }
        // Give processes a moment to clean up, then force kill
        std::thread::sleep(Duration::from_millis(500));
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid; // suppress unused warning
                     // On non-Unix, just kill the direct child (best effort)
                     // child.kill() requires &mut, so we can't call it here
    }
}

// ---------------------------------------------------------------------------
// Trigger agents matching a given trigger condition
// ---------------------------------------------------------------------------

/// Spawn agents that match the given trigger. Returns the number of agents started.
pub(crate) fn trigger_agents(
    agents: &[AgentConfig],
    trigger: &AgentTrigger,
    project_root: &Path,
    shell_init: &str,
    cue_id: Option<i64>,
    prompt: &str,
    tx: &mpsc::Sender<AgentResult>,
    statuses: &mut HashMap<AgentKind, AgentStatus>,
    cancel_flags: &mut HashMap<AgentKind, Arc<AtomicBool>>,
) -> usize {
    let mut count = 0;
    for config in agents {
        if !config.enabled {
            continue;
        }
        let matches = match (&config.trigger, trigger) {
            (AgentTrigger::AfterAgent(k1), AgentTrigger::AfterAgent(k2)) => k1 == k2,
            (a, b) => a == b,
        };
        if !matches {
            continue;
        }
        // Don't start an agent that's already running
        if statuses.get(&config.kind) == Some(&AgentStatus::Running) {
            continue;
        }
        statuses.insert(config.kind, AgentStatus::Running);
        let cancel = Arc::new(AtomicBool::new(false));
        cancel_flags.insert(config.kind, Arc::clone(&cancel));

        let config = config.clone();
        let root = project_root.to_path_buf();
        let init = shell_init.to_string();
        let prompt = prompt.to_string();
        let tx = tx.clone();

        std::thread::spawn(move || {
            run_agent(&config, &root, &init, cue_id, &prompt, &tx, &cancel);
        });

        count += 1;
    }
    count
}

// ---------------------------------------------------------------------------
// Cargo JSON diagnostic parser
// ---------------------------------------------------------------------------

/// Parse compiler/clippy diagnostics from `cargo --message-format=json` output.
fn parse_cargo_diagnostics(output: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
            // Cargo wraps compiler messages in {"reason":"compiler-message","message":{...}}
            let msg = if value.get("reason").and_then(|r| r.as_str()) == Some("compiler-message") {
                value.get("message")
            } else {
                // Direct rustc JSON diagnostic
                Some(&value)
            };

            if let Some(msg) = msg {
                if let Some(message_text) = msg.get("message").and_then(|m| m.as_str()) {
                    let severity = match msg.get("level").and_then(|l| l.as_str()) {
                        Some("error") => Severity::Error,
                        Some("warning") => Severity::Warning,
                        _ => Severity::Info,
                    };

                    // Get the primary span
                    if let Some(spans) = msg.get("spans").and_then(|s| s.as_array()) {
                        for span in spans {
                            let is_primary =
                                span.get("is_primary").and_then(|p| p.as_bool()) == Some(true);
                            if !is_primary && spans.len() > 1 {
                                continue;
                            }
                            if let (Some(file), Some(line)) = (
                                span.get("file_name").and_then(|f| f.as_str()),
                                span.get("line_start").and_then(|l| l.as_u64()),
                            ) {
                                let col = span
                                    .get("column_start")
                                    .and_then(|c| c.as_u64())
                                    .map(|c| c as usize);
                                diagnostics.push(Diagnostic {
                                    file: file.to_string(),
                                    line: line as usize,
                                    col,
                                    message: message_text.to_string(),
                                    severity,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    diagnostics
}

// ---------------------------------------------------------------------------
// Generic diagnostic parser (file:line:col: severity: message)
// ---------------------------------------------------------------------------

/// Parse diagnostics from generic compiler/linter output using common patterns:
/// - `file:line:col: error: message` (gcc, clang, rustc, tsc, swiftc)
/// - `file:line:col: warning: message`
/// - `file:line: error: message` (without column)
/// - `file(line,col): error message` (MSVC-style)
/// - `file:line: message` (generic, treated as error if process failed)
fn parse_generic_diagnostics(output: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Pattern: file:line:col: severity: message
    // Matches: src/main.rs:10:5: error: something went wrong
    //          src/app.ts(15,3): error TS2345: something
    let re = Regex::new(
        r"(?m)^(.+?):(\d+)(?::(\d+))?:\s*(?:(error|warning|warn|info|note|hint))(?:\[.*?\])?:\s*(.+)$"
    ).unwrap();

    // MSVC / TypeScript pattern: file(line,col): error CODE: message
    let re_paren =
        Regex::new(r"(?m)^(.+?)\((\d+),(\d+)\):\s*(?:(error|warning))(?:\s+\w+)?:\s*(.+)$")
            .unwrap();

    for cap in re.captures_iter(output) {
        let file = cap[1].to_string();
        // Skip lines that look like URLs or stack traces
        if file.starts_with("http") || file.starts_with("    ") || file.starts_with("\t") {
            continue;
        }
        let line: usize = cap[2].parse().unwrap_or(0);
        if line == 0 {
            continue;
        }
        let col = cap.get(3).and_then(|m| m.as_str().parse().ok());
        let severity = match &cap[4] {
            "error" => Severity::Error,
            "warning" | "warn" => Severity::Warning,
            _ => Severity::Info,
        };
        let message = cap[5].trim().to_string();
        diagnostics.push(Diagnostic {
            file,
            line,
            col,
            message,
            severity,
        });
    }

    for cap in re_paren.captures_iter(output) {
        let file = cap[1].to_string();
        let line: usize = cap[2].parse().unwrap_or(0);
        if line == 0 {
            continue;
        }
        let col: Option<usize> = cap[3].parse().ok();
        let severity = match &cap[4] {
            "error" => Severity::Error,
            _ => Severity::Warning,
        };
        let message = cap[5].trim().to_string();
        diagnostics.push(Diagnostic {
            file,
            line,
            col,
            message,
            severity,
        });
    }

    diagnostics
}

#[cfg(test)]
impl AgentKind {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "format" => Some(AgentKind::Format),
            "lint" => Some(AgentKind::Lint),
            "build" => Some(AgentKind::Build),
            "test" => Some(AgentKind::Test),
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

    #[test]
    fn agent_kind_roundtrip() {
        for kind in AgentKind::builtins() {
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
    fn parse_cargo_diagnostics_empty() {
        assert!(parse_cargo_diagnostics("").is_empty());
        assert!(parse_cargo_diagnostics("not json").is_empty());
    }

    #[test]
    fn parse_cargo_compiler_message() {
        let json = r#"{"reason":"compiler-message","message":{"message":"unused variable: `x`","level":"warning","spans":[{"file_name":"src/main.rs","line_start":10,"column_start":5,"is_primary":true}]}}"#;
        let diags = parse_cargo_diagnostics(json);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].file, "src/main.rs");
        assert_eq!(diags[0].line, 10);
        assert_eq!(diags[0].col, Some(5));
        assert_eq!(diags[0].severity, Severity::Warning);
    }

    #[test]
    fn default_agents_is_empty() {
        let agents = default_agents();
        assert!(agents.is_empty());
    }

    #[test]
    fn parse_generic_gcc_style() {
        let output = "src/main.c:42:10: error: expected ';' after expression\n";
        let diags = parse_generic_diagnostics(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].file, "src/main.c");
        assert_eq!(diags[0].line, 42);
        assert_eq!(diags[0].col, Some(10));
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].message, "expected ';' after expression");
    }

    #[test]
    fn parse_generic_warning() {
        let output = "lib/utils.py:15:1: warning: unused import 'os'\n";
        let diags = parse_generic_diagnostics(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Warning);
    }

    #[test]
    fn parse_generic_no_column() {
        let output = "src/app.rs:100: error: something broke\n";
        let diags = parse_generic_diagnostics(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].col, None);
    }

    #[test]
    fn parse_generic_msvc_style() {
        let output =
            "src/app.ts(15,3): error TS2345: Argument of type 'string' is not assignable\n";
        let diags = parse_generic_diagnostics(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].file, "src/app.ts");
        assert_eq!(diags[0].line, 15);
        assert_eq!(diags[0].col, Some(3));
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn parse_generic_empty() {
        assert!(parse_generic_diagnostics("").is_empty());
        assert!(parse_generic_diagnostics("all good!").is_empty());
    }
}
