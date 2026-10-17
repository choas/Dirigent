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
| Pre-run / Post-run Script | Shell commands around each run, like the other providers; they only run in repositories whose settings you approved | `cargo fmt` |
| Env Variables | `KEY=VAL`, one per line; `.Dirigent/.env` is applied too | `GOOSE_PROVIDER=anthropic` |

4. Click **Test Connection**. Dirigent sends `hello` to the agent and shows its reply (or the error) next to the button. See [Testing the connection](#testing-the-connection).
5. Create a cue and run it. The running log shows `[ACP: <agent name> <version>]`, then streamed text, `→ <tool>` lines, plan checklists and `✗` lines for failed tools or denied permissions.

## Agents

| Agent | Agent Command | Extra Arguments | Status | Notes |
|---|---|---|---|---|
| Claude Code | `npx` | `-y @agentclientprotocol/claude-agent-acp@0.90.0` | ✅ verified (0.90.0) | Uses your existing `claude` login. Formerly `@zed-industries/claude-code-acp`, which is no longer updated. Pin a version; with a global install (`npm i -g @agentclientprotocol/claude-agent-acp`) the command is `claude-agent-acp` with no arguments |
| OpenCode | `opencode` | `acp` | ✅ verified (1.18.30) | Uses OpenCode's own provider login and model |
| Codex | `npx` | `-y @zed-industries/codex-acp@0.16.0` | ⚠️ connects, prompts fail | 0.16.0 (June 2026) bundles an older Codex core and rejects newer models from `~/.codex/config.toml` (e.g. `gpt-6-luna`) with `-32603 Internal error`. Starts sessions in read-only mode, so edits need **Permissions** on |
| Gemini CLI | `gemini` | `--experimental-acp` | not verified | `npm i -g @google/gemini-cli`, then run `gemini` once to log in |
| Goose | `goose` | `acp` | not verified | Needs a provider key in **Env Variables**, e.g. `GOOSE_PROVIDER=anthropic` and `ANTHROPIC_API_KEY=…` |
| Anything else | path to the binary | as documented by the agent | | Must speak ACP v1 on stdin/stdout |

"Verified" means both smoke tests below passed on 2026-10-17. Flags and package names change quickly; check the agent's own docs or the registry at <https://agentclientprotocol.com> if one does not start.

Running an agent through `npx` downloads it on first use, so the first run (or connection test) can take a while. If your npm config sets a minimum release age, npm refuses versions newer than that; pick an older version if the pinned one is too new.

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
6. When the turn ends, stdin is closed, so the agent exits. SIGTERM and then SIGKILL go to its process group if it lingers. Then the post-run script runs, also after an error or a cancel.
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

## Testing the connection

**Settings → General → Test Connection** works for every CLI provider, not just ACP. It sends the prompt `hello` with the settings currently on screen (unsaved edits included) and passes if a non-empty reply comes back:

- `✓ <first line of the reply> (3.2s)`: the CLI started, authenticated and answered. Hover for the full reply.
- `✗ <error>`: the CLI failed to start, needs authentication, returned an error, or replied with nothing. Hover for the full message, which includes the agent's stderr when it exited.
- `no reply after 120s`: the test was cancelled. Agents started through `npx` may still be downloading; try again.

Pre-run and post-run scripts are skipped. ACP agents run read-only, as they do for analysis prompts. The test calls the model, so it costs a few tokens.

## Smoke-testing an agent

Two ignored tests run a real agent in a temporary directory: one with a read-only prompt, and one that edits a file.

```sh
ACP_SMOKE_AGENT="opencode acp" cargo test acp_smoke -- --ignored --nocapture
ACP_SMOKE_AGENT="npx -y @agentclientprotocol/claude-agent-acp@0.90.0" cargo test acp_smoke -- --ignored --nocapture
```

Both call the agent's model, so they cost a few tokens.

The connection test has a matching ignored test. It uses this repository's saved settings, and `ACP_SMOKE_AGENT` when set:

```sh
CONNECTION_SMOKE_PROVIDER=claude cargo test connection_smoke -- --ignored --nocapture
```
