use super::types::{AgentConfig, AgentKind, AgentTrigger};

fn agent(kind: AgentKind, command: &str, trigger: AgentTrigger, timeout_secs: u64) -> AgentConfig {
    AgentConfig {
        kind,
        name: String::new(),
        enabled: true,
        command: command.into(),
        trigger,
        timeout_secs,
        working_dir: String::new(),
        before_run: String::new(),
    }
}

fn audit_agent(command: &str) -> AgentConfig {
    agent(AgentKind::Audit, command, AgentTrigger::Manual, 120)
}

fn outdated_agent(command: &str, timeout: u64) -> AgentConfig {
    agent(AgentKind::Outdated, command, AgentTrigger::Manual, timeout)
}

/// Standard pipeline: Format → Lint → Build → Test (each chained via AfterAgent).
fn pipeline(
    fmt_cmd: &str,
    fmt_timeout: u64,
    lint_cmd: &str,
    lint_timeout: u64,
    build_cmd: &str,
    build_timeout: u64,
    test_cmd: &str,
    test_timeout: u64,
) -> Vec<AgentConfig> {
    vec![
        agent(
            AgentKind::Format,
            fmt_cmd,
            AgentTrigger::AfterRun,
            fmt_timeout,
        ),
        agent(
            AgentKind::Lint,
            lint_cmd,
            AgentTrigger::AfterAgent(AgentKind::Format),
            lint_timeout,
        ),
        agent(
            AgentKind::Build,
            build_cmd,
            AgentTrigger::AfterAgent(AgentKind::Lint),
            build_timeout,
        ),
        agent(
            AgentKind::Test,
            test_cmd,
            AgentTrigger::AfterAgent(AgentKind::Build),
            test_timeout,
        ),
    ]
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
        AgentLanguage::Rust => {
            let mut v = pipeline(
                "cargo fmt",
                30,
                "cargo clippy --message-format=json 2>&1",
                120,
                "cargo build --message-format=json 2>&1",
                120,
                "cargo test 2>&1",
                300,
            );
            v.push(outdated_agent("cargo outdated 2>&1", 120));
            v.push(audit_agent("cargo audit 2>&1"));
            v
        }
        AgentLanguage::TypeScript => {
            let mut v = pipeline(
                "npx prettier --write .",
                30,
                "npx eslint . 2>&1",
                120,
                "npx tsc --noEmit 2>&1",
                120,
                "npx jest 2>&1",
                300,
            );
            v.push(outdated_agent("npm outdated 2>&1", 60));
            v.push(audit_agent("npm audit 2>&1"));
            v
        }
        AgentLanguage::Python => {
            let mut v = pipeline(
                "black .",
                30,
                "ruff check . 2>&1",
                120,
                "python -m py_compile *.py 2>&1",
                60,
                "pytest 2>&1",
                300,
            );
            v.push(outdated_agent("pip list --outdated 2>&1", 60));
            v.push(audit_agent("pip-audit 2>&1"));
            v
        }
        AgentLanguage::Go => {
            let mut v = pipeline(
                "gofmt -w .",
                30,
                "golangci-lint run 2>&1",
                120,
                "go build ./... 2>&1",
                120,
                "go test ./... 2>&1",
                300,
            );
            v.push(outdated_agent("go list -m -u all 2>&1", 60));
            v.push(audit_agent("govulncheck ./... 2>&1"));
            v
        }
        AgentLanguage::Java => {
            let mut v = pipeline(
                "./mvnw com.diffplug.spotless:spotless-maven-plugin:apply 2>&1",
                60,
                "mvn checkstyle:check 2>&1",
                120,
                "mvn compile 2>&1",
                180,
                "mvn test 2>&1",
                300,
            );
            v.push(outdated_agent(
                "mvn versions:display-dependency-updates 2>&1",
                120,
            ));
            v.push(audit_agent(
                "mvn org.owasp:dependency-check-maven:check 2>&1",
            ));
            v
        }
        AgentLanguage::CSharp => {
            let mut v = pipeline(
                "dotnet format 2>&1",
                60,
                "dotnet format --verify-no-changes 2>&1",
                120,
                "dotnet build 2>&1",
                180,
                "dotnet test 2>&1",
                300,
            );
            v.push(outdated_agent("dotnet list package --outdated 2>&1", 60));
            v.push(audit_agent("dotnet list package --vulnerable 2>&1"));
            v
        }
        AgentLanguage::Ruby => {
            let mut v = pipeline(
                "bundle exec rubocop -a 2>&1",
                60,
                "bundle exec rubocop 2>&1",
                120,
                "ruby -c **/*.rb 2>&1",
                60,
                "bundle exec rspec 2>&1",
                300,
            );
            v.push(outdated_agent("bundle outdated 2>&1", 60));
            v.push(audit_agent("bundle audit check 2>&1"));
            v
        }
        AgentLanguage::Swift => pipeline(
            "swift-format format -i -r . 2>&1",
            30,
            "swiftlint 2>&1",
            120,
            "swift build 2>&1",
            180,
            "swift test 2>&1",
            300,
        ),
        AgentLanguage::Kotlin => pipeline(
            "ktlint --format 2>&1",
            60,
            "ktlint 2>&1",
            120,
            "./gradlew compileKotlin 2>&1",
            180,
            "./gradlew test 2>&1",
            300,
        ),
        AgentLanguage::Cpp => pipeline(
            "find . -name '*.cpp' -o -name '*.h' | xargs clang-format -i",
            30,
            "cppcheck --enable=all . 2>&1",
            120,
            "cmake --build build 2>&1",
            180,
            "ctest --test-dir build 2>&1",
            300,
        ),
        AgentLanguage::Elixir => {
            let mut v = pipeline(
                "mix format",
                30,
                "mix credo 2>&1",
                120,
                "mix compile 2>&1",
                120,
                "mix test 2>&1",
                300,
            );
            v.push(outdated_agent("mix hex.outdated 2>&1", 60));
            v.push(audit_agent("mix hex.audit 2>&1"));
            v
        }
        AgentLanguage::Zig => pipeline(
            "zig fmt .",
            30,
            "zig build 2>&1",
            120,
            "zig build 2>&1",
            120,
            "zig build test 2>&1",
            300,
        ),
        AgentLanguage::Lua => pipeline(
            "stylua .",
            30,
            "luacheck . 2>&1",
            120,
            "luac -p *.lua 2>&1",
            60,
            "busted 2>&1",
            300,
        ),
    }
}
