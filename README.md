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
  <a href="#license">License</a>
</p>

---

Dirigent is a desktop application for AI-assisted code review and editing. You browse your codebase read-only, place **cues** (instructions for AI) on specific files and lines, then let [Claude Code](https://docs.anthropic.com/en/docs/claude-code) execute the work. Review the resulting diffs, accept or reject changes, and commit — all from a single interface.

The name comes from the German word for *conductor*: you direct, the AI performs.

## Features

- **File tree & code viewer** — Browse any Git repository with syntax highlighting (via [syntect](https://crates.io/crates/syntect)); lines with active cues are marked with a yellow dot
- **Code symbols** — Multi-language symbol parsing (functions, structs, enums, traits, classes) for 11+ languages with definition lookup
- **Cue system** — Select lines in the code viewer and create cues describing what you want changed; each cue carries a file path, line range, and natural-language prompt
- **Cue pool (kanban)** — Track cues through stages: Inbox → Ready → Review → Done → Archived, with a Backlog column for long-term items
- **Claude Code integration** — Sends prompts to Claude Code CLI and streams progress in real time; configurable model (Opus 4.6, Sonnet 4.6), CLI path, and extra arguments
- **OpenCode support** — Alternative CLI backend supporting multiple providers: OpenAI (o3, o3-mini), Anthropic (Sonnet/Opus), and Google (Gemini 2.5 Pro/Flash)
- **Conversation history** — View the full conversation log for each cue, including all past executions and live streaming output; rendered with full Markdown support (code blocks, tables, lists, blockquotes)
- **Reply workflow** — Send follow-up feedback on diffs for iterative refinement without creating a new cue
- **Prompt quality hints** — Heuristic analysis flags vague, too-short, or missing-context prompts before sending to AI
- **Image attachments** — Attach images to cues for Claude to reference during execution
- **Diff view** — Review AI-generated changes inline or side-by-side before accepting; collapsible per-file diffs with +/- statistics and in-diff search
- **Git integration** — View commit history (last 50 commits), create commits, manage worktrees, see dirty-file indicators in the file tree; merge conflict resolution dialog, diverged-branch handling, and `git init` for non-repo directories
- **Pull requests** — Create GitHub PRs directly from Dirigent (title, base branch, description, draft flag); import actionable findings from PR reviews (e.g. CodeRabbit) as cues
- **Search** — In-file search (Cmd+F) with match navigation; project-wide search (Cmd+Shift+F) with clickable results
- **Source integration** — Import cues from GitHub Issues, Notion, MCP, or custom shell commands with automatic deduplication and configurable polling
- **Markdown import** — Import cues from a Markdown document (headings become cue titles); supports upsert to avoid duplicates
- **Playbook** — Predefined prompts (e.g. "Update README", "Security audit", "Add tests", "Commit changes") that can be run as global cues; supports template variables with free-text and dropdown inputs
- **Themes** — 20 themes: 10 dark (Nord, Dracula, Solarized, Monokai, Gruvbox, Tokyo Night, One Dark, Catppuccin Mocha, Everforest, Dark) and 10 light (Solarized, Gruvbox, GitHub, Catppuccin Latte, Everforest, Rose Pine Dawn, One Light, Nord, Tokyo Night, Light)
- **File watching** — Automatic filesystem monitoring with debounced rescan when files change on disk
- **Notifications** — macOS notifications with three-tier fallback (UNUserNotificationCenter → NSUserNotificationCenter → osascript) and configurable sound/popup
- **Lava lamp** — Retro pixelated lava lamp animation while a cue is running
- **Agents** — Post-run automation agents (Format, Lint, Build, Test) with configurable triggers: AfterRun, AfterCommit, AfterAgent chaining, OnFileChange, and Manual; per-cue agent run history and dedicated log viewer
- **Task management** — Background task lifecycle with cancellation support; running Claude tasks can be stopped mid-execution
- **Settings** — Configure theme, CLI backend and model, font family and size, notification preferences, sources, playbook, and agent commands/triggers
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

## Architecture

```
src/
├── main.rs                — Entry point, window setup, macOS integration
├── agents.rs              — Post-run agents (Format, Lint, Build, Test)
├── app/
│   ├── mod.rs             — App state, update loop, panel orchestration
│   ├── agents_poll.rs     — Agent result processing and state management
│   ├── claude_run.rs      — Claude/OpenCode execution, streaming, conversation history
│   ├── code_viewer.rs     — Syntax-highlighted code display with cue markers
│   ├── lava_lamp.rs       — Retro pixelated lava lamp animation overlay
│   ├── markdown_parser.rs — Markdown-to-block parsing (headings, code, tables, lists)
│   ├── markdown_viewer.rs — Rendered Markdown display with syntax-highlighted code
│   ├── symbols.rs         — Multi-language code symbol parsing and definition lookup
│   ├── cue_pool/
│   │   ├── mod.rs         — Kanban columns and cue card rendering
│   │   ├── cue_card.rs    — Cue card rendering logic
│   │   └── markdown_import.rs — Markdown import for batch cue creation
│   ├── dialog/
│   │   ├── mod.rs         — Dialog submodule imports
│   │   ├── agent_log.rs   — Agent execution log viewer
│   │   ├── create_pr.rs   — GitHub pull request creation dialog
│   │   ├── cue_agent_runs.rs — Per-cue agent run history
│   │   ├── diff_review.rs — Diff review with reply, search, accept/reject
│   │   ├── git_init.rs    — Git init confirmation for non-repo directories
│   │   ├── import_pr.rs   — Import PR review findings as cues
│   │   ├── merge_conflicts.rs — Merge conflict resolution dialog
│   │   ├── play_variables.rs — Playbook template variable input dialog
│   │   ├── pull_diverged.rs — Diverged branch merge/rebase strategy picker
│   │   ├── pull_unmerged.rs — Unmerged files guidance dialog
│   │   ├── repo.rs        — Repository picker and recent repos
│   │   ├── running_log.rs — Live conversation viewer for Claude/OpenCode output
│   │   └── settings.rs   — Settings panel (theme, model, fonts, sources, agents)
│   ├── notifications.rs   — macOS notifications (three-tier fallback)
│   ├── panels.rs          — Menu bar, repo bar, file tree, status bar, prompt field
│   ├── search.rs          — In-file and project-wide search
│   ├── sources_poll.rs    — Background polling of external cue sources
│   ├── tasks.rs           — Background task lifecycle and cancellation
│   └── theme.rs           — Theme system, semantic colors, font loading
├── claude.rs              — Claude Code CLI invocation and stream parsing
├── opencode.rs            — OpenCode CLI support (multi-provider backend)
├── db.rs                  — SQLite persistence (cues, executions, agent runs, migrations)
├── diff_view.rs           — Unified diff parsing, inline and side-by-side rendering
├── error.rs               — Unified error types (DirigentError, Result alias)
├── file_tree.rs           — Recursive directory scanning with ignore patterns
├── git.rs                 — Git status, history, commit, worktree operations
├── prompt_hints.rs        — Heuristic prompt quality analysis and warnings
├── prompt_suggestions.rs  — Prompt improvement suggestions (context, specificity)
├── settings.rs            — Themes, fonts, model, sources, agents, playbook
└── sources.rs             — External cue sources (GitHub Issues, custom commands)
```

**Key dependencies:**

| Crate | Purpose |
|-------|---------|
| [eframe](https://crates.io/crates/eframe) 0.33 / [egui](https://crates.io/crates/egui) | Cross-platform immediate-mode GUI |
| [egui_extras](https://crates.io/crates/egui_extras) (syntect) | Syntax highlighting |
| [rusqlite](https://crates.io/crates/rusqlite) 0.38 | SQLite (bundled) |
| [git2](https://crates.io/crates/git2) 0.20 | Git operations via libgit2 |
| [notify](https://crates.io/crates/notify) | Cross-platform filesystem watching |
| [pulldown-cmark](https://crates.io/crates/pulldown-cmark) | Markdown parsing for conversation and import rendering |
| [rfd](https://crates.io/crates/rfd) | Native file dialogs |
| [thiserror](https://crates.io/crates/thiserror) | Ergonomic error type definitions |

## Status

Dirigent is in active development (v0.3.1, 311 commits). Core features work — file browsing, code symbols, cue management, Claude Code and OpenCode integration with conversation history and reply workflow, diff review, GitHub PR creation and import, git operations with merge conflict resolution, search, source integration, image attachments, prompt quality hints, post-run agents, and macOS app bundling — but expect rough edges. Contributions and feedback are welcome.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

Copyright 2026 Lars Gregori
