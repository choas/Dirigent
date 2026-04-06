<p align="center">
  <img src="assets/logo.png" alt="Dirigent logo" width="128" />
</p>

<h1 align="center">Dirigent</h1>

<p align="center">
  A read-only code viewer where humans direct and AI performs.
</p>

<p align="center">
  <a href="#features">Features</a> &middot;
  <a href="#prerequisites">Prerequisites</a> &middot;
  <a href="#building">Building</a> &middot;
  <a href="#usage">Usage</a> &middot;
  <a href="#documentation">Documentation</a> &middot;
  <a href="#license">License</a>
</p>

---

Dirigent is a desktop application for AI-assisted code review and editing. You browse your codebase read-only, place **cues** (instructions for AI) on specific files and lines, then let [Claude Code](https://docs.anthropic.com/en/docs/claude-code) execute the work. Review the resulting diffs, accept or reject changes, and commit — all from a single interface.

The name comes from the German word for *conductor*: you direct, the AI performs.

## Features

- **File tree & code viewer** — Browse any Git repository with syntax highlighting (via [syntect](https://crates.io/crates/syntect)); lines with active cues are marked with a yellow dot
- **Code symbols** — Multi-language symbol parsing (functions, structs, enums, traits, classes) for 11+ languages with definition lookup
- **LSP integration** — Built-in Language Server Protocol client with presets for 13 languages (Rust, TypeScript, Python, Go, Java, C#, C/C++, Ruby, Swift, Kotlin, Elixir, Zig, Lua); provides diagnostics, hover info, goto definition, find references, and document symbols
- **Quick Open** — Fuzzy file finder (Cmd+P) for fast navigation across the project
- **Tabs & navigation** — Open multiple files in tabs; breadcrumb path bar; back/forward navigation history
- **Cue system** — Select lines in the code viewer and create cues describing what you want changed; each cue carries a file path, line range, and natural-language prompt
- **Cue pool (kanban)** — Track cues through stages: Inbox → Ready → Review → Done → Archived, with a Backlog column for long-term items; bulk actions on Review and Done columns
- **Claude Code integration** — Sends prompts to Claude Code CLI and streams progress in real time; configurable model (Opus 4.6, Sonnet 4.6), CLI path, and extra arguments
- **OpenCode support** — Alternative CLI backend supporting multiple providers: OpenAI (o3, o3-mini), Anthropic (Sonnet/Opus), and Google (Gemini 2.5 Pro/Flash)
- **Conversation history** — View the full conversation log for each cue, including all past executions and live streaming output; rendered with full Markdown support (code blocks, tables, lists, blockquotes); searchable prompt history for reuse
- **Reply workflow** — Send follow-up feedback on diffs for iterative refinement without creating a new cue
- **Prompt quality hints** — Heuristic analysis flags vague, too-short, or missing-context prompts before sending to AI
- **Cue commands** — Prefix cues with `[plan]`, `[test]`, `[refactor]`, `[review]`, or `[fix]` to apply specialized prompt templates with optional pre/post scripts; customizable in Settings
- **Image attachments** — Attach images to cues for Claude to reference during execution; drag and drop files onto the window to attach
- **Diff view** — Review AI-generated changes inline or side-by-side before accepting; collapsible per-file diffs with +/- statistics and in-diff search
- **Git integration** — View commit history (last 50 commits), create commits, manage worktrees, see dirty-file indicators in the file tree; merge conflict resolution dialog, diverged-branch handling, and `git init` for non-repo directories
- **Pull requests** — Create GitHub PRs directly from Dirigent (title, base branch, description, draft flag); import actionable findings from PR reviews (e.g. CodeRabbit) as cues with a filter dialog for selective import
- **Search** — In-file search (Cmd+F) with match navigation; project-wide search (Cmd+Shift+F) with clickable results
- **Source integration** — Import cues from GitHub Issues, Slack, SonarQube, Notion, Trello, Asana, MCP, or custom shell commands with automatic deduplication and configurable polling
- **Markdown import** — Import cues from a Markdown document (headings become cue titles); supports upsert to avoid duplicates
- **Playbook** — Predefined prompts (e.g. "Update README", "Security audit", "Add tests", "Commit changes") that can be run as global cues; supports template variables with free-text and dropdown inputs
- **Themes** — 20 themes: 10 dark (Nord, Dracula, Solarized, Monokai, Gruvbox, Tokyo Night, One Dark, Catppuccin Mocha, Everforest, Dark) and 10 light (Solarized, Gruvbox, GitHub, Catppuccin Latte, Everforest, Rose Pine Dawn, One Light, Nord, Tokyo Night, Light)
- **Extended syntax** — Custom syntax definitions for Kotlin and Dart on top of syntect defaults
- **File watching** — Automatic filesystem monitoring with debounced rescan when files change on disk
- **File operations** — Rename files directly from the code viewer
- **Notifications** — macOS notifications with three-tier fallback (UNUserNotificationCenter → NSUserNotificationCenter → osascript) and configurable sound/popup
- **Lava lamp** — Retro pixelated lava lamp animation while a cue is running
- **Agents** — Post-run automation agents (Format, Lint, Build, Test) with configurable triggers: AfterRun, AfterCommit, AfterAgent chaining, OnFileChange, and Manual; per-cue agent run history, dedicated log viewer, and cargo diagnostic parsing
- **Workflow planning** — LLM-analyzed execution plans for multiple Inbox cues; automatically groups independent cues for parallel execution and orders dependent steps sequentially; visual workflow graph overlay with step-by-step progress
- **Task management** — Background task lifecycle with cancellation support; running Claude tasks can be stopped mid-execution
- **Error tracking** — Optional Sentry integration for crash reporting; optional OpenTelemetry log export via `DIRIGENT_OTEL_ENDPOINT`
- **Home directory guard** — Optional Claude Code hook that blocks tool calls targeting personal directories (configurable in Settings)
- **Settings** — Configure theme, CLI backend and model, font family and size, notification preferences, cue commands, sources, playbook, agent commands/triggers, and LSP servers
- **Repository picker** — Switch between projects and track recent repositories
- **macOS native** — Custom About panel, dock icon, notification integration, .app bundle with code signing and notarization

## Prerequisites

- **Rust** (1.75+ recommended) — [rustup.rs](https://rustup.rs)
- **Claude Code CLI** — Must be installed and available on your `PATH` (or configured via Settings). See [Claude Code docs](https://docs.anthropic.com/en/docs/claude-code).
- **System dependencies** for your platform:
  - **macOS**: Xcode Command Line Tools (`xcode-select --install`)
  - **Linux**: `libgtk-3-dev`, `libxcb-shape0-dev`, `libxcb-xfixes0-dev`, and OpenSSL dev headers. On Debian/Ubuntu:
    ```
    sudo apt install libgtk-3-dev libxcb-shape0-dev libxcb-xfixes0-dev libssl-dev pkg-config
    ```

## Building

```bash
git clone https://github.com/choas/Dirigent.git
cd Dirigent
cargo build --release
```

The binary will be at `target/release/Dirigent`.

### macOS .app bundle

A Makefile is provided for building, bundling, code signing, and creating a DMG:

```bash
make build          # cargo build --release
make bundle         # Create .app bundle
make sign IDENTITY="Developer ID Application: ..."   # Code sign
make dmg            # Create DMG (requires create-dmg)
make notarize APPLE_ID=... TEAM_ID=...               # Submit for notarization
```

Releases are also automated via GitHub Actions — pushing a `v*` tag triggers a workflow that builds, signs, notarizes, and publishes a DMG to GitHub Releases.

## Usage

```bash
# Open the current directory
cargo run --release

# Open a specific project
cargo run --release -- /path/to/your/project
```

On macOS, the bundled .app can also be launched from Finder or the Dock. When launched without a path, Dirigent opens the repository picker so you can choose a project.

### Workflow

1. **Browse** — Navigate your codebase in the file tree on the left
2. **Cue** — Select lines in the code viewer and create a cue describing what you want changed
3. **Run** — Click Run on a cue to send it to Claude Code; streaming progress is shown in real time
4. **Review** — Inspect the diff that Claude produced (inline or side-by-side)
5. **Reply** — Optionally send follow-up feedback for iterative refinement
6. **Accept or reject** — Commit the changes or revert them
7. **Repeat** — Use the global prompt field or playbook for cues that don't target specific lines

### Per-project data

Dirigent stores its database (`Dirigent.db`) and settings (`settings.json`) in a `.Dirigent/` directory inside your project root. This directory is local to each project and should be added to `.gitignore`.

## Documentation

Detailed documentation is organized following the [Diataxis](https://diataxis.fr/) framework:

| Type | Document | Description |
|------|----------|-------------|
| Tutorial | [Getting Started](docs/tutorial.md) | Step-by-step walkthrough from install to first cue |
| How-To Guides | [How-To Guides](docs/how-to.md) | Task-oriented recipes (cue commands, agents, PR creation, LSP, etc.) |
| Reference | [Reference](docs/reference.md) | Complete settings, keyboard shortcuts, cue commands, playbook, and environment variables |
| Explanation | [Explanation](docs/explanation.md) | Architecture, design decisions, and core concepts |
| How-To Guide | [Observability Setup](docs/observability-setup.md) | OpenTelemetry + Grafana local observability stack |

## Architecture

```
src/
├── main.rs                   — Entry point, window setup, macOS integration
├── agents/                   — Post-run agents (Format, Lint, Build, Test)
│   ├── diagnostics.rs        — Cargo JSON diagnostic parser
│   ├── execution.rs          — Agent execution logic
│   ├── presets.rs             — Built-in agent presets
│   ├── run_state.rs           — Agent run state tracking
│   ├── trigger.rs             — Agent trigger types
│   └── types.rs               — Agent data types
├── app/
│   ├── mod.rs                — App state, update loop, panel orchestration
│   ├── agents_poll.rs        — Agent result processing and state management
│   ├── background.rs         — Background task orchestration
│   ├── claude_run.rs         — Claude/OpenCode execution, streaming, conversation history
│   ├── file_navigation.rs    — File open, tab management, navigation history
│   ├── git_operations.rs     — Git operation handlers
│   ├── lava_lamp.rs          — Retro pixelated lava lamp animation overlay
│   ├── markdown_parser.rs    — Markdown-to-block parsing (headings, code, tables, lists)
│   ├── markdown_viewer.rs    — Rendered Markdown display with syntax-highlighted code
│   ├── notifications.rs      — macOS notification delivery (UNUserNotificationCenter fallback)
│   ├── rendering.rs          — Drag-and-drop handling, global keyboard shortcuts
│   ├── repo_management.rs    — Repository switching and management
│   ├── search.rs             — In-file and project-wide search
│   ├── sources_poll.rs       — Background polling of external cue sources
│   ├── symbols.rs            — Multi-language code symbol parsing and definition lookup
│   ├── tasks.rs              — Background task lifecycle and cancellation
│   ├── theme.rs              — Theme system, semantic colors, font loading
│   ├── types.rs              — Shared app-level types (PendingPlay, DiffReview, etc.)
│   ├── util.rs               — Utility functions (ANSI stripping, duration formatting)
│   ├── workflow_graph.rs     — Workflow plan graph overlay rendering
│   ├── workflow_run.rs       — Workflow creation, step execution, and progress
│   ├── code_viewer/
│   │   ├── breadcrumb.rs     — Breadcrumb path bar with navigation
│   │   ├── cue_input.rs      — Inline cue creation from code selection
│   │   ├── goto_definition.rs — LSP-powered go-to-definition
│   │   ├── line_rendering.rs — Line-level rendering with diagnostics
│   │   ├── quick_open.rs     — Fuzzy file finder overlay (Cmd+P)
│   │   ├── tab_bar.rs        — Multi-file tab bar
│   │   └── types.rs          — Code viewer types
│   ├── cue_pool/
│   │   ├── mod.rs            — Kanban columns and cue card rendering
│   │   ├── actions.rs        — Cue status transition actions
│   │   ├── bulk_actions.rs   — Bulk operations on cue sections
│   │   ├── helpers.rs        — Cue pool utility functions
│   │   ├── history.rs        — Searchable prompt history
│   │   ├── markdown_import.rs — Markdown import for batch cue creation
│   │   └── cue_card/         — Cue card rendering (activity, buttons, inputs, etc.)
│   ├── dialog/
│   │   ├── agent_log.rs      — Agent execution log viewer
│   │   ├── create_pr.rs      — GitHub pull request creation dialog
│   │   ├── cue_agent_runs.rs — Per-cue agent run history
│   │   ├── diff_review.rs    — Diff review with reply, search, accept/reject
│   │   ├── file_ops.rs       — File rename dialog
│   │   ├── filter_pr.rs      — PR findings filter dialog
│   │   ├── git_init.rs       — Git init confirmation for non-repo directories
│   │   ├── import_pr.rs      — Import PR review findings as cues
│   │   ├── merge_conflicts.rs — Merge conflict resolution dialog
│   │   ├── play_variables.rs — Playbook template variable input dialog
│   │   ├── pull_diverged.rs  — Diverged branch merge/rebase strategy picker
│   │   ├── pull_unmerged.rs  — Unmerged files guidance dialog
│   │   ├── repo.rs           — Repository picker and recent repos
│   │   ├── running_log.rs    — Live conversation viewer for Claude/OpenCode output
│   │   └── settings/         — Settings panel (general, LSP, sources, agents, commands, playbook)
│   └── panels/
│       ├── menu_bar.rs       — Application menu bar
│       ├── repo_bar.rs       — Repository and branch bar
│       ├── file_tree.rs      — File tree panel
│       ├── prompt_field.rs   — Prompt input field
│       └── status_bar.rs     — Status bar
├── claude/                   — Claude Code CLI invocation and stream parsing
│   ├── cli.rs                — CLI detection and invocation
│   ├── diff_parser.rs        — Diff extraction from Claude output
│   ├── invoke.rs             — Prompt building and execution
│   ├── prompt.rs             — Prompt construction
│   ├── stream.rs             — Streaming output parser
│   └── types.rs              — Claude-specific data types
├── db/                       — SQLite persistence
│   ├── activity.rs           — Activity/history queries
│   ├── agent_runs.rs         — Agent run persistence
│   ├── converters.rs         — Row-to-type converters
│   ├── cue_ops.rs            — Cue CRUD operations
│   ├── execution_ops.rs      — Execution record operations
│   ├── migrations.rs         — Schema migrations
│   ├── pattern_ops.rs        — Pattern/ignore operations
│   ├── source_ops.rs         — Source-imported cue operations
│   └── types.rs              — DB data types
├── git/                      — Git operations
│   ├── archive.rs            — Worktree DB archival before removal
│   ├── commit.rs             — Commit creation
│   ├── diff.rs               — Diff generation
│   ├── graph.rs              — Branch/commit graph visualization
│   ├── history.rs            — Commit history queries
│   ├── merge.rs              — Merge and conflict resolution
│   ├── pr.rs                 — GitHub pull request operations
│   ├── status.rs             — Working tree status
│   └── worktree.rs           — Worktree management
├── lsp/                      — Language Server Protocol client
│   ├── client.rs             — LSP JSON-RPC client
│   ├── manager.rs            — Multi-server lifecycle and routing
│   └── types.rs              — Server configs, language presets (13 languages)
├── settings/                 — App settings and configuration
│   ├── app_settings.rs       — Core settings struct and defaults
│   ├── commands.rs           — Cue command definitions ([plan], [test], etc.)
│   ├── home_guard.rs         — Claude Code home-directory guard hook
│   ├── io.rs                 — Settings file I/O
│   ├── playbook.rs           — Playbook prompts and template variables
│   ├── providers.rs          — CLI provider configuration
│   ├── recent.rs             — Recent repositories tracking
│   ├── semantic_colors.rs    — Theme-aware semantic color palette
│   └── theme.rs              — Theme definitions (20 themes)
├── sources/                  — External cue sources
│   ├── custom.rs             — Custom shell command sources
│   ├── external.rs           — GitHub Issues, Slack, SonarQube, Notion, Trello, Asana, MCP
│   ├── finding_text.rs       — Finding text extraction and formatting
│   ├── html.rs               — HTML content parsing
│   ├── pr_comments.rs        — PR comment filtering (confirmation, summary detection)
│   ├── pr_feedback.rs        — PR review feedback processing
│   ├── pr_findings.rs        — PR findings import via gh CLI
│   └── types.rs              — Source data types
├── syntax.rs                 — Extended syntax highlighting (Kotlin, Dart)
├── opencode.rs               — OpenCode CLI support (multi-provider backend)
├── diff_view.rs              — Unified diff parsing, inline and side-by-side rendering
├── error.rs                  — Unified error types (DirigentError, Result alias)
├── file_tree.rs              — Recursive directory scanning with ignore patterns
├── prompt_hints.rs           — Heuristic prompt quality analysis and warnings
├── prompt_suggestions.rs     — Prompt improvement suggestions (context, specificity)
├── telemetry.rs              — OpenTelemetry OTLP log export (opt-in via env var)
└── workflow.rs               — Workflow plan types, LLM prompt building, response parsing
```

**Key dependencies:**

| Crate | Purpose |
|-------|---------|
| [eframe](https://crates.io/crates/eframe) 0.34 / [egui](https://crates.io/crates/egui) | Cross-platform immediate-mode GUI |
| [egui_extras](https://crates.io/crates/egui_extras) (syntect) | Syntax highlighting |
| [rusqlite](https://crates.io/crates/rusqlite) 0.39 | SQLite (bundled) |
| [git2](https://crates.io/crates/git2) 0.20 | Git operations via libgit2 |
| [lsp-types](https://crates.io/crates/lsp-types) 0.97 | Language Server Protocol types |
| [notify](https://crates.io/crates/notify) | Cross-platform filesystem watching |
| [pulldown-cmark](https://crates.io/crates/pulldown-cmark) | Markdown parsing for conversation and import rendering |
| [reqwest](https://crates.io/crates/reqwest) | HTTP client for source integrations |
| [rfd](https://crates.io/crates/rfd) | Native file dialogs |
| [sentry](https://crates.io/crates/sentry) | Error tracking and crash reporting |
| [thiserror](https://crates.io/crates/thiserror) | Ergonomic error type definitions |

## Status

Dirigent is in active development (v0.3.5, 800+ commits). Core features work — file browsing with tabs and quick-open, LSP-powered code intelligence (diagnostics, goto definition, hover), code symbols, cue management with bulk actions and cue commands, Claude Code and OpenCode integration with conversation history and reply workflow, diff review, GitHub PR creation and import with filtering, git operations with merge conflict resolution, search, source integration (GitHub Issues, Slack, SonarQube, Notion, Trello, Asana, MCP, custom), image attachments with drag-and-drop, prompt quality hints, workflow planning, post-run agents with diagnostic parsing, OpenTelemetry telemetry, home directory guard, and macOS app bundling — but expect rough edges. Contributions and feedback are welcome.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

Copyright 2026 Lars Gregori
