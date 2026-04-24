# Getting Started with Dirigent

This tutorial walks you through installing Dirigent, opening your first project, creating a cue, running it with Claude Code, and reviewing the result. By the end you will understand the core workflow.

---

## 1. Install prerequisites

You need Rust and Claude Code CLI on your machine.

**Rust** (1.75+):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**Claude Code CLI** — install and authenticate:

```bash
npm install -g @anthropic-ai/claude-code
claude  # follow the auth flow once
```

**macOS** also requires Xcode Command Line Tools:

```bash
xcode-select --install
```

**Linux** (Debian/Ubuntu) requires GTK and OpenSSL headers:

```bash
sudo apt install libgtk-3-dev libxcb-shape0-dev libxcb-xfixes0-dev libssl-dev pkg-config
```

## 2. Install Dirigent

### Option A: Build from source (CLI)

```bash
git clone https://github.com/choas/Dirigent.git
cd Dirigent
cargo build --release
```

The binary is at `target/release/dirigent`. To install it as a CLI command (requires `~/.cargo/bin` in PATH):

```bash
cargo install --path .
```

### Option B: macOS application (DMG)

Download the latest `.dmg` from [GitHub Releases](https://github.com/choas/Dirigent/releases), open it, and drag **Dirigent.app** into your `/Applications` folder. No Rust toolchain required.

## 3. Open a project

### CLI

If you installed via `cargo install --path .`:

```bash
dirigent /path/to/your/project
dirigent .
```

### macOS application

- **Double-click Dirigent.app** — opens the **repository picker** where you can browse to a project
- **From Terminal** — open a specific project directory:
  ```bash
  open -a Dirigent --args /path/to/your/project
  open -a Dirigent --args .
  ```
- **Drag & drop** — drag a project folder onto the Dirigent icon in the Dock

If you omit the path, Dirigent opens the **repository picker** so you can choose a project.

When Dirigent opens you will see three main areas:

- **File tree** (left) — your project files, with dirty-file indicators from Git
- **Code viewer** (center) — syntax-highlighted, read-only view of the selected file
- **Cue pool** (right) — kanban board tracking your cues through stages

## 4. Browse your code

Click a file in the tree to open it. Key navigation shortcuts:

| Shortcut | Action |
|----------|--------|
| Cmd+P | Quick Open — fuzzy file finder |
| Cmd+W | Close current tab |
| Cmd+[ / Cmd+] | Navigate back / forward |
| Cmd+F | Search in current file |
| Cmd+Shift+F | Search across all files |

The code viewer is **read-only** by design — you direct, the AI performs.

## 5. Create your first cue

A **cue** is an instruction for the AI, optionally anchored to specific lines of code.

1. In the code viewer, click and drag to **select one or more lines**
2. A text input appears — type what you want changed, for example: `Add input validation for the email parameter`
3. Press **Enter** to create the cue

The cue appears in the **Inbox** column of the cue pool on the right. You can also create cues without selecting lines by typing in the **prompt field** at the bottom.

## 6. Run the cue

Click the **Run** button on your cue card. Dirigent sends the prompt to Claude Code CLI and streams progress in real time. A lava lamp animation plays while the cue is running.

When Claude finishes:
- The cue moves to the **Review** column
- A macOS notification sounds (if enabled)
- The diff is ready for review

## 7. Review the diff

Click the cue card in the Review column to open the **diff review dialog**. You will see:

- **Inline diff** — additions in green, deletions in red, collapsible per file
- **Side-by-side** toggle for a two-column comparison
- **+/- statistics** per file showing lines added and removed
- **Search** (Cmd+F) within the diff

You have several options:

| Action | What it does |
|--------|-------------|
| **Accept** | Applies the changes to your working tree |
| **Reject** | Discards the diff; the cue moves back so you can retry |
| **Reply** | Send follow-up feedback for iterative refinement |

## 8. Commit the changes

After accepting a diff, the cue moves to **Done**. You can now commit:

1. Click the **Commit** button on the Done cue card
2. Dirigent creates a Git commit with the cue text as the commit message
3. Optionally push to remote

You can also use **Commit All** to commit all Done cues at once.

## 9. Next steps

You now know the core loop: **Browse > Cue > Run > Review > Commit**. Here are some things to explore next:

- **Cue commands** — prefix a cue with `[plan]`, `[test]`, `[review]`, or `[refactor]` for specialized prompts. See [Reference](reference.md#cue-commands).
- **Playbook** — run predefined prompts like "Security audit" or "Add tests" from the cue pool menu. See [Reference](reference.md#playbook).
- **Sources** — import cues from GitHub Issues, Slack, SonarQube, or other tools. See [How-To Guides](how-to.md#import-cues-from-external-sources).
- **Agents** — configure post-run automation (format, lint, build, test). See [How-To Guides](how-to.md#set-up-post-run-agents).
- **Settings** — customize themes, fonts, models, and more via the Settings dialog. See [Reference](reference.md#settings).
- **Observability** — set up OpenTelemetry dashboards. See [Observability Setup](observability-setup.md).
