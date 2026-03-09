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
- **Cue system** — Select lines in the code viewer and create cues describing what you want changed; each cue carries a file path, line range, and natural-language prompt
- **Cue pool (kanban)** — Track cues through stages: Inbox → Ready → Review → Done → Archived
- **Claude Code integration** — Sends prompts to Claude Code CLI and streams stderr progress in real time; configurable model (Opus 4.6, Sonnet 4.6)
- **Diff view** — Review AI-generated changes inline or side-by-side before accepting; collapsible per-file diffs with +/- statistics
- **Git integration** — View commit history (last 50 commits), create commits, manage worktrees, see dirty-file indicators in the file tree
- **Search** — In-file search (Cmd+F) with match navigation; project-wide search (Cmd+Shift+F) with clickable results
- **Source integration** — Import cues from GitHub Issues, Notion, MCP, or custom shell commands with automatic deduplication and configurable polling
- **Playbook** — Predefined prompts (e.g. "Update README", "Security audit", "Add tests") that can be run as global cues
- **Themes** — 20 themes: 10 dark (Nord, Dracula, Monokai, Gruvbox, Tokyo Night, One Dark, Catppuccin Mocha, Everforest, Solarized) and 10 light (Solarized, Gruvbox, GitHub, Catppuccin Latte, Everforest, Rose Pine Dawn, One Light, Nord, Tokyo Night)
- **File watching** — Automatic filesystem monitoring with debounced rescan when files change on disk
- **Notifications** — macOS sound (Glass.aiff) and popup notifications when Claude finishes a task
- **Settings** — Configure theme, Claude model, font family and size, notification preferences, sources, and playbook
- **Repository picker** — Switch between projects and track recent repositories
- **macOS native** — Custom About panel, dock icon, and notification integration

## Prerequisites

- **Rust** (1.75+ recommended) — [rustup.rs](https://rustup.rs)
- **Claude Code CLI** — Must be installed and available on your `PATH`. See [Claude Code docs](https://docs.anthropic.com/en/docs/claude-code).
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

## Usage

```bash
# Open the current directory
cargo run --release

# Open a specific project
cargo run --release -- /path/to/your/project
```

### Workflow

1. **Browse** — Navigate your codebase in the file tree on the left
2. **Cue** — Select lines in the code viewer and create a cue describing what you want changed
3. **Run** — Click Run on a cue to send it to Claude Code; streaming progress is shown in real time
4. **Review** — Inspect the diff that Claude produced (inline or side-by-side)
5. **Accept or reject** — Commit the changes or revert them
6. **Repeat** — Use the global prompt field or playbook for cues that don't target specific lines

### Per-project data

Dirigent stores its database (`Dirigent.db`) and settings (`settings.json`) in a `.Dirigent/` directory inside your project root. This directory is local to each project and should be added to `.gitignore`.

## Architecture

```
src/
├── main.rs          — Entry point, window setup, macOS integration
├── app/
│   ├── mod.rs       — App state, update loop, panel orchestration
│   ├── code_viewer.rs — Syntax-highlighted code display with cue markers
│   ├── cue_pool.rs  — Kanban columns and cue card rendering
│   ├── dialogs.rs   — Settings, diff review, repo picker, worktree manager
│   ├── panels.rs    — Menu bar, repo bar, file tree, status bar, prompt field
│   └── search.rs    — In-file and project-wide search
├── claude.rs        — Claude Code CLI invocation and stream parsing
├── db.rs            — SQLite persistence (cues, executions, migrations)
├── diff_view.rs     — Unified diff parsing, inline and side-by-side rendering
├── file_tree.rs     — Recursive directory scanning with ignore patterns
├── git.rs           — Git status, history, commit, worktree operations
├── settings.rs      — Themes, fonts, model, sources, playbook
└── sources.rs       — External cue sources (GitHub Issues, custom commands)
```

**Key dependencies:**

| Crate | Purpose |
|-------|---------|
| [eframe](https://crates.io/crates/eframe) / [egui](https://crates.io/crates/egui) | Cross-platform immediate-mode GUI |
| [egui_extras](https://crates.io/crates/egui_extras) (syntect) | Syntax highlighting |
| [rusqlite](https://crates.io/crates/rusqlite) | SQLite (bundled) |
| [git2](https://crates.io/crates/git2) | Git operations via libgit2 |
| [notify](https://crates.io/crates/notify) | Cross-platform filesystem watching |

## Status

Dirigent is in early development (v0.1.0). Core features work — file browsing, cue management, Claude Code integration, diff review, git operations, search, and source integration — but expect rough edges. Contributions and feedback are welcome.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

Copyright 2026 Lars Gregori
