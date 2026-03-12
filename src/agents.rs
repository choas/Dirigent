use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Agent kinds
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum AgentKind {
    Format,
    Lint,
    Build,
    Test,
}

impl AgentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentKind::Format => "format",
            AgentKind::Lint => "lint",
            AgentKind::Build => "build",
            AgentKind::Test => "test",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            AgentKind::Format => "Format",
            AgentKind::Lint => "Lint",
            AgentKind::Build => "Build",
            AgentKind::Test => "Test",
        }
    }

    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "format" => Some(AgentKind::Format),
            "lint" => Some(AgentKind::Lint),
            "build" => Some(AgentKind::Build),
            "test" => Some(AgentKind::Test),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn all() -> &'static [AgentKind] {
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
    /// Only run when manually triggered.
    Manual,
}

impl AgentTrigger {
    pub fn display_name(&self) -> &str {
        match self {
            AgentTrigger::AfterRun => "After Run",
            AgentTrigger::AfterCommit => "After Commit",
            AgentTrigger::AfterAgent(_) => "After Agent",
            AgentTrigger::Manual => "Manual",
        }
    }

    /// The base variants used for the trigger type selector (without inner data).
    pub fn base_variants() -> &'static [AgentTrigger] {
        &[
            AgentTrigger::AfterRun,
            AgentTrigger::AfterCommit,
            AgentTrigger::AfterAgent(AgentKind::Format), // placeholder
            AgentTrigger::Manual,
        ]
    }

    /// Returns the discriminant index for comparison in the UI selector.
    pub fn variant_index(&self) -> usize {
        match self {
            AgentTrigger::AfterRun => 0,
            AgentTrigger::AfterCommit => 1,
            AgentTrigger::AfterAgent(_) => 2,
            AgentTrigger::Manual => 3,
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
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentStatus::Idle => "idle",
            AgentStatus::Running => "running",
            AgentStatus::Passed => "passed",
            AgentStatus::Failed => "failed",
            AgentStatus::Error => "error",
        }
    }

    #[allow(dead_code)]
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

// ---------------------------------------------------------------------------
// Agent configuration (persisted in settings)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AgentConfig {
    pub kind: AgentKind,
    pub enabled: bool,
    pub command: String,
    pub trigger: AgentTrigger,
    pub timeout_secs: u64,
}

pub(crate) fn default_agents() -> Vec<AgentConfig> {
    Vec::new()
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
        ]
    }
}

pub(crate) fn agents_for_language(lang: AgentLanguage) -> Vec<AgentConfig> {
    match lang {
        AgentLanguage::Rust => vec![
            AgentConfig {
                kind: AgentKind::Format,
                enabled: true,
                command: "cargo fmt".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
            },
            AgentConfig {
                kind: AgentKind::Lint,
                enabled: false,
                command: "cargo clippy --message-format=json 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Build,
                enabled: false,
                command: "cargo build --message-format=json 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Test,
                enabled: false,
                command: "cargo test 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 300,
            },
        ],
        AgentLanguage::TypeScript => vec![
            AgentConfig {
                kind: AgentKind::Format,
                enabled: true,
                command: "npx prettier --write .".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
            },
            AgentConfig {
                kind: AgentKind::Lint,
                enabled: false,
                command: "npx eslint . 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Build,
                enabled: false,
                command: "npx tsc --noEmit 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Test,
                enabled: false,
                command: "npx jest 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 300,
            },
        ],
        AgentLanguage::Python => vec![
            AgentConfig {
                kind: AgentKind::Format,
                enabled: true,
                command: "black .".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
            },
            AgentConfig {
                kind: AgentKind::Lint,
                enabled: false,
                command: "ruff check . 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Build,
                enabled: false,
                command: "python -m py_compile *.py 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 60,
            },
            AgentConfig {
                kind: AgentKind::Test,
                enabled: false,
                command: "pytest 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 300,
            },
        ],
        AgentLanguage::Go => vec![
            AgentConfig {
                kind: AgentKind::Format,
                enabled: true,
                command: "gofmt -w .".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
            },
            AgentConfig {
                kind: AgentKind::Lint,
                enabled: false,
                command: "golangci-lint run 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Build,
                enabled: false,
                command: "go build ./... 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Test,
                enabled: false,
                command: "go test ./... 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 300,
            },
        ],
        AgentLanguage::Java => vec![
            AgentConfig {
                kind: AgentKind::Format,
                enabled: true,
                command: "mvn spotless:apply 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 60,
            },
            AgentConfig {
                kind: AgentKind::Lint,
                enabled: false,
                command: "mvn checkstyle:check 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Build,
                enabled: false,
                command: "mvn compile 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 180,
            },
            AgentConfig {
                kind: AgentKind::Test,
                enabled: false,
                command: "mvn test 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 300,
            },
        ],
        AgentLanguage::CSharp => vec![
            AgentConfig {
                kind: AgentKind::Format,
                enabled: true,
                command: "dotnet format 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 60,
            },
            AgentConfig {
                kind: AgentKind::Lint,
                enabled: false,
                command: "dotnet format --verify-no-changes 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Build,
                enabled: false,
                command: "dotnet build 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 180,
            },
            AgentConfig {
                kind: AgentKind::Test,
                enabled: false,
                command: "dotnet test 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 300,
            },
        ],
        AgentLanguage::Ruby => vec![
            AgentConfig {
                kind: AgentKind::Format,
                enabled: true,
                command: "bundle exec rubocop -a 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 60,
            },
            AgentConfig {
                kind: AgentKind::Lint,
                enabled: false,
                command: "bundle exec rubocop 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Build,
                enabled: false,
                command: "ruby -c **/*.rb 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 60,
            },
            AgentConfig {
                kind: AgentKind::Test,
                enabled: false,
                command: "bundle exec rspec 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 300,
            },
        ],
        AgentLanguage::Swift => vec![
            AgentConfig {
                kind: AgentKind::Format,
                enabled: true,
                command: "swift-format format -i -r . 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
            },
            AgentConfig {
                kind: AgentKind::Lint,
                enabled: false,
                command: "swiftlint 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Build,
                enabled: false,
                command: "swift build 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 180,
            },
            AgentConfig {
                kind: AgentKind::Test,
                enabled: false,
                command: "swift test 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 300,
            },
        ],
        AgentLanguage::Kotlin => vec![
            AgentConfig {
                kind: AgentKind::Format,
                enabled: true,
                command: "ktlint --format 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 60,
            },
            AgentConfig {
                kind: AgentKind::Lint,
                enabled: false,
                command: "ktlint 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Build,
                enabled: false,
                command: "./gradlew compileKotlin 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 180,
            },
            AgentConfig {
                kind: AgentKind::Test,
                enabled: false,
                command: "./gradlew test 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 300,
            },
        ],
        AgentLanguage::Cpp => vec![
            AgentConfig {
                kind: AgentKind::Format,
                enabled: true,
                command: "find . -name '*.cpp' -o -name '*.h' | xargs clang-format -i".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
            },
            AgentConfig {
                kind: AgentKind::Lint,
                enabled: false,
                command: "cppcheck --enable=all . 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Build,
                enabled: false,
                command: "cmake --build build 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 180,
            },
            AgentConfig {
                kind: AgentKind::Test,
                enabled: false,
                command: "ctest --test-dir build 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 300,
            },
        ],
        AgentLanguage::Elixir => vec![
            AgentConfig {
                kind: AgentKind::Format,
                enabled: true,
                command: "mix format".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
            },
            AgentConfig {
                kind: AgentKind::Lint,
                enabled: false,
                command: "mix credo 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Build,
                enabled: false,
                command: "mix compile 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Test,
                enabled: false,
                command: "mix test 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 300,
            },
        ],
        AgentLanguage::Zig => vec![
            AgentConfig {
                kind: AgentKind::Format,
                enabled: true,
                command: "zig fmt .".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 30,
            },
            AgentConfig {
                kind: AgentKind::Lint,
                enabled: false,
                command: "zig build 2>&1".into(),
                trigger: AgentTrigger::AfterRun,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Build,
                enabled: false,
                command: "zig build 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 120,
            },
            AgentConfig {
                kind: AgentKind::Test,
                enabled: false,
                command: "zig build test 2>&1".into(),
                trigger: AgentTrigger::Manual,
                timeout_secs: 300,
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

pub(crate) struct AgentRunState {
    pub tx: mpsc::Sender<AgentResult>,
    pub rx: mpsc::Receiver<AgentResult>,
    /// Latest status per agent kind (for status bar display).
    pub statuses: HashMap<AgentKind, AgentStatus>,
    /// Latest output per agent kind (for detail panel).
    pub latest_output: HashMap<AgentKind, String>,
    /// Latest diagnostics per agent kind.
    pub latest_diagnostics: HashMap<AgentKind, Vec<Diagnostic>>,
    /// Which agent's output panel is currently shown (None = hidden).
    pub show_output: Option<AgentKind>,
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
            show_output: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Run an agent (called from a worker thread)
// ---------------------------------------------------------------------------

/// Execute a single agent command. This is meant to be called from a spawned
/// thread — it blocks until the command finishes or times out.
pub(crate) fn run_agent(
    config: &AgentConfig,
    project_root: &Path,
    cue_id: Option<i64>,
    tx: &mpsc::Sender<AgentResult>,
) {
    let start = Instant::now();
    let kind = config.kind;

    let result = Command::new("sh")
        .arg("-c")
        .arg(&config.command)
        .current_dir(project_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    let duration_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let combined = if stderr.is_empty() {
                stdout.clone()
            } else if stdout.is_empty() {
                stderr.clone()
            } else {
                format!("{}\n{}", stdout, stderr)
            };

            let status = if output.status.success() {
                AgentStatus::Passed
            } else {
                AgentStatus::Failed
            };

            // Parse diagnostics from cargo JSON output (for lint/build)
            let diagnostics = match kind {
                AgentKind::Lint | AgentKind::Build => parse_cargo_diagnostics(&stdout),
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
        Err(e) => {
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

// ---------------------------------------------------------------------------
// Trigger agents matching a given trigger condition
// ---------------------------------------------------------------------------

/// Spawn agents that match the given trigger. Returns the number of agents started.
pub(crate) fn trigger_agents(
    agents: &[AgentConfig],
    trigger: &AgentTrigger,
    project_root: &Path,
    cue_id: Option<i64>,
    tx: &mpsc::Sender<AgentResult>,
    statuses: &mut HashMap<AgentKind, AgentStatus>,
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

        let config = config.clone();
        let root = project_root.to_path_buf();
        let tx = tx.clone();

        std::thread::spawn(move || {
            run_agent(&config, &root, cue_id, &tx);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_kind_roundtrip() {
        for kind in AgentKind::all() {
            let s = kind.as_str();
            assert_eq!(AgentKind::from_str(s), Some(*kind));
        }
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
}
