# ACP in Dirigent: concept

This page covers what the Agent Client Protocol is, why Dirigent speaks it as a **client**, how the implementation is built, and what is left to do. For setup, see [How_to_ACP.md](How_to_ACP.md). The opposite role, Dirigent as an ACP **agent**, is described separately in [acp-agent-concept.md](acp-agent-concept.md) and is not implemented.

> History: this replaces the May 2026 `feature/acp` prototype (`analyze_acp.md`, `How_to_ACP.md`, `what_we_implemented.md`). The prototype's client spoke an earlier, partly invented message shape (`kind`-tagged updates, `session/close`, an `allow_always` permission reply, no `mcpServers`), so it could not work with spec-conformant agents. It was rewritten against the v1 schema; see [What changed from the prototype](#what-changed-from-the-prototype).

## What ACP is

ACP is an open (Apache-2.0) protocol started by Zed Industries that standardises how an editor or other *client* talks to an AI coding *agent*. It is to agents what LSP is to language servers. Any client that implements it can use every compatible agent, and the reverse.

- Spec: <https://agentclientprotocol.com>, repo: <https://github.com/agentclientprotocol/agent-client-protocol>
- Protocol version: a single integer, currently **1**, bumped only on breaking changes. A v2 is being drafted (new prompt lifecycle, message ids, remote transports).
- Transport: newline-delimited JSON-RPC 2.0 over the agent's stdin/stdout. stderr is free-form logging.
- Ecosystem (mid-2026): 30+ agents (Claude Code and Codex via Zed adapters, Gemini CLI, Goose, OpenCode, Junie, Kiro, OpenHands, Copilot, Cline, Qwen Code, …) and 50+ clients (Zed, JetBrains, Neovim, Emacs, VS Code extensions, notebooks, chat bridges).

### Roles

| Role | Responsibility | Dirigent |
|---|---|---|
| **Client** | Spawns the agent and owns the UI, the file system view, permissions and (optionally) terminals | **Implemented**: the "ACP Agent" CLI provider |
| **Agent** | Runs the model loop and tools, streams progress, edits code | Concept only ([acp-agent-concept.md](acp-agent-concept.md)) |

### One prompt turn (v1)

```
Dirigent (client)                                  agent subprocess
  ── initialize {protocolVersion:1, clientCapabilities{fs,terminal}, clientInfo} ─▶
  ◀─ {protocolVersion, agentCapabilities, authMethods, agentInfo} ──
  ── session/new {cwd, mcpServers:[]} ─────────────────────────────▶
  ◀─ {sessionId} ──   (or error -32000 "auth required")
  ── session/prompt {sessionId, prompt:[{type:text,text}]} ────────▶
  ◀─ session/update {sessionId, update:{sessionUpdate:"agent_message_chunk", content}}
  ◀─ session/update {… "tool_call" / "tool_call_update" / "plan" …}
  ◀─ fs/read_text_file {sessionId, path, line?, limit?}   ──▶ {content}
  ◀─ session/request_permission {toolCall, options[]}      ──▶ {outcome:{outcome:"selected", optionId}}
  ◀─ fs/write_text_file {sessionId, path, content}         ──▶ {}
  ◀─ {stopReason: end_turn | max_tokens | max_turn_requests | refusal | cancelled}
  ── (close stdin; agent exits) ──
```

`session/cancel` is a notification. The agent stops, replies `stopReason: cancelled`, and expects pending permission requests to be answered with `outcome: cancelled`.

## Why Dirigent is a client

Dirigent already supported four agents (Claude, Codex, Gemini, OpenCode), each through a custom adapter that parses CLI-specific output. ACP adds a fifth provider that covers *every* ACP agent with one adapter, and the data it gets back is more structured:

- Tool calls arrive typed (`kind`, `status`, `locations`) instead of being scraped from stream-json or stderr.
- Edits arrive as first-class diff content (`path`, `oldText`, `newText`).
- Plans arrive as checklists.
- Permission requests are explicit, so Dirigent decides, not a CLI flag.

The dedicated providers stay. They expose things ACP v1 does not (model selection, PTY mode, cost and token metrics, session reuse), and they are the better default for those CLIs.

## Implementation

Code: `src/acp/` (`types.rs` wire types, `client.rs` connection, `invoke.rs` run lifecycle). It is wired in like every other provider: `CliProvider::Acp`, `acp_*` settings, and `AcpBackend` in `src/ai_backend.rs`. The shared `AiBackend::invoke` driver runs the hooks, the final cancel check and the duration measurement; `acp::run_acp_cli` only runs the turn. Analysis prompts (`split_cue::run_cli_prompt`) set `BackendRequest::read_only`.

### Decisions

| Decision | Why |
|---|---|
| **Direct JSON-RPC implementation, no `agent-client-protocol` crate** | Dirigent needs a small, synchronous client subset (about 10 message types). The crate is async (tokio) and still pre-1.0 with frequent breaking releases, and it would track v2 churn. Inbound types are lenient (`Option`, `#[serde(other)]`), so new fields and update kinds don't break runs. Revisit when v2 lands or if we need much more of the surface. |
| **Threads + channel, no async runtime** | Matches the other providers (`std::thread`, `mpsc`). A reader thread owns stdout and forwards parsed messages. The run thread polls with a 200 ms timeout so cancel is noticed even when the agent is silent. |
| **One process and one session per run** | Matches how cues execute today and keeps failure isolation simple. Resume is future work. |
| **Working tree is the diff source of truth** | ACP diff blocks and edit-tool locations only contribute *paths*. The diff itself goes through the shared `resolve_run_diff` (VCS diff of reported files, then baseline-scoped diff), like every provider. The prototype's whole-file `-old/+new` hunks are gone. |
| **Client fs confined to the project root** | `fs/*` paths are resolved against the canonical project root. Symlinks are resolved on the existing prefix, so a link pointing outside is rejected, and `..` is rejected in the not-yet-existing tail. |
| **Permission policy, not a dialog** | Mirrors `allow_dangerous_skip_permissions`. Analysis prompts are read-only: no `writeTextFile` capability, and only read-like tool kinds are approved. Dirigent prefers *allow once* over *allow always* so adapters don't persist approvals. |
| **No terminals advertised** | Agents fall back to their own command execution, which they all have. Unknown agent requests get `-32601`, so an agent never hangs waiting. |
| **Process-group shutdown** | v1 has no session-close request, so Dirigent closes stdin, then sends SIGTERM and SIGKILL to the group. Adapters such as `claude-code-acp` spawn the real CLI underneath. |

### Mapping to Dirigent concepts

| ACP | Dirigent |
|---|---|
| session | one cue execution |
| prompt | cue text + auto-context, as built for every provider |
| `agent_message_chunk` | streamed into the running log; concatenated text becomes the execution response |
| `tool_call` / `tool_call_update` | `→ title` log lines, `✗` on failure; completed calls are counted as turns |
| diff content, edit/delete/move locations | `edited_files` → shared diff resolution → diff review |
| `plan` | checklist in the running log (logged only when it changes) |
| `request_permission` | policy above; denials are logged |
| `stopReason` | `refusal` / `cancelled` → error; `max_tokens` / `max_turn_requests` → warning |
| `authMethods` + `-32000` | "requires authentication" error naming the methods |

### Testing

- Unit tests cover update parsing and merging, plan de-duplication, the permission policy, path confinement (including symlink escapes) and line windows.
- Scripted-agent tests (a `sh` script speaking ACP) cover the full turn, read-only runs, auth-required, agent crash with stderr, and cancel.
- `acp_smoke_*` (ignored) run a real agent: `ACP_SMOKE_AGENT="opencode acp" cargo test acp_smoke -- --ignored`.

## What changed from the prototype

| Prototype (`feature/acp`, May 2026) | Now |
|---|---|
| Updates parsed as `{kind:{type:…}}` with top-level `text` | v1 shape: `{update:{sessionUpdate:…, content:{type:"text",…}}}` |
| `initialize` sent `capabilities` | `clientCapabilities` (+ `terminal:false`), `clientInfo` with `title` |
| `session/new` without `mcpServers` | `mcpServers: []` (required) |
| Permission reply `{decision:"allow_always"}` | `{outcome:{outcome:"selected", optionId}}`, chosen by policy |
| `fs/write_text_file` replied `{success}`; any path accepted | replies `{}`; confined to the project root; refused on read-only runs |
| Unknown agent requests ignored (agent hangs) | `-32601 method not found` |
| `session/close` request on shutdown | not in v1: close stdin, then terminate the process group |
| Blocking reads; cancel only between messages | reader thread + poll; `session/cancel` with a 5 s grace period |
| ACP diffs → synthetic whole-file hunks | paths only; shared VCS-first diff resolution |
| Analysis prompts could write files | read-only policy for split cue, workflow planning and AI themes |
| stderr discarded | stderr tail included when the agent dies |
| no env vars | env vars field + `.Dirigent/.env`, like other providers |

## Roadmap

Roughly in order of value:

1. **Interactive permission dialog**: surface `session/request_permission` in the running-log panel (allow once/always, reject) instead of the fixed policy. Needs a request/response channel from the run thread to the UI.
2. **Session reuse for replies**: keep the agent process alive per cue and send follow-up prompts into the same `sessionId`; use `session/load` when `agentCapabilities.loadSession` is set. Store the session id in `executions.session_id` like Claude does.
3. **MCP forwarding**: pass Dirigent's MCP server (`dirigent-mcp`) and configured sources in `session/new.mcpServers`.
4. **Image prompts**: send cue attachments as `image` content blocks when `promptCapabilities.image` is advertised.
5. **Agent presets**: a dropdown of known agents (command + args) next to the free-form fields, possibly fed from the ACP registry.
6. **Client terminals**: implement `terminal/*` so command output streams into the running log with exit codes.
7. **Modes / config options**: expose `session/set_mode` (v1) or the v2 config options for plan vs. code modes.
8. **v2**: re-evaluate the SDK crate once v2 stabilises; the lenient wire types should keep v1 agents working meanwhile.
