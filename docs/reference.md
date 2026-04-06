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

### General

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| Theme | enum | Dark | UI color theme (see [Themes](#themes)) |
| Font family | string | Menlo | Monospace font for code and UI |
| Font size | float | 13.0 | Font size in points |
| Notify sound | bool | true | Play sound when a cue finishes |
| Notify popup | bool | true | Show macOS notification popup |
| Lava lamp | bool | true | Show lava lamp animation during runs |
| Prompt suggestions | bool | false | Show heuristic refinement hints below prompt |
| Auto-context: file | bool | false | Include surrounding lines around cue location in prompt |
| Auto-context: git diff | bool | false | Include current git diff in prompt |

### CLI Provider

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| CLI provider | enum | Claude | `Claude` or `OpenCode` |
| Claude model | string | claude-opus-4-6 | Model to use with Claude Code |
| Claude custom models | list | empty | Additional model names for the dropdown |
| Claude CLI path | string | (auto-detect) | Path to `claude` binary; auto-detected via `which` |
| Claude extra args | string | empty | Additional CLI arguments appended to every invocation |
| Claude env vars | string | empty | Environment variable names to forward (one per line) |
| Claude pre-run script | string | empty | Shell script executed before each Claude run |
| Claude post-run script | string | empty | Shell script executed after each Claude run |
| Skip permissions | bool | true | Append `--dangerously-skip-permissions` to CLI |
| OpenCode model | string | openai/o1 | Model for OpenCode provider |
| OpenCode CLI path | string | (auto-detect) | Path to `opencode` binary |
| OpenCode extra args | string | empty | Additional OpenCode CLI arguments |

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
| Verify last 5 commits | Review recent commits for bugs or inconsistencies | -- |
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
| **Archived** | Completed and archived (paginated, 50 per page) |

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
| `execution.completed` | `project`, `cue_id`, `provider`, `cost_usd`, `duration_ms`, `num_turns`, `input_tokens`, `output_tokens`, `has_diff` |
| `execution.failed` | `project`, `cue_id`, `provider`, `error` |
| `execution.rate_limited` | `project`, `cue_id`, `message` |
| `cue.status_changed` | `project`, `cue_id`, `from_status`, `to_status` |
| `agent.completed` | `project`, `agent_kind`, `status`, `duration_ms`, `cue_id` |
| `git.commit` | `project`, `files_changed` |

All events include a `session_id` attribute unique to each Dirigent launch.
