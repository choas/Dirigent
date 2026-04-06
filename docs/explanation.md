# Explanation

Understanding Dirigent's architecture, design decisions, and core concepts.

---

## Table of Contents

- [What is Dirigent?](#what-is-dirigent)
- [The conductor metaphor](#the-conductor-metaphor)
- [Why read-only?](#why-read-only)
- [Core concepts](#core-concepts)
- [Architecture overview](#architecture-overview)
- [Data flow](#data-flow)
- [Persistence model](#persistence-model)
- [AI backend abstraction](#ai-backend-abstraction)
- [The agent system](#the-agent-system)
- [Workflow planning](#workflow-planning)
- [Security boundaries](#security-boundaries)

---

## What is Dirigent?

Dirigent is a desktop application for AI-assisted code editing. Unlike AI-powered IDEs that let you edit code directly alongside AI suggestions, Dirigent enforces a strict separation: you browse code **read-only** and create **cues** (natural-language instructions), then an AI backend (Claude Code or OpenCode) executes the changes. You review the resulting diffs before accepting them into your working tree.

This makes Dirigent a coordination layer between human intent and AI execution, rather than a code editor.

## The conductor metaphor

The name "Dirigent" comes from the German word for *conductor* (as in an orchestra conductor). The metaphor is intentional:

- **You** are the conductor -- you decide what needs to happen, in what order, and how the pieces fit together
- **The AI** is the performer -- it executes the instructions with technical skill
- **Cues** are the conductor's gestures -- precise instructions that communicate intent

This separation of concerns means you focus on the *what* and *why*, while the AI handles the *how*.

## Why read-only?

The code viewer is deliberately read-only. This is a design choice, not a limitation:

1. **Clear accountability** -- every change comes from either the AI (via diff) or a direct git commit. There is no ambiguity about who wrote what.
2. **Reviewability** -- all AI changes go through the diff review step before entering your working tree. You cannot accidentally accept unreviewed changes.
3. **Focus on direction** -- instead of switching between editing and prompting, you stay in "director" mode: reading code, understanding context, and writing precise instructions.

If you need to make quick manual edits, use your preferred editor alongside Dirigent. Dirigent watches the filesystem and updates its view automatically.

## Core concepts

### Cue

A **cue** is the central unit of work. It contains:

- **Text** -- a natural-language instruction describing what should change
- **File path** (optional) -- the file the cue targets
- **Line range** (optional) -- specific lines the cue references
- **Status** -- where the cue is in its lifecycle (Inbox, Ready, Review, Done, Archived)
- **Attachments** (optional) -- images for the AI to reference
- **Tags** (optional) -- for organization

Cues persist in a SQLite database, surviving app restarts.

### Cue commands

A cue can be prefixed with a **command** in brackets (e.g. `[test]`, `[plan]`, `[review]`) to apply a specialized prompt template. Commands shape how the AI interprets the instruction -- for example, `[plan]` runs in plan-only mode and produces no code changes, while `[test]` instructs the AI to write comprehensive tests.

### Cue pool

The **cue pool** is a kanban board that visualizes cue lifecycle. Columns represent status stages:

```
Backlog --> Inbox --> Ready --> (Running) --> Review --> Done --> Archived
```

- **Backlog** -- long-term items, parked for later
- **Inbox** -- newly created or imported cues
- **Ready** -- queued for AI execution
- **Review** -- AI has produced a diff; awaiting human review
- **Done** -- diff accepted; ready for git commit
- **Archived** -- completed and filed away

### Diff review

When the AI finishes, its output is parsed into a **unified diff**. The diff review dialog shows additions and deletions per file, with inline or side-by-side display. You can search within the diff, reply with feedback for refinement, or accept/reject the changes.

### Playbook

The **playbook** is a collection of predefined prompts for common tasks (security audits, test generation, documentation updates, etc.). Plays can include template variables that prompt for user input before running.

## Architecture overview

Dirigent is built with Rust using the [egui](https://github.com/emilk/egui) immediate-mode GUI framework via [eframe](https://github.com/emilk/egui/tree/master/crates/eframe). The architecture follows a single-threaded UI loop with background threads for I/O-heavy work.

### Source layout

```
src/
  main.rs          -- Entry point, window config, macOS integration
  app/             -- UI state, rendering, panels, dialogs
  claude/          -- Claude Code CLI invocation and output parsing
  db/              -- SQLite persistence layer
  git/             -- Git operations (status, commit, worktree, PR)
  agents/          -- Post-run automation (format, lint, build, test)
  lsp/             -- Language Server Protocol client
  settings/        -- Configuration, themes, commands, playbook
  sources/         -- External cue source integrations
  diff_view.rs     -- Diff parsing and rendering
  file_tree.rs     -- Directory scanning with ignore patterns
  workflow.rs      -- Multi-cue workflow planning
```

### Key design choices

- **Immediate-mode GUI** -- egui redraws the entire UI every frame. This simplifies state management (no widget trees to synchronize) but means all rendering code must be fast. Heavy work runs on background threads.
- **SQLite for persistence** -- cues, executions, and agent runs are stored in SQLite rather than flat files. This provides ACID transactions, easy querying, and schema migrations.
- **CLI subprocess model** -- instead of embedding an AI SDK, Dirigent shells out to `claude` or `opencode` as subprocesses. This keeps the AI integration simple, updatable independently, and authentication-free (the CLI handles auth).
- **File watching** -- the `notify` crate monitors the project directory. When files change on disk (from the AI, another editor, or git operations), Dirigent automatically rescans and updates the file tree and code viewer.

## Data flow

A typical cue execution follows this path:

```
1. User creates cue (text + optional file/lines)
       |
2. Cue saved to SQLite (status: Inbox)
       |
3. User clicks Run
       |
4. Prompt built: cue text + context (file content, git diff, etc.)
       |
5. claude/opencode CLI spawned as subprocess
       |
6. Streaming output parsed in real time (stderr for progress, stdout for result)
       |
7. Diff extracted from AI output
       |
8. Cue moves to Review; diff stored in DB
       |
9. User reviews diff (inline or side-by-side)
       |
10. Accept: changes applied to working tree; cue moves to Done
    Reject: diff discarded; cue returns to Inbox
    Reply: feedback sent; new execution starts from step 5
       |
11. User commits (via Dirigent or external tool)
       |
12. Cue moves to Archived
```

## Persistence model

Each project has a `.Dirigent/` directory at the project root containing:

- **Dirigent.db** -- SQLite database with tables for cues, executions, agent runs, and source imports. Schema is versioned with incremental migrations.
- **settings.json** -- project-specific settings (theme, model, agents, sources, etc.)
- **.env** -- environment variables forwarded to AI CLI invocations

This per-project approach means settings and cue history travel with the project directory. When using git worktrees, each worktree gets its own `.Dirigent/` directory and database. Databases from removed worktrees are archived.

## AI backend abstraction

Dirigent supports two CLI backends:

### Claude Code

The primary backend. Dirigent invokes the `claude` CLI with a prompt on stdin, parses streaming output from stderr (progress messages) and stdout (final result), and extracts diffs from the response. The model is configurable (Opus, Sonnet, etc.).

### OpenCode

An alternative backend that supports multiple AI providers (OpenAI, Anthropic, Google). Dirigent invokes `opencode` similarly but with provider-specific model strings (e.g. `openai/o3`, `google/gemini-2.5-pro`).

Both backends produce diffs in the same format, so the review workflow is identical regardless of provider.

## The agent system

**Agents** are post-execution automation steps. After the AI produces a diff (or after a commit), agents can automatically run shell commands like formatters, linters, build tools, or test suites.

Agents are configured with:
- A shell **command** (e.g. `cargo fmt`, `npm run lint`)
- A **trigger** that determines when they run (AfterRun, AfterCommit, AfterAgent, OnFileChange, Manual)

Agent chaining via the `AfterAgent` trigger allows pipelines: format, then lint, then build, then test. Agent output is captured and displayed in a log viewer; cargo JSON diagnostics are parsed into structured error messages with file/line references.

## Workflow planning

When multiple cues are in the Inbox, the **workflow planner** uses the AI to analyze dependencies between them and produce an execution plan:

- **Independent cues** are grouped for parallel execution
- **Dependent cues** are ordered sequentially (e.g. "add the database table" must run before "write the API endpoint that queries it")

The plan is visualized as a graph overlay in the UI, and execution proceeds step by step with progress indicators.

## Security boundaries

Dirigent implements several security measures:

- **Home directory guard** -- an optional Claude Code hook that blocks AI tool calls targeting personal directories (Documents, Desktop, Downloads, etc.). This prevents the AI from accessing or modifying files outside the project. Configurable in Settings.
- **Read-only code viewer** -- the UI cannot modify files directly. All file modifications flow through the AI diff + review pipeline or explicit git operations.
- **CLI subprocess isolation** -- the AI runs as a separate process. Dirigent does not embed AI model weights or make direct API calls; it relies on the CLI's own authentication and sandboxing.
- **LSP command sanitization** -- language server command strings are validated to prevent shell injection when spawning LSP processes.
