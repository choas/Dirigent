# Reference

Complete reference for Dirigent's settings, keyboard shortcuts, cue commands, playbook, CLI usage, environment variables, and per-project data.

---

## Table of Contents

- [CLI Usage](#cli-usage)
- [Keyboard Shortcuts](#keyboard-shortcuts)
- [Settings](#settings)
- [Cue Commands](#cue-commands)
- [Playbook](#playbook)
- [Cue Lifecycle](#cue-lifecycle)
- [Agent Triggers](#agent-triggers)
- [LSP Language Presets](#lsp-language-presets)
- [Themes](#themes)
- [Environment Variables](#environment-variables)
- [Per-Project Data](#per-project-data)
- [Telemetry Events](#telemetry-events)

---

## CLI Usage

```
Dirigent [PATH]
```

| Argument | Description |
|----------|-------------|
| `PATH` | Optional. Path to a Git repository to open. Defaults to the current working directory. If omitted when launched from an app bundle, the repository picker is shown. |

## Keyboard Shortcuts

All shortcuts use the Command key on macOS.

### Navigation

| Shortcut | Action |
|----------|--------|
| Cmd+P | Quick Open (fuzzy file finder) |
| Cmd+W | Close current tab |
| Cmd+[ | Navigate back in history |
| Cmd+] | Navigate forward in history |
| Arrow keys | Navigate lines in code viewer |

### Search

| Shortcut | Action |
|----------|--------|
| Cmd+F | Search in current file (or in diff review) |
| Cmd+Shift+F | Project-wide search |

### Cues

| Shortcut | Action |
|----------|--------|
| Enter | Create cue from prompt field |
| Cmd+Enter | Create and immediately run cue |
| Shift+Enter | Newline in prompt field (multiline input) |

### Window

| Shortcut | Action |
|----------|--------|
| Cmd+N | Open new Dirigent window |

### Tabs

Right-click a tab for additional actions: Close All, Close Others, Close to Right.

## Settings

Settings are stored per-project in `.Dirigent/settings.json` and edited via the Settings dialog in the app.

### Repository trust

Because `.Dirigent/settings.json` arrives with a checkout, many of its fields are execution-capable input (LSP server commands, source commands, pre/post-run scripts, agent commands, extra CLI args, program paths, LLM endpoints, …). On the first open of a repository — or whenever those fields change on disk outside the app — Dirigent shows exactly the execution-capable fields and asks for approval. Until you trust the repository, those fields stay at their defaults and `--dangerously-skip-permissions` is off, so agents run restricted. Only cosmetic fields (theme, fonts, layout, notifications) apply without approval; any settings field not on that safe list — including fields added in future versions — is gated by default. Approvals are remembered per repository by a content hash of the execution-capable fields in `~/Library/Application Support/Dirigent/trusted_repos.json`.

### General

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| Theme | enum | Dark | UI color theme (see [Themes](#themes)) |
| Font family | string | Menlo | Monospace font for code and UI |
| Font size | float | 13.0 | Font size in points |
| Notify sound | bool | true | Play sound when a cue finishes |
| Notify popup | bool | true | Show macOS notification popup |
| Running animation | enum | LavaLamp | Overlay during runs: `Off`, `LavaLamp`, `ClaudeCodeName`, `Dino`, `Jujutsu`, `Thunderdome` |
| Prompt suggestions | bool | false | Show heuristic refinement hints below prompt |
| Auto-context: file | bool | false | Include surrounding lines around cue location in prompt |
| Auto-context: git diff | bool | false | Include current git diff in prompt |
| Font weight | enum | Regular | `Light`, `Regular`, `Medium`, `SemiBold`, `Bold` |
| Custom themes | list | empty | User-defined themes (see [Themes](#themes)) |
| Diff color scheme | enum | RedGreen | `RedGreen`, `RedBlue`, `YellowBlue` |
| Heartbeat style | enum | Heartbeat | Status-bar activity indicator: `Heartbeat`, `Wave`, `GabbaPeak`, `MorseCode`, `Off` |
| Claude Code display name | string | empty | Rename "Claude Code" in the UI |
| Custom dock icon path | string | empty | PNG used as the macOS Dock icon; falls back to a project `logo.png` |
| Show frame timing | bool | false | Overlay per-panel render times (debugging) |
| External diff tool | string | empty | Command used to open a diff externally instead of the built-in viewer |

### CLI Provider

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| CLI provider | enum | Claude | `Claude`, `OpenCode`, `Gemini`, `Codex`, or `ACP Agent` |
| Test Connection | button | — | Sends `hello` to the selected provider with the current (unsaved) settings and shows the reply or error; pre/post-run scripts are skipped ([How_to_ACP.md](How_to_ACP.md#testing-the-connection)) |
| Claude model | string | claude-opus-4-8 | Model to use with Claude Code |
| Claude custom models | list | empty | Additional model names for the dropdown |
| Claude CLI path | string | (auto-detect) | Path to `claude` binary; auto-detected via `which` |
| Claude extra args | string | empty | Additional CLI arguments appended to every invocation |
| Claude env vars | string | empty | Environment variable names to forward (one per line) |
| Claude pre-run script | string | empty | Shell script executed before each Claude run |
| Claude post-run script | string | empty | Shell script executed after each Claude run |
| Skip permissions | bool | true | Append `--dangerously-skip-permissions` to CLI |
| Claude use PTY | bool | true | Drive `claude` through a pseudo-terminal (the interactive TUI). Off falls back to headless `claude -p`. |
| OpenCode model | string | openai/o1 | Model for OpenCode provider |
| OpenCode CLI path | string | (auto-detect) | Path to `opencode` binary |
| OpenCode extra args | string | empty | Additional OpenCode CLI arguments |
| OpenCode env vars / pre-run / post-run | string | empty | As for Claude, per provider |
| Gemini model | string | gemini-2.5-flash | Model for the Gemini provider |
| Gemini CLI path | string | (auto-detect) | Path to `gemini` binary |
| Gemini extra args / env vars / pre-run / post-run | string | empty | As for Claude, per provider |
| Codex model | string | gpt-5-codex | Model for the Codex provider |
| Codex CLI path | string | (auto-detect) | Path to `codex` binary |
| Codex extra args / env vars / pre-run / post-run | string | empty | As for Claude, per provider |
| ACP agent command | string | empty | ACP agent executable, e.g. `opencode`, `gemini`, `goose` ([How_to_ACP.md](How_to_ACP.md)) |
| ACP extra args | string | empty | Arguments that start the agent in ACP mode, e.g. `acp`, `--experimental-acp` |
| ACP env vars | string | empty | `KEY=VAL` lines set on the agent process |
| ACP pre-/post-run script | string | empty | Shell scripts executed around each ACP run |
| Auto-continue | bool | false | Re-prompt a run that stopped with unfinished work |
| Auto-continue max | int | 3 | Cap on consecutive auto-continues per cue |

Pre/post-run scripts only execute for a repository whose execution-capable
settings you approved — see [Repository trust](#repository-trust). In an
unapproved repository they are skipped with a warning; the run itself still
proceeds.

### Version control

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| VCS backend | enum | Git | `Git` or `Jj`. Auto-switches to `Jj` when the project has a `.jj` directory. |
| jj CLI path | string | (auto-detect) | Path to the `jj` binary |
| Auto commit | bool | false | Commit automatically when a run's changes are accepted |
| Commit footer | bool | true | Append Dirigent's co-author footer to commit messages |

### Fast LLM

A small local model used for quick, non-agentic jobs (commit-message drafts,
change-set grouping). When disabled, those jobs fall back to the configured CLI
provider.

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| Fast LLM enabled | bool | false | Use the Fast LLM for quick jobs |
| Fast LLM provider | string | Ollama | Label for the OpenAI-compatible endpoint |
| Fast LLM base URL | string | `http://localhost:11434/v1` | OpenAI-compatible endpoint. A non-local URL requires explicit approval before any repository content is sent to it. |
| Fast LLM API key | string | empty | Bearer token for the endpoint |
| Fast LLM model | string | gemma4:26b-mlx | Model identifier |

### Dirigent MCP server

Dirigent exposes its own cue database over MCP so an agent can read and move
cues mid-run. Injected automatically when the prompt mentions "dirigent".

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| MCP server path | string | (auto-detect) | Path to the `dirigent-mcp-server` binary |
| MCP database path | string | (project DB) | Override the database the MCP server reads |

### SSH servers

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| SSH servers | list | empty | Remote hosts browsable from the SSH panel (name, host, username) |

### Security

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| Allow home folder access | bool | false | When false, installs a Claude Code hook that blocks tool calls targeting personal directories |

### Agents

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| Agents | list | Format, Lint, Build, Test | Post-run automation agents |
| Agent shell init | string | empty | Shell snippet prepended to agent commands (useful for PATH setup on macOS GUI launches) |

Each agent has: name, command, trigger, and optional language.

### LSP

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| LSP enabled | bool | false | Master toggle for Language Server Protocol |
| LSP servers | list | 13 presets | Server configurations per language |

A server's command must be the language server binary itself. Shells, exec wrappers and language interpreters (`sh`, `env`, `xargs`, `node`, `python3`, `java`, …) are refused as the command, and eval-style arguments (`-c`, `-e`, `--eval`, `-exec`, …) are refused in the args list — otherwise a repository's `.Dirigent/settings.json` could run arbitrary code through a server definition. Wrap such an invocation in a small executable script on your PATH and point the command at that. Servers also do not start until the repository is trusted (see [Repository trust](#repository-trust)).

### Sources

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| Sources | list | empty | External cue source integrations |

### Commands

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| Commands | list | 8 defaults | Cue command definitions (see [Cue Commands](#cue-commands)) |

### Playbook

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| Playbook | list | 11 defaults | Predefined prompt templates (see [Playbook](#playbook)) |

## Cue Commands

Prefix a cue with `[command]` to apply a specialized prompt template. The `{task}` placeholder in the template is replaced with the rest of your cue text.

| Command | Purpose | Produces changes? |
|---------|---------|-------------------|
| `[plan]` | Analyze the task, create a detailed implementation plan | No (plan-only mode) |
| `[test]` | Write comprehensive tests (happy paths, edge cases, errors) | Yes |
| `[refactor]` | Improve clarity and maintainability; preserve behavior | Yes |
| `[review]` | Find bugs, security issues, performance problems | No (report only) |
| `[fix]` | Fix a specific bug with minimal, correct changes | Yes |
| `[docs]` | Write or update documentation with examples | Yes |
| `[explain]` | Explain control flow, data structures, design decisions | No (explanation only) |
| `[optimize]` | Optimize for performance while preserving correctness | Yes |

Each command can also specify:
- **cli_args** -- extra CLI arguments (e.g. `[plan]` sets `--permission-mode plan`)
- **pre_agent / post_agent** -- override agent scripts for that command

Custom commands can be added in **Settings > Commands**.

## Playbook

Predefined prompts available from the Playbook menu. Plays with template variables prompt for input before running.

| Play | Description | Variables |
|------|-------------|-----------|
| Documentation (Diataxis) | Update docs following Diataxis framework; sync README | -- |
| Verify architecture | Check for structural issues, circular dependencies | -- |
| Git Pull | Pull the latest changes and resolve any conflicts | -- |
| Create release | Prepare release: version numbers, CHANGELOG, tests, tag | `{VERSION}`, `{LICENSE}` (dropdown) |
| Security audit | Check for secrets, insecure deps, injection vulnerabilities | -- |
| Check dead code | Find unused functions, imports, unreachable branches | -- |
| Add tests | Identify untested paths and write comprehensive tests | -- |
| Fix all warnings | Detect project type, run linter, fix every warning | -- |
| Commit changes | Commit staged changes using cue pool titles | -- |
| Zero day test | Challenge to find RCE 0-day in file opening | -- |
| Pin dependency versions | Lock all floating versions to exact pins | -- |

Template variable syntax: `{VAR_NAME}` for free-text, `{VAR_NAME:option1,option2}` for dropdown.

## Cue Lifecycle

Cues move through kanban columns representing their status:

```
Backlog -> Inbox -> Ready -> (Running) -> Review -> Done -> Archived
```

| Status | Description |
|--------|-------------|
| **Backlog** | Long-term items not yet prioritized |
| **Inbox** | Newly created or imported cues |
| **Ready** | Cue is queued and ready to run |
| **Running** | Claude/OpenCode is executing (transient) |
| **Review** | Execution complete; diff awaiting review |
| **Done** | Changes accepted; ready to commit |
| **Archived** | Completed and archived (paginated, 10 per page) |

Bulk actions are available on the Review and Done columns.

## Agent Triggers

| Trigger | When it fires |
|---------|---------------|
| `AfterRun` | Immediately after a Claude/OpenCode execution completes |
| `AfterCommit` | After a git commit is created |
| `AfterAgent` | After another agent finishes (chaining) |
| `OnFileChange` | When files change on disk (via filesystem watcher) |
| `Manual` | Only when explicitly triggered by the user |

## LSP Language Presets

Dirigent includes built-in configurations for these language servers:

| Language | Server binary |
|----------|--------------|
| Rust | `rust-analyzer` |
| TypeScript/JavaScript | `typescript-language-server` |
| Python | `pylsp` |
| Go | `gopls` |
| Java | `jdtls` |
| C# | `omnisharp` |
| C/C++ | `clangd` |
| Ruby | `solargraph` |
| Swift | `sourcekit-lsp` |
| Kotlin | `kotlin-language-server` |
| Elixir | `elixir-ls` |
| Zig | `zls` |
| Lua | `lua-language-server` |

The language server must be installed on your system. Custom server configurations can be added in Settings.

## Themes

### Dark themes

Nord, Dracula, Solarized Dark, Monokai, Gruvbox Dark, Tokyo Night, One Dark, Catppuccin Mocha, Everforest Dark, Dark (default)

### Light themes

Solarized Light, Gruvbox Light, GitHub Light, Catppuccin Latte, Everforest Light, Rose Pine Dawn, One Light, Nord Light, Tokyo Night Light, Light

## Environment Variables

| Variable | Description |
|----------|-------------|
| `DIRIGENT_OTEL_ENDPOINT` | OTLP HTTP endpoint for telemetry (e.g. `http://localhost:4318`). Read once at startup. |
| `SENTRY_DSN` | Sentry DSN for crash reporting. Optional. |

Source integrations read their tokens from the process environment, then from
`.Dirigent/.env` (preferred) or `.env`. Setting the token in the source's
configuration overrides both.

| Variable | Used by |
|----------|---------|
| `SLACK_BOT_TOKEN` | Slack source |
| `SONAR_TOKEN` | SonarQube source |
| `TRELLO_API_KEY` | Trello source (API key; paired with the token below) |
| `TRELLO_TOKEN` | Trello source |
| `ASANA_TOKEN` | Asana source |
| `NOTION_TOKEN` | Notion source |
| `SENTRY_AUTH_TOKEN` | Sentry source |

Claude Code environment variables (for observability) are documented in the [Observability Setup](observability-setup.md) guide.

## Per-Project Data

Dirigent stores project-specific data in a `.Dirigent/` directory at the project root.

| Path | Description |
|------|-------------|
| `.Dirigent/Dirigent.db` | SQLite database: cues, executions, history, agent runs |
| `.Dirigent/settings.json` | Project settings (overrides defaults) |
| `.Dirigent/.env` | Environment variables forwarded to AI runs |
| `.Dirigent/archives/` | Archived databases from removed worktrees |

Add `.Dirigent` to your `.gitignore`. It is local to each developer and should not be committed.

Recent project history is stored globally in the platform's Application Support directory under `Dirigent/recent_projects.json`.

## Telemetry Events

When `DIRIGENT_OTEL_ENDPOINT` is set, Dirigent emits these structured log events via OTLP HTTP:

| Event | Key Attributes |
|-------|---------------|
| `app.started` | `project` |
| `execution.started` | `project`, `cue_id`, `provider`, `model` |
| `execution.completed` | `project`, `cue_id`, `provider`, `duration_ms`, `has_diff`; plus `cost_usd`, `num_turns`, `input_tokens`, `output_tokens` when the backend reported them |
| `execution.failed` | `project`, `cue_id`, `provider`, `error` |
| `execution.rate_limited` | `project`, `cue_id`, `message` |
| `agent.completed` | `project`, `agent_kind`, `status`, `duration_ms`, `cue_id` |
| `git.commit` | `project`, `files_changed` |

All events include a `session_id` attribute unique to each Dirigent launch.
