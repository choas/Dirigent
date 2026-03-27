# OpenCode Integration

Dirigent supports [OpenCode](https://github.com/nicepkg/opencode) as an alternative CLI backend to Claude Code. This document covers configuration, invocation details, and permission tips.

## Running OpenCode with Dirigent

1. Install OpenCode: follow [OpenCode installation instructions](https://github.com/nicepkg/opencode)
2. In Dirigent, go to **Settings** and set the CLI backend to **OpenCode**
3. Configure (all optional):
   - **CLI Path** — path to the `opencode` binary. Auto-detected on startup by searching the login shell PATH, plain PATH, and well-known directories (Homebrew, npm, `.local`, `.nvm`). Leave empty if auto-detection works.
   - **Model** — selected from a dropdown populated by `opencode models`. Default models include `openai/o1`, `openai/o3`, `openai/o4-mini`, `openai/gpt-4.1`, `anthropic/claude-sonnet-4-6`, `anthropic/claude-opus-4-6`. Click the refresh button (↻) to re-fetch available models.
   - **Extra Args** — additional CLI arguments passed to `opencode run` (parsed via `shlex`, so quoting works)
   - **Pre-run Script** — shell command executed before each run (e.g. set up credentials, pull latest config). Failures abort the run.
   - **Post-run Script** — shell command executed after each run (e.g. cleanup, notifications). Failures are logged but don't affect results.
   - **Environment Variables** — env var **names** to forward to the CLI process (one per line in the settings JSON). Values are resolved from the current process environment at runtime, never stored.

### How Dirigent Invokes OpenCode

Dirigent runs OpenCode with these default flags:

```
opencode run "<prompt>" --format json --print-logs --model <model>
```

Any **Extra Args** from settings are appended after these. Environment variables from `.Dirigent/.env` in the project root are automatically injected into the CLI process (same mechanism as the Claude backend), so you can keep AI-specific credentials separate from your regular `.env`.

### Cancellation

Dirigent supports cancelling a running OpenCode invocation. A watchdog thread monitors a cancellation flag and kills the child process when triggered.

## Yolo Mode (Auto-Approve All Permissions)

OpenCode has a permission system that prompts for confirmation on potentially dangerous actions (writing files, running commands, accessing external directories, etc.). **Yolo mode** auto-approves everything so that OpenCode runs non-interactively — which is how Dirigent invokes it via `opencode run`.

### Default Permission Ruleset

When running in the TUI, OpenCode uses a ruleset like this (visible in logs at `INFO` level). Rules are evaluated in order and the **last matching rule wins**, so later entries override earlier ones for the same permission and pattern:

```jsonc
[
  {"permission": "*",                   "action": "allow",  "pattern": "*"},
  {"permission": "doom_loop",           "action": "ask",    "pattern": "*"},
  {"permission": "external_directory",  "action": "ask",    "pattern": "*"},
  {"permission": "external_directory",  "action": "allow",  "pattern": "/Users/<you>/.local/share/opencode/tool-output/*"},
  {"permission": "question",            "action": "deny",   "pattern": "*"},   // denied here…
  {"permission": "plan_enter",          "action": "deny",   "pattern": "*"},   // denied here…
  {"permission": "plan_exit",           "action": "deny",   "pattern": "*"},
  {"permission": "read",               "action": "allow",  "pattern": "*"},
  {"permission": "read",               "action": "ask",    "pattern": "*.env"},
  {"permission": "read",               "action": "ask",    "pattern": "*.env.*"},
  {"permission": "read",               "action": "allow",  "pattern": "*.env.example"},
  {"permission": "question",            "action": "allow",  "pattern": "*"},   // …overridden to allow (last match wins)
  {"permission": "plan_enter",          "action": "allow",  "pattern": "*"},   // …overridden to allow (last match wins)
  {"permission": "external_directory",  "action": "allow",  "pattern": "/Users/<you>/.local/share/opencode/tool-output/*"}
]
```

### Key Permissions

| Permission | Description |
|---|---|
| `*` | Catch-all: file writes, bash commands, etc. |
| `read` | Reading files (`.env` files get special treatment) |
| `external_directory` | Accessing files outside the project root |
| `doom_loop` | Detects when the model is stuck in a loop |
| `question` | Model asking the user a question |
| `plan_enter` / `plan_exit` | Entering/exiting plan mode |

### Actions

- **`allow`** — auto-approve without prompting
- **`ask`** — prompt the user for confirmation
- **`deny`** — silently deny the action

### Making It Fully Non-Interactive

For Dirigent's `opencode run` invocation (headless, no TUI), you want all permissions auto-approved. The key rules:

1. **`{"permission": "*", "action": "allow", "pattern": "*"}`** — allows all actions by default
2. **`question` → `deny`** — prevents the model from asking questions (no one to answer)
3. **`plan_enter` / `plan_exit` → `deny`** — prevents plan mode (not useful headless)

If OpenCode still prompts or blocks, check the log for `service=permission` entries to see which permission is being evaluated.

### Configuration

OpenCode reads project-level config from a `.opencode` file (JSON or YAML) in the project root. To set up Yolo mode, create `.opencode/config.json` or configure permissions via the OpenCode TUI settings.

Alternatively, when running via Dirigent, you can pass extra arguments in **Settings > Extra Args** that OpenCode supports for the `run` subcommand.

### Safety Considerations

Running in Yolo mode means:
- The AI can write/delete any file in the project (and potentially outside it)
- Arbitrary bash commands execute without confirmation
- `.env` files and secrets can be read without prompting

This is acceptable for local development with version control (you can always `git checkout` or `git stash`), but **do not use Yolo mode in production environments or on machines with sensitive credentials**.

## Stream Processing & Log Filtering

Dirigent processes OpenCode's JSON event stream from stdout and structured log lines from stderr in real time.

### JSON Events (stdout)

Events are dispatched by type:

| Event Type | Handling |
|---|---|
| `text` | Streamed text output displayed in the log |
| `tool_use` / `tool` | Tool invocations — file-modifying tools (Write, Edit, Bash, Task, etc.) are tracked for diff calculation |
| `step_finish` | Collects metrics (cost, tokens, duration) when reason is `"stop"` |
| `error` | Error messages displayed in the log |

### Structured Logs (stderr)

OpenCode emits structured log lines on stderr (`INFO`, `DEBUG`, `WARN`, `ERROR` with ISO timestamps). Dirigent filters these:

- **DEBUG** — dropped (too noisy)
- **WARN / ERROR** — always passed through
- **INFO** — formatted per service:

| Service | Display |
|---|---|
| `service=llm` | `→ <modelID> (<providerID>)` |
| `service=permission` | `→ <permission> — <pattern>` |
| `service=format` | `→ format: <file>` |
| `service=session` | `→ session: <slug>` |
| `service=vcs` | `→ branch: <name>` |
| `service=lsp` | `→ lsp: <method>` |
| `service=session.prompt` | `→ step <N>` or `→ loop done` |

## Debugging

Enable verbose logging to see permission decisions and stream events:

```sh
opencode run "your prompt" --print-logs --log-level DEBUG --format json
```

Look for `service=permission` log lines to understand which permissions are being evaluated and whether they are allowed or denied.
