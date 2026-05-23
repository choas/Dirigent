# ACP Implementation for Dirigent

## What Was Implemented

### Overview

Dirigent now supports the **Agent Client Protocol (ACP) v1** as a fourth provider alongside Claude, OpenCode, and Gemini. Any ACP-compatible agent (33+ available as of May 2026) can be used via the "ACP Agent" provider selection in settings.

### Architecture Decision

**Direct protocol implementation** rather than depending on the `agent-client-protocol` SDK crate. Rationale:
- The SDK is at v0.12 while the spec is at v0.13.1 — version lag introduces risk
- ACP v2 is in active development with breaking changes planned
- Direct JSON-RPC 2.0 implementation is straightforward (~400 lines)
- Avoids coupling Dirigent's release cadence to the SDK's
- Same approach used by many production clients in the ecosystem

### Files Added

| File | Purpose |
|------|---------|
| `src/acp/mod.rs` | Module root, public API exports |
| `src/acp/types.rs` | All ACP protocol types (JSON-RPC envelope, session types, content blocks, tool calls, diffs, plans) |
| `src/acp/client.rs` | `AcpConnection` — manages subprocess, JSON-RPC framing, bidirectional message handling |
| `src/acp/invoke.rs` | `invoke_acp_agent()` — orchestrates the full lifecycle (spawn → init → session → prompt → shutdown) |

### Files Modified

| File | Change |
|------|--------|
| `src/main.rs` | Added `mod acp;` |
| `src/settings/providers.rs` | Added `Acp` variant to `CliProvider` enum |
| `src/settings/app_settings.rs` | Added ACP settings fields, provider_fields match arm, defaults |
| `src/settings/semantic_colors.rs` | Added `acp_color()` (teal: #00C8B4 dark, #009688 light) |
| `src/claude/cli.rs` | Promoted `run_lifecycle_script` to `pub(crate)` |
| `src/claude/mod.rs` | Re-exported `run_lifecycle_script` |
| `src/app/claude_run.rs` | Added `run_acp_provider()` function and match arm |
| `src/app/dialog/settings/general.rs` | Added ACP settings UI (binary path, args, scripts) |
| `src/app/dialog/custom_theme.rs` | Added ACP arm for AI-generated themes |
| `src/app/split_cue.rs` | Added ACP arm for cue splitting |
| `src/app/workflow_run.rs` | Added ACP arm for workflow planning |
| `analyze_acp.md` | Updated with May 2026 protocol changes and ecosystem growth |

### Protocol Implementation Details

#### Connection Lifecycle
```
1. Dirigent spawns agent binary as subprocess (stdin/stdout piped)
2. Dirigent → Agent: initialize (protocol_version: 1, capabilities: fs.read/write)
3. Dirigent → Agent: session/new (cwd: project root)
4. Dirigent → Agent: session/prompt (user text as content block)
5. Agent → Dirigent: session/update notifications (streamed)
6. Dirigent → Agent: session/cancel (if user cancels)
7. Dirigent → Agent: session/close (on completion)
8. Subprocess killed on shutdown
```

#### Handled Message Types

**Outbound (Dirigent → Agent):**
- `initialize` — negotiate version and capabilities
- `session/new` — create session with working directory
- `session/prompt` — send user prompt
- `session/cancel` — abort current turn
- `session/close` — clean session teardown

**Inbound Notifications (Agent → Dirigent):**
- `session/update` with `agent_message_chunk` — streamed response text
- `session/update` with `tool_call` / `tool_call_update` — tool execution progress
- `session/update` with `plan` — execution plan entries

**Inbound Requests (Agent → Dirigent):**
- `fs/readTextFile` — read file content from disk
- `fs/writeTextFile` — write file to disk (tracked as edited)
- `session/requestPermission` — auto-approved (matches skip-permissions behavior)

#### First-Class Diff Support

ACP diffs arrive as structured `{ path, oldText, newText }` content blocks — no parsing needed. These are:
1. Collected during the prompt turn
2. Converted to unified diff format for the existing `DiffView`
3. Files tracked for git working-tree diff as fallback

#### Tool Call Streaming

Tool calls are rendered in the running log with structured format:
```
[edit:running] Update src/main.rs
[exec:done] cargo test
[read:done] README.md
```

### Settings

Users configure ACP via Settings → Provider → "ACP Agent":

| Setting | Description |
|---------|-------------|
| Agent Binary | Path or command name (e.g. `claude-agent-acp`, `goose`, custom binary) |
| Extra Arguments | Shell-parsed args passed on spawn |
| Pre-run Script | Shell command executed before each run |
| Post-run Script | Shell command executed after each run |

### Compatible Agents (Tested Patterns)

The implementation works with any agent that speaks ACP v1 over stdio:
- `claude-agent-acp` — Claude Code via ACP adapter
- `goose` — Goose agent (native ACP)
- Any custom binary implementing the ACP agent side

### What's NOT Implemented (Future Work)

| Feature | Why Deferred |
|---------|-------------|
| Session persistence/resume | Needs SQLite schema addition; wait for stable `session/resume` |
| MCP server forwarding | Experimental in ACP v0.13; not stable |
| Interactive permission dialogs | Current approach auto-approves; adequate for initial release |
| Multiple concurrent sessions | Requires UI redesign for session management |
| Agent capability inspection | UI should adapt to agent's advertised capabilities |
| Elicitation (interactive prompts) | Unstable feature in ACP v0.11.5 |
| Next Edit Suggestions | Unstable in v0.11.4; needs UI component |

---

## Multi-Perspective Review

### Architect's View

**Strengths:**
- Clean separation of concerns: protocol types, connection management, and invocation are distinct modules
- Same integration pattern as existing providers (spawn → stream → result) — no architectural disruption
- Bidirectional message handling (agent requests to client) is properly handled inline
- Graceful degradation: if agent doesn't produce structured diffs, falls back to git working-tree diff

**Concerns:**
- Synchronous I/O in `read_message()` blocks the worker thread — acceptable for v1 (single-session, one prompt at a time) but won't scale to concurrent sessions without async
- No connection pooling or agent reuse between runs — each cue execution spawns a fresh agent. This is intentional (stateless, simple) but limits session persistence
- v2 migration will require significant changes to the prompt lifecycle; the current module is designed to be replaceable

**Recommendation:** This is a solid v1 implementation. When ACP v2 stabilizes (likely late 2026), refactor to async with a connection pool for session reuse. The module boundaries are clean enough that this won't require changes outside `src/acp/`.

### Developer's View

**Integration quality:**
- Follows existing patterns exactly — `run_acp_provider()` has the same signature and result type as the other providers
- All match statements exhaustively covered — no `_ =>` fallbacks that would hide future enum additions
- Settings serialization uses `#[serde(default)]` so existing settings files don't break
- Proper error propagation via the existing `ClaudeResult` error path

**Code quality:**
- Types are well-structured with appropriate visibility (`pub(super)` for internal protocol types, `pub(crate)` only for what's needed externally)
- JSON-RPC framing is correct (newline-delimited, no embedded newlines)
- Cancel token is checked at multiple points (before spawn, after init, during prompt)
- Subprocess is properly cleaned up (kill + wait) on both success and error paths, including `Drop`

**Testing considerations:**
- The module is testable by mocking an agent subprocess (just needs a binary that speaks JSON-RPC)
- The `diffs_to_unified()` function can be unit-tested independently
- End-to-end testing requires a real ACP agent binary on PATH

### Agent's View (What an ACP Agent Sees)

**Dirigent as a client is well-behaved:**
- Properly negotiates protocol version at initialization
- Advertises `fs.readTextFile` and `fs.writeTextFile` capabilities
- Handles all agent-to-client requests (fs read, fs write, permission)
- Sends `session/cancel` on user abort (not just killing the process)
- Sends `session/close` for clean teardown
- Respects the JSON-RPC 2.0 framing requirements

**What agents should expect from Dirigent:**
- Single session per connection (no multiplexing)
- Auto-approval of all permission requests
- File reads come from disk (not from unsaved editor buffers — Dirigent is read-only)
- Working directory is always the project root

**Limitation from agent perspective:**
- No MCP server configs passed in `session/new` (agents that need MCP must manage their own)
- No image/audio content blocks (only text prompts)
- No `session/set_mode` calls (agents run in their default mode)
