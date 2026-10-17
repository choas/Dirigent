# How to run ACP agents from Dirigent

Dirigent can drive any agent that speaks the **Agent Client Protocol (ACP) v1** over stdio. ACP is the editor↔agent protocol Zed introduced (think "LSP for coding agents"). With the **ACP Agent** provider selected, a cue is sent to the agent as an ACP prompt. You still get the usual cue → running log → diff review → commit workflow.

Background and design: [acp-concept.md](acp-concept.md). The reverse direction, where Dirigent is the agent (for example inside Zed), is a concept only: [acp-agent-concept.md](acp-agent-concept.md).

## When to use it

- **Claude / Codex / Gemini / OpenCode providers**: use these for daily work. They drive each CLI directly and support model pickers, PTY mode, cost metrics and session reuse.
- **ACP Agent**: use this for agents that have no dedicated provider (Goose, Junie, Kiro, OpenHands, custom agents…). It's also a way to compare agents through one uniform protocol.

## Setup

1. Install an ACP-capable agent (see the table below) and authenticate it with its own CLI once. Dirigent cannot run interactive logins.
2. Open **Settings → General**, set **CLI Provider** to **ACP Agent**.
3. Fill in:

| Field | Meaning | Example |
|---|---|---|
| Agent Command | Executable on `PATH`, or an absolute path | `opencode` |
| Extra Arguments | Shell-quoted arguments; most agents need an "ACP mode" flag here | `acp` |
| Permissions | Auto-approve the agent's permission requests (shared with Claude's skip-permissions toggle) | on |
| Pre-run / Post-run Script | Shell commands around each run, like the other providers | `cargo fmt` |
| Env Variables | `KEY=VAL`, one per line; `.Dirigent/.env` is applied too | `GOOSE_PROVIDER=anthropic` |

4. Create a cue and run it. The running log shows `[ACP: <agent name> <version>]`, then streamed text, `→ <tool>` lines, plan checklists and `✗` lines for failed tools or denied permissions.

## Agents

| Agent | Agent Command | Extra Arguments | Notes |
|---|---|---|---|
| OpenCode | `opencode` | `acp` | Verified against OpenCode 1.18.30 |
| Gemini CLI | `gemini` | `--experimental-acp` | `npm i -g @google/gemini-cli` |
| Goose | `goose` | `acp` | |
| Claude Code | `claude-code-acp` | | Zed's adapter (`npm i -g @zed-industries/claude-code-acp`); newer releases may be named `claude-agent-acp` |
| Codex | `codex-acp` | | Zed's adapter for Codex CLI |
| Anything else | path to the binary | as documented by the agent | Must speak ACP v1 on stdin/stdout |

Flags and package names change quickly; check the agent's own docs or the registry at <https://agentclientprotocol.com> if one does not start. Only OpenCode was verified end to end when this page was written. The smoke test below lets you check any other agent the same way.

## What happens during a run

1. Dirigent runs the pre-run script, then spawns the agent in the project root, in its own process group.
2. **initialize**: Dirigent advertises `fs.readTextFile`, and `fs.writeTextFile` unless the run is read-only. It does not offer terminals.
3. **session/new**: `cwd` is the project root and no MCP servers are passed.
4. **session/prompt**: the cue prompt is sent as one text block, built exactly as it is for the other providers.
5. While the turn runs, Dirigent answers the agent's requests:
   - `fs/read_text_file`: files inside the project root only (symlinks are resolved first).
   - `fs/write_text_file`: inside the project root only; written files are tracked for the diff.
   - `session/request_permission`: see [Permissions](#permissions).
   - Anything else (such as `terminal/create`): answered with JSON-RPC "method not found", so the agent falls back to its own tools.
6. When the turn ends, stdin is closed, so the agent exits. SIGTERM and then SIGKILL go to its process group if it lingers. Then the post-run script runs.
7. The diff is computed like it is for every other provider: the working tree is the source of truth, scoped to the files the agent reported editing.

**Cancel** sends `session/cancel`. Dirigent waits up to 5 s for the agent to wind down, then kills it.

## Permissions

ACP agents ask before risky actions. Dirigent answers on its own, because there's no interactive dialog yet:

| Run | Read / search / fetch / think | Edit / execute / delete / other |
|---|---|---|
| Cue run, auto-approve **on** | allowed once | allowed once |
| Cue run, auto-approve **off** | rejected | rejected (logged as `✗ permission denied`) |
| Analysis prompts (split cue, workflow planning, AI theme) | allowed once | rejected, and file writes are refused |

Dirigent always picks *allow once* rather than *allow always*. Some adapters persist "always" decisions into the agent's own config files, and a one-shot run shouldn't leave that behind.

File access through `fs/*` is confined to the project root regardless of these settings. Agents that edit files with their own built-in tools (most do) are confined only by their own sandbox, the same as with the other providers.

## Limitations

- One fresh agent process and session per run: no session resume or follow-up replies within the agent's context.
- Text prompts only: image attachments on cues are not forwarded.
- No model picker: the agent uses its own configured model.
- No cost or token metrics: ACP v1 does not report them. Duration and completed tool calls are recorded.
- No MCP forwarding and no client-hosted terminals.
- Agents that need interactive authentication fail with "requires authentication". Log in with the agent's CLI first.

## Smoke-testing an agent

Two ignored tests run a real agent in a temporary directory: one with a read-only prompt, and one that edits a file.

```sh
ACP_SMOKE_AGENT="opencode acp" cargo test acp_smoke -- --ignored --nocapture
```

Both call the agent's model, so they cost a few tokens.
