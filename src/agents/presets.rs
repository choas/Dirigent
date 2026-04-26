use std::path::Path;

use super::types::{AgentConfig, AgentKind, AgentTrigger};

struct Step<'a> {
    cmd: &'a str,
    timeout: u64,
}

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
fn pipeline(fmt: Step, lint: Step, build: Step, test: Step) -> Vec<AgentConfig> {
    vec![
        agent(
            AgentKind::Format,
            fmt.cmd,
            AgentTrigger::AfterRun,
            fmt.timeout,
        ),
        agent(
            AgentKind::Lint,
            lint.cmd,
            AgentTrigger::AfterAgent(AgentKind::Format),
            lint.timeout,
        ),
        agent(
            AgentKind::Build,
            build.cmd,
            AgentTrigger::AfterAgent(AgentKind::Lint),
            build.timeout,
        ),
        agent(
            AgentKind::Test,
            test.cmd,
            AgentTrigger::AfterAgent(AgentKind::Build),
            test.timeout,
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

/// Inspects only the immediate children of `repo_root` (not recursive).
fn find_xcodeproj(repo_root: &Path) -> Option<String> {
    std::fs::read_dir(repo_root).ok()?.find_map(|entry| {
        let entry = entry.ok()?;
        if !entry.file_type().ok()?.is_dir() {
            return None;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        name.strip_suffix(".xcodeproj").map(|s| s.to_owned())
    })
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub(crate) fn agents_for_language(lang: AgentLanguage, repo_root: &Path) -> Vec<AgentConfig> {
    match lang {
        AgentLanguage::Rust => {
            let mut v = pipeline(
                Step {
                    cmd: "cargo fmt",
                    timeout: 30,
                },
                Step {
                    cmd: "cargo clippy --message-format=json 2>&1",
                    timeout: 120,
                },
                Step {
                    cmd: "cargo build --message-format=json 2>&1",
                    timeout: 120,
                },
                Step {
                    cmd: "cargo test 2>&1",
                    timeout: 300,
                },
            );
            v.push(outdated_agent("cargo outdated 2>&1", 120));
            v.push(audit_agent("cargo audit 2>&1"));
            v
        }
        AgentLanguage::TypeScript => {
            let mut v = pipeline(
                Step {
                    cmd: "npx prettier --write .",
                    timeout: 30,
                },
                Step {
                    cmd: "npx eslint . 2>&1",
                    timeout: 120,
                },
                Step {
                    cmd: "npx tsc --noEmit 2>&1",
                    timeout: 120,
                },
                Step {
                    cmd: "npx jest 2>&1",
                    timeout: 300,
                },
            );
            v.push(outdated_agent("npm outdated 2>&1", 60));
            v.push(audit_agent("npm audit 2>&1"));
            v
        }
        AgentLanguage::Python => {
            let mut v = pipeline(
                Step {
                    cmd: "black .",
                    timeout: 30,
                },
                Step {
                    cmd: "ruff check . 2>&1",
                    timeout: 120,
                },
                Step {
                    cmd: "python -m compileall -q . 2>&1",
                    timeout: 60,
                },
                Step {
                    cmd: "pytest 2>&1",
                    timeout: 300,
                },
            );
            v.push(outdated_agent("pip list --outdated 2>&1", 60));
            v.push(audit_agent("pip-audit 2>&1"));
            v
        }
        AgentLanguage::Go => {
            let mut v = pipeline(
                Step {
                    cmd: "gofmt -w .",
                    timeout: 30,
                },
                Step {
                    cmd: "golangci-lint run 2>&1",
                    timeout: 120,
                },
                Step {
                    cmd: "go build ./... 2>&1",
                    timeout: 120,
                },
                Step {
                    cmd: "go test ./... 2>&1",
                    timeout: 300,
                },
            );
            v.push(outdated_agent("go list -m -u all 2>&1", 60));
            v.push(audit_agent("govulncheck ./... 2>&1"));
            v
        }
        AgentLanguage::Java => {
            let mut v = pipeline(
                Step {
                    cmd: "./mvnw com.diffplug.spotless:spotless-maven-plugin:apply 2>&1",
                    timeout: 60,
                },
                Step {
                    cmd: "./mvnw checkstyle:check 2>&1",
                    timeout: 120,
                },
                Step {
                    cmd: "./mvnw compile 2>&1",
                    timeout: 180,
                },
                Step {
                    cmd: "./mvnw test 2>&1",
                    timeout: 300,
                },
            );
            v.push(outdated_agent(
                "./mvnw versions:display-dependency-updates 2>&1",
                120,
            ));
            v.push(audit_agent(
                "./mvnw org.owasp:dependency-check-maven:check 2>&1",
            ));
            v
        }
        AgentLanguage::CSharp => {
            let mut v = pipeline(
                Step {
                    cmd: "dotnet format 2>&1",
                    timeout: 60,
                },
                Step {
                    cmd: "dotnet format --verify-no-changes 2>&1",
                    timeout: 120,
                },
                Step {
                    cmd: "dotnet build 2>&1",
                    timeout: 180,
                },
                Step {
                    cmd: "dotnet test 2>&1",
                    timeout: 300,
                },
            );
            v.push(outdated_agent("dotnet list package --outdated 2>&1", 60));
            v.push(audit_agent("dotnet list package --vulnerable 2>&1"));
            v
        }
        AgentLanguage::Ruby => {
            let mut v = pipeline(
                Step {
                    cmd: "bundle exec rubocop -a 2>&1",
                    timeout: 60,
                },
                Step {
                    cmd: "bundle exec rubocop 2>&1",
                    timeout: 120,
                },
                Step {
                    cmd: "find . -name '*.rb' -exec ruby -c {} + 2>&1",
                    timeout: 60,
                },
                Step {
                    cmd: "bundle exec rspec 2>&1",
                    timeout: 300,
                },
            );
            v.push(outdated_agent("bundle outdated 2>&1", 60));
            v.push(audit_agent("bundle audit check 2>&1"));
            v
        }
        AgentLanguage::Swift => {
            if let Some(project_name) = find_xcodeproj(repo_root) {
                let xcodeproj = shell_quote(&format!("{project_name}.xcodeproj"));
                let scheme = shell_quote(&project_name);
                let build_cmd = format!(
                    "xcodebuild -project {xcodeproj} -scheme {scheme} \
                     -destination 'platform=iOS Simulator,name=iPhone 17 Pro' build 2>&1"
                );
                let test_cmd = format!(
                    "xcodebuild -project {xcodeproj} -scheme {scheme} \
                     -destination 'platform=iOS Simulator,name=iPhone 17 Pro' test 2>&1"
                );
                pipeline(
                    Step {
                        cmd: "xcrun swift-format format -i -r . 2>&1",
                        timeout: 30,
                    },
                    Step {
                        cmd: "swiftlint 2>&1",
                        timeout: 120,
                    },
                    Step {
                        cmd: &build_cmd,
                        timeout: 180,
                    },
                    Step {
                        cmd: &test_cmd,
                        timeout: 300,
                    },
                )
            } else {
                pipeline(
                    Step {
                        cmd: "xcrun swift-format format -i -r . 2>&1",
                        timeout: 30,
                    },
                    Step {
                        cmd: "swiftlint 2>&1",
                        timeout: 120,
                    },
                    Step {
                        cmd: "swift build 2>&1",
                        timeout: 180,
                    },
                    Step {
                        cmd: "swift test 2>&1",
                        timeout: 300,
                    },
                )
            }
        }
        AgentLanguage::Kotlin => pipeline(
            Step {
                cmd: "ktlint --format 2>&1",
                timeout: 60,
            },
            Step {
                cmd: "ktlint 2>&1",
                timeout: 120,
            },
            Step {
                cmd: "./gradlew compileKotlin 2>&1",
                timeout: 180,
            },
            Step {
                cmd: "./gradlew test 2>&1",
                timeout: 300,
            },
        ),
        AgentLanguage::Cpp => pipeline(
            Step {
                cmd: "find . -name '*.cpp' -o -name '*.h' | xargs clang-format -i",
                timeout: 30,
            },
            Step {
                cmd: "cppcheck --enable=all . 2>&1",
                timeout: 120,
            },
            Step {
                cmd: "cmake --build build 2>&1",
                timeout: 180,
            },
            Step {
                cmd: "ctest --test-dir build 2>&1",
                timeout: 300,
            },
        ),
        AgentLanguage::Elixir => {
            let mut v = pipeline(
                Step {
                    cmd: "mix format",
                    timeout: 30,
                },
                Step {
                    cmd: "mix credo 2>&1",
                    timeout: 120,
                },
                Step {
                    cmd: "mix compile 2>&1",
                    timeout: 120,
                },
                Step {
                    cmd: "mix test 2>&1",
                    timeout: 300,
                },
            );
            v.push(outdated_agent("mix hex.outdated 2>&1", 60));
            v.push(audit_agent("mix hex.audit 2>&1"));
            v
        }
        AgentLanguage::Zig => pipeline(
            Step {
                cmd: "zig fmt .",
                timeout: 30,
            },
            Step {
                cmd: "zig fmt --check . 2>&1",
                timeout: 120,
            },
            Step {
                cmd: "zig build 2>&1",
                timeout: 120,
            },
            Step {
                cmd: "zig build test 2>&1",
                timeout: 300,
            },
        ),
        AgentLanguage::Lua => pipeline(
            Step {
                cmd: "stylua .",
                timeout: 30,
            },
            Step {
                cmd: "luacheck . 2>&1",
                timeout: 120,
            },
            Step {
                cmd: "find . -name '*.lua' -exec luac -p {} + 2>&1",
                timeout: 60,
            },
            Step {
                cmd: "busted 2>&1",
                timeout: 300,
            },
        ),
    }
}
