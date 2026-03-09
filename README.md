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

- **File tree & code viewer** — Browse any Git repository with syntax highlighting (via [syntect](https://github.com/trekhleb/syntect))
- **Cue system** — Create cues on code to direct AI; each cue carries a file path, line range, and natural-language prompt
- **Cue pool (kanban)** — Track cues through stages: Inbox → Ready → Review → Done → Archived
- **Claude Code integration** — Sends batched prompts to Claude Code CLI and streams progress in real time
- **Diff view** — Review AI-generated changes with a side-by-side diff before accepting
- **Git integration** — View commit history, create commits, manage worktrees
- **Themes** — 10 dark and 10 light themes (Nord, Solarized, Dracula, Gruvbox, and more)
- **Settings & global prompt** — Configure model, font size, and a global system prompt prepended to every cue
- **macOS native** — Custom About panel and dock icon on macOS

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
3. **Batch** — Move cues from Inbox to Ready when you're satisfied with the prompt
4. **Execute** — Dirigent sends the batch to Claude Code and streams progress
5. **Review** — Inspect the diff that Claude produced
6. **Accept or reject** — Apply the changes or discard them
7. **Commit** — Commit accepted changes directly from the app

### Per-project data

Dirigent stores its database and settings in a `.dirigent/` directory inside your project root. This directory is local to each project and should be added to `.gitignore` (Dirigent's own `.gitignore` already excludes it).

## Architecture

```
src/
├── main.rs        — Entry point, window setup
├── app/           — UI state, panels, code viewer (egui)
├── claude.rs      — Claude Code CLI integration
├── db.rs          — SQLite persistence (cues, executions)
├── diff_view.rs   — Diff parsing and rendering
├── file_tree.rs   — File system navigation
├── git.rs         — Git operations (log, commit, worktrees)
└── settings.rs    — Themes, fonts, preferences
```

**Key dependencies:**

| Crate | Purpose |
|-------|---------|
| [eframe](https://crates.io/crates/eframe) / [egui](https://crates.io/crates/egui) | Cross-platform immediate-mode GUI |
| [egui_extras](https://crates.io/crates/egui_extras) (syntect) | Syntax highlighting |
| [rusqlite](https://crates.io/crates/rusqlite) | SQLite (bundled) |
| [git2](https://crates.io/crates/git2) | Git operations via libgit2 |

## Status

Dirigent is in early development (v0.1.0). Core features work — file browsing, cue management, Claude Code integration, diff review, and git operations — but expect rough edges. Contributions and feedback are welcome.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

Copyright 2026 Lars Gregori
