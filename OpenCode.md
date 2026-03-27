# OpenCode Integration

Dirigent supports [OpenCode](https://github.com/nicepkg/opencode) as an alternative CLI backend to Claude Code. This document covers configuration tips, particularly around permissions.

## Yolo Mode (Auto-Approve All Permissions)

OpenCode has a permission system that prompts for confirmation on potentially dangerous actions (writing files, running commands, accessing external directories, etc.). **Yolo mode** auto-approves everything so that OpenCode runs non-interactively — which is how Dirigent invokes it via `opencode run`.

### Default Permission Ruleset

When running in the TUI, OpenCode uses a ruleset like this (visible in logs at `INFO` level):

```json
[
  {"permission": "*",                   "action": "allow",  "pattern": "*"},
  {"permission": "doom_loop",           "action": "ask",    "pattern": "*"},
  {"permission": "external_directory",  "action": "ask",    "pattern": "*"},
  {"permission": "external_directory",  "action": "allow",  "pattern": "/Users/<you>/.local/share/opencode/tool-output/*"},
  {"permission": "question",            "action": "deny",   "pattern": "*"},
  {"permission": "plan_enter",          "action": "deny",   "pattern": "*"},
  {"permission": "plan_exit",           "action": "deny",   "pattern": "*"},
  {"permission": "read",               "action": "allow",  "pattern": "*"},
  {"permission": "read",               "action": "ask",    "pattern": "*.env"},
  {"permission": "read",               "action": "ask",    "pattern": "*.env.*"},
  {"permission": "read",               "action": "allow",  "pattern": "*.env.example"},
  {"permission": "question",            "action": "allow",  "pattern": "*"},
  {"permission": "plan_enter",          "action": "allow",  "pattern": "*"},
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

## Running OpenCode with Dirigent

1. Install OpenCode: follow [OpenCode installation instructions](https://github.com/nicepkg/opencode)
2. In Dirigent, go to **Settings** and set the CLI backend to **OpenCode**
3. Optionally configure:
   - **CLI Path** — path to the `opencode` binary (leave empty if it's on your PATH)
   - **Model** — e.g. `anthropic/claude-sonnet-4-20250514`, `openai/o3`, `google/gemini-2.5-pro`
   - **Extra Args** — additional CLI arguments passed to `opencode run`
   - **Environment Variables** — env var names to forward (e.g. `ANTHROPIC_API_KEY`)

## Debugging

Enable verbose logging to see permission decisions and stream events:

```sh
opencode run "your prompt" --print-logs --log-level DEBUG --format json
```

Look for `service=permission` log lines to understand which permissions are being evaluated and whether they are allowed or denied.
