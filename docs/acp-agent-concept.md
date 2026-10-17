# Dirigent as an ACP agent: concept (not implemented)

**Status:** concept only. Nothing on this page exists in the code. It records the design and the lessons from the May 2026 prototype (`feature/acp`, now deleted) so the idea can be picked up again. The implemented direction, Dirigent as an ACP *client* that drives other agents, is in [acp-concept.md](acp-concept.md).

## Idea

Turn the roles around: an ACP client such as **Zed** (or JetBrains, Neovim, …) spawns `dirigent --acp` as its agent. Each prompt typed into the editor's agent panel becomes a **cue** in Dirigent. Dirigent runs the cue with the configured provider (Claude, Codex, …, even an ACP agent) and streams progress back to the editor. Meanwhile the Dirigent window shows the kanban board, the running log, diff review and history.

```
Zed agent panel ──stdin──▶ dirigent --acp ──▶ cue (Ready) ──▶ provider run (Claude/Codex/…)
       ▲                        │                                   │
       └──────stdout────────────┴──── session/update, diffs, ◀──────┘
                                      stopReason
                        Dirigent window: cue pool · running log · diff review
```

What the editor user gets over talking to Claude Code directly:

- Persistent cue history in SQLite.
- The kanban flow (Inbox → Ready → Review → Done).
- Dirigent's diff reviewer.
- Post-run agents (format/lint/test).
- Playbook prompts.
- Cost tracking.
- One place to switch providers.

## Protocol surface Dirigent would implement (agent side, v1)

| Message | Direction | Dirigent behaviour |
|---|---|---|
| `initialize` | client → agent | Reply `protocolVersion: 1`, `agentInfo {name:"dirigent", title:"Dirigent", version}`, `agentCapabilities {loadSession:false, promptCapabilities{image:true?, embeddedContext:true}}`, `authMethods: []` |
| `session/new {cwd, mcpServers}` | client → agent | Open or attach the project at `cwd`; generate a `sessionId`; remember `mcpServers` for runs |
| `session/prompt {sessionId, prompt[]}` | client → agent | Create a cue from the text (plus resource links as file context), set it Ready, trigger the run, and reply with `stopReason` when the execution finishes |
| `session/update` | agent → client | `agent_message_chunk` from the run's log stream; `tool_call` / `tool_call_update` per provider tool event; `plan` from cue workflow steps |
| `session/cancel` | client → agent | Cancel the cue's task handle; reply `stopReason: cancelled` |
| `session/request_permission` | agent → client | Optional: ask the editor before applying a diff, instead of auto-applying |
| `fs/read_text_file` / `fs/write_text_file` | agent → client | Optional: read unsaved editor buffers for context; write through the editor so its undo history includes the change |
| `session/load` | client → agent | Only advertise `loadSession: true` once implemented: map `sessionId` to cue thread and replay history as updates |

## Lessons from the prototype

The prototype got Zed to spawn Dirigent and open its window. It never got a working chat, for these reasons, which any new attempt must handle:

1. **stdout is the protocol channel and nothing else may write to it.** On macOS, starting `eframe`/NSApplication interfered with the process's stdout. The prototype `dup()`ed the fd before starting the GUI and wrote JSON-RPC only through the copy. Any `println!`, a library banner or a child process inheriting stdout corrupts the stream. Child provider processes must get their own pipes, as they already do.
2. **Answer the handshake off the UI thread.** `initialize` and `session/new` arrive before the egui update loop runs. Routing them through the app's channel left Zed stuck on "Loading…". They must be answered directly on the stdin reader thread. Prompts can then be forwarded to the app, with a repaint request so egui wakes up.
3. **Request ids are arbitrary JSON values.** Zed sends UUID strings. Parsing ids as `u64` dropped Zed's `initialize` entirely.
4. **Use the exact v1 field names.** These were all wrong at first: `agentCapabilities` (not `capabilities`), `prompt` (not `content`) in `session/prompt`, and updates shaped as `{sessionId, update:{sessionUpdate:"agent_message_chunk", content:{type:"text", text}}}` (not `{kind:{type,text}}`). With the wrong update shape the editor shows nothing even though Dirigent is working. The prototype still had this bug at the end.
5. **Don't advertise what you don't implement.** `loadSession: true` without a real `session/load` makes Zed try to resume and fail on restart.
6. **Diff content needs the real old text.** The prototype sent `oldText: ""`, which renders every edit as a full-file rewrite. Send the pre-run content, or `null` only for new files.
7. **Process lifetime belongs to the editor.** Zed owns the process. Closing the Dirigent window ends the agent ("Server exited with status 0"), and Zed starts one agent process per thread or panel. The GUI must cope with being launched once per editor thread, or hand off to a single running instance.

## Design questions to settle first

| Question | Options |
|---|---|
| GUI or headless? | **(a)** `dirigent --acp` opens the full window (prototype); **(b)** headless agent process that forwards to an already running Dirigent over a local socket (one window, many editor threads); **(c)** headless only, cues land in the DB and show up when Dirigent is opened. (b) avoids the stdout/NSApplication problem and the window-per-thread problem, and is the recommended direction. |
| Cue placement | New cue per prompt (simple) or one cue per session with replies as follow-ups (matches Dirigent's reply workflow) |
| Review gate | Reply `end_turn` when the run finishes (diff already in the working tree), or hold the turn open until the user accepts it in Dirigent's diff review |
| Permissions | Auto-apply (as today for providers), or relay provider permission prompts as `session/request_permission` to the editor |
| Provider recursion | Allow the ACP *client* provider underneath (editor → Dirigent → other ACP agent)? Useful, but guard against `dirigent --acp` calling itself |
| Distribution | Manual Zed `agent_servers` entry first; the ACP registry later. The prototype's Zed extension used `context_servers` (MCP), not agent servers, so it did not help here. |

## Zed configuration (for when it exists)

```json
{
  "agent_servers": {
    "Dirigent": {
      "type": "custom",
      "command": "dirigent",
      "args": ["--acp"],
      "env": {}
    }
  }
}
```

Settings file: `~/.config/zed/settings.json` (Linux and newer macOS builds) or `~/Library/Application Support/Zed/settings.json`. Check Zed's current docs for the exact schema.

## Rough plan

1. Headless `dirigent acp` (option b): a stdin/stdout JSON-RPC loop with lenient types reused from `src/acp/types.rs`, plus agent-side types. Handshake answered immediately.
2. Local IPC to the running app, reusing the `dirigent-mcp` plumbing where possible: `create_cue`, `run_cue`, `cancel`, `subscribe(cue_id)` for log, tool and diff events.
3. Map provider events to `session/update`s; send diffs with real `oldText`.
4. Test with a scripted ACP *client* (the mirror image of the client tests), then Zed.
5. Only then: `session/load`, permission relay, editor fs access.
