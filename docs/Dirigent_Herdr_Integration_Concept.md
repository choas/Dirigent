# Concept: Integrating Dirigent and herdr

> **Status:** Draft concept / RFC
> **Audience:** Dirigent maintainers, herdr users evaluating a GUI direction/review layer
> **TL;DR:** Dirigent and herdr are the two halves of the same workflow. Dirigent is the
> *direction & review* layer (a GUI where a human anchors instructions to code and reviews
> diffs); herdr is the *execution & observability* substrate (a durable, detachable,
> multi-agent terminal multiplexer). Wire them together so Dirigent stops running agents
> inside its own process and instead orchestrates a herdr fleet, while herdr-driven lead
> agents can push work into and read results out of Dirigent's cue pool.

> **Implementation status (October 2026):** a first slice of Direction A has
> shipped as the experimental **Herdr** CLI provider (`src/herdr/`, how-to:
> [How_to_Herdr.md](How_to_Herdr.md)). It runs a *single* cue as native Claude
> Code in a herdr tab, over herdr's socket API (protocol 22, herdr 0.9), and
> completes on Claude's Stop hook. It is a `HerdrBackend` on the existing
> `AiBackend` seam (`src/ai_backend.rs`), not a new execution abstraction, and
> it does not synthesize `claude_pty` events (see §4.2). Directions B and C,
> the Needs-Input lane and fleet execution are still concepts.

---

## 1. The two tools at a glance

| | **Dirigent** | **herdr** |
|---|---|---|
| What it is | Desktop GUI for AI-assisted code review & editing | Terminal multiplexer with agent-state awareness (tmux, rebuilt for agents) |
| Language | Rust (macOS-native, Linux) | Rust (single ~10 MB binary) |
| Core metaphor | *Conductor*: human directs, AI performs | *Herder*: many agents, one control room |
| Unit of work | **Cue** — a file path + line range + natural-language prompt, tracked on a kanban pool (Inbox → Ready → Review → Done → Archived) | **Pane** — a real terminal running one agent, grouped into Tabs and Workspaces |
| How agents run | Spawns Claude Code / OpenCode / Codex / Gemini **inside the GUI process** under a PTY (`claude_pty`), streams typed events, auto-accepts tool confirmations | Persistent **background server** owns every pane; agents survive detach/laptop-close/SSH-reattach |
| State model | `SessionState`: `Starting`, `ReadyForInput`, `Running`, `WaitingForPermission`, `DrainingAfterStop`, `Exiting`, `Exited` | Sidebar: 🔴 blocked · 🟡 working · 🔵 done · 🟢 idle (auto-detected for 15+ agents, zero config) |
| Parallelism | LLM workflow planner groups Inbox cues into dependency-ordered steps; up to `MAX_PARALLEL_CUES = 5` run in parallel — but all inside one process | Unlimited panes across workspaces; a lead agent spins up helpers via the socket API |
| Programmatic surface | **`dirigent-mcp`** MCP server over the SQLite pool (`.Dirigent/Dirigent.db`): `dirigent_list_cues`, `dirigent_add_cue`, `dirigent_move_cue`, `dirigent_update_cue`, `dirigent_pool_summary`, `dirigent_cue_history`, … | **Local Unix socket API + CLI**: create workspaces, split/zoom panes, read pane output, subscribe to state changes; MCP bridges (`herdr-mcp`) and Python/Go/Rust/TS clients |
| Durability | Agents die when the GUI closes | Agents persist; reattach from another terminal or phone |
| Review UX | First-class: inline/side-by-side diffs, accept/reject, PR creation, CodeRabbit import | None — raw terminals only |

**The one-sentence case for integration:** Dirigent already re-implements a slice of what
herdr does well (PTY management, per-agent state detection, parallel scheduling) but is
capped by its single-process, GUI-bound execution model; herdr does durable multi-agent
execution superbly but has no structured way to *direct* work at specific code and *review*
the result. Each covers the other's gap almost exactly.

---

## 2. Why integrate (the complementarity)

**What herdr gives Dirigent**

1. **Durable, detachable execution.** Today a running cue is a child process of the GUI —
   quit Dirigent (or crash it) and in-flight work dies. Running cues *as herdr panes* means
   work survives, and you can close the laptop and reattach later.
2. **True fleet-scale parallelism.** The workflow planner's parallel steps stop being bounded
   by one process's PTY farm. Each parallel cue becomes an independent herdr pane in a
   workspace; the 5-cue soft cap becomes a scheduling choice, not an architectural one.
3. **Remote & mobile monitoring for free.** herdr already offers SSH/phone attach, web UIs,
   and notification bridges. A cue kicked off from the desktop can be watched from a phone.
4. **A mature, agent-agnostic state engine.** herdr detects blocked/working/done/idle for
   15+ agents with zero config. Dirigent could lean on this instead of growing its own
   `SessionState` heuristics per backend (Claude, OpenCode, Codex, Gemini).

**What Dirigent gives herdr**

1. **Structured direction.** Cues anchor an instruction to an exact `file:line` range with
   prompt-quality hints, cue commands (`[plan]`, `[test]`, `[refactor]`…), and image
   attachments — a far richer way to task an agent than typing into a raw pane.
2. **A review & accept loop.** herdr has no diff view. Dirigent's inline/side-by-side diff,
   accept/reject, reply-to-refine, and PR creation become the "what did the fleet actually
   do?" gate that herdr lacks.
3. **A kanban of intent.** The cue pool is a durable backlog of *what should happen*,
   independent of *which pane is running right now* — exactly the persistent work-list a
   herdr fleet needs to pull from.
4. **LLM dependency planning.** Dirigent already computes which cues can run in parallel and
   which must be sequential — a ready-made scheduler for a herdr fleet factory.

---

## 3. Integration directions

Three directions, which compose rather than compete. The recommendation (§4) combines A and B.

### Direction A — Dirigent orchestrates a herdr fleet (execution offload)

Dirigent stops spawning agents in-process and instead treats herdr as its execution backend.
When a cue (or a workflow step) runs, Dirigent asks herdr to create a pane running the chosen
agent CLI, then observes that pane's state and output over the herdr socket.

```
┌────────────────────────────┐         herdr socket API        ┌───────────────────────────┐
│  Dirigent GUI              │  create_workspace / split_pane   │  herdr background server  │
│  • cue pool (kanban)       │ ───────────────────────────────► │  ┌─────┐ ┌─────┐ ┌─────┐  │
│  • workflow planner        │  subscribe(state) / read(pane)   │  │pane │ │pane │ │pane │  │
│  • diff review + accept    │ ◄─────────────────────────────── │  │cue17│ │cue18│ │cue22│  │
│  • SQLite pool             │                                   │  └─────┘ └─────┘ └─────┘  │
└────────────────────────────┘                                   └───────────────────────────┘
        keeps: intent + review                                     owns: durable execution
```

- **Workspace per repo, tab per workflow step, pane per cue.** A parallel step with three
  cues becomes one tab with three panes; the planner's sequencing drives when the next tab
  opens.
- Dirigent maps herdr's pane state back onto its own UI (see §4.1) and drives the cue through
  Ready → Review as the pane goes working → done.
- Dirigent keeps ownership of the *content*: it still builds the prompt (cue text + file/line
  context + `[command]` template + MCP injection), still parses the resulting diff, still
  runs post-run agents (Format/Lint/Build/Test).
- **Payoff:** durability, remote monitoring, unbounded parallelism — with minimal change to
  the user-facing GUI.

### Direction B — herdr agents drive Dirigent's cue pool (the reverse bridge)

A lead agent running *inside* herdr uses the existing `dirigent-mcp` server to read and write
the cue pool. This already works today with zero new code — it just needs to be documented as
a first-class pattern and packaged as a herdr skill.

- A herdr "PM" agent (cf. `herdr-pm`) reads the repo, decides on work, and calls
  `dirigent_add_cue` to file line-anchored cues into Inbox.
- Helper agents it spawns pull cues via `dirigent_list_cues` / `dirigent_move_cue`, execute,
  and record results the human later reviews in the Dirigent GUI.
- Dirigent becomes the **durable, human-reviewable backlog and audit log** for an otherwise
  ephemeral herdr fleet — `dirigent_cue_history` and `dirigent_execution_stats` give the
  fleet a memory across detach/reattach cycles.
- **Payoff:** herdr users get structured intent + a review surface; ships as a
  `npx skills add …` skill + an MCP config snippet, no Dirigent code change required.

### Direction C — Dirigent as a herdr "control-room" GUI (deeper convergence)

The most ambitious option: Dirigent grows a **Fleet view** that renders live herdr state
(workspaces/tabs/panes with color-coded status) alongside the cue pool, and lets the human
click a blocked pane to answer a permission prompt or attach to its terminal. This positions
Dirigent as the graphical front-end to herdr's headless server — the GUI control room herdr
deliberately doesn't ship. This is a superset of Direction A and is best treated as the
long-term north star, not the MVP.

---

## 4. Recommended architecture (A + B)

Adopt **Direction A** as Dirigent's execution substrate and **Direction B** as the reverse
bridge, sharing one integration layer. Concretely:

### 4.1 State mapping

Dirigent's `SessionState` and herdr's four-state model line up almost one-to-one. A thin
adapter translates in both directions so the GUI, the cue pool, and the herdr sidebar always
agree:

| Dirigent `SessionState` | herdr state | Cue pool effect |
|---|---|---|
| `Starting`, `PromptSubmitted` | 🟡 working | cue enters **Ready/Running** |
| `Running` | 🟡 working | — |
| `WaitingForPermission` | 🔴 blocked | surface permission dialog; badge cue as needing input |
| `DrainingAfterStop` | 🔵 done (draining) | prepare diff |
| `Exited` (success, diff ready) | 🔵 done | move cue to **Review** |
| `ReadyForInput` (idle) | 🟢 idle | pane reusable for a reply/refine |

Because both systems already compute this state, the adapter is data-mapping, not new
heuristics. Where herdr's detector is more mature (non-Claude backends), Dirigent can prefer
herdr's signal.

### 4.2 A `herdr` execution backend in Dirigent

Introduce an execution-backend abstraction alongside the current in-process PTY path:

- **`InProcessBackend`** (today): `claude_pty` child under the GUI. Default; no herdr needed.
- **`HerdrBackend`** (new): talks to the herdr socket. `run_cue(cue)` →
  `create/split pane` running the configured agent CLI in the repo's worktree; then
  `subscribe(state)` + `read(pane)` feed the same event stream the rest of Dirigent already
  consumes.

> *As implemented:* the backend seam already existed as `AiBackend`
> (Claude, Gemini, OpenCode, Codex, ACP, now Herdr), so `HerdrBackend` only
> implements `invoke_cli`. Instead of streaming synthesized events it waits
> for the Stop hook while watching the pane, and rebuilds the response from
> Claude's session transcript. Streaming `claude_pty::Event`s from herdr
> remains the route to a live running log.

Key design point: **keep the event contract stable.** Dirigent's UI, diff parser, and
post-run agents consume `claude_pty::Event`s. The `HerdrBackend` should emit the *same* event
enum, synthesized from herdr socket messages, so nothing downstream changes. This makes herdr
an opt-in setting (`Settings → Execution backend: In-process | herdr`), not a rewrite.

### 4.3 Prompt & MCP wiring carries over unchanged

The existing auto-injection (`--mcp-config` when a prompt mentions "Dirigent") works
identically whether the agent runs in-process or in a herdr pane — the pane just launches
`claude --mcp-config .Dirigent/mcp-config.json …`. Direction B is literally the same
`dirigent-mcp` server, now reachable by *any* agent herdr is running.

### 4.4 Worktree isolation for parallel cues

Parallel cues that touch overlapping files need isolation. Dirigent already manages git
worktrees; pair that with herdr's `herdr-worktree-lifecycle` pattern so each parallel pane
runs in its own worktree, and Dirigent merges/reviews the results serially in the diff view.

---

## 5. Phased roadmap

| Phase | Scope | Effort | New code |
|---|---|---|---|
| **0 — Document B** | Publish a "Drive Dirigent from herdr" guide + ship a herdr skill that wires `dirigent-mcp` into a herdr agent. Validate the round-trip: herdr agent files a cue, human reviews it in the GUI. | S | Docs + skill manifest only |
| **1 — Needs-Input lane + read-only Fleet panel** | Add the "Needs Input" lane (§7) driven by existing `WaitingForPermission` / `PermissionDialogDetected` events — useful with the in-process backend alone. Optionally connect to a running herdr socket to show live pane state next to the cue pool (Direction C, read-only). No execution change. | M | Needs-Input view; herdr socket client (read/subscribe) |
| **2 — `HerdrBackend` MVP** | Run a *single* cue as a herdr pane behind the backend abstraction; map state; parse the resulting diff. Feature-flagged. | M–L | Backend trait + herdr client (write) + event adapter |
| **3 — Fleet execution** | Wire the workflow planner to fan parallel steps out as panes/tabs with worktree isolation; drive cues through Review as panes complete. | L | Scheduler ↔ herdr mapping, worktree lifecycle |
| **4 — Control room** | Interactive Fleet view: answer permission prompts, attach to a pane's terminal, reassign work — Dirigent as herdr's GUI. | L | Interactive socket ops, embedded terminal view |

Phases 0–1 deliver value with essentially no risk to the existing app; 2+ are gated behind a
setting so the in-process backend stays the default.

---

## 6. Open questions & risks

- **herdr socket API stability & schema.** The concept assumes documented, versioned socket
  commands (`create_workspace`, `split_pane`, `read_pane`, `subscribe`). The exact JSON
  schema must be pinned before Phase 2; if unstable, prefer the `herdr-mcp` bridge over the
  raw socket. *(Done for the MVP: the socket protocol is newline-delimited JSON with a
  published schema, `herdr api schema --json`; the backend uses `tab.create`,
  `pane.send_input`, `agent.prompt`, `pane.get`, `pane.process_info`, `pane.read`,
  `pane.send_keys` and `pane.close`.)*
- **Platform reach.** Dirigent is macOS-first (native About panel, notifications, `.app`
  bundle); herdr is cross-platform Rust. The herdr backend could actually *broaden* Dirigent's
  reach on Linux, but the GUI-native bits stay macOS-only.
- **Permission handling.** In-process, Dirigent auto-accepts tool confirmations. Through
  herdr, a 🔴 blocked pane needs its prompt surfaced back to the GUI (or answered from a
  phone). The `WaitingForPermission ↔ blocked` mapping (§4.1) is the seam to get right.
- **Double state detection.** Running Claude inside herdr means two systems parse the same
  TUI. Decide per-backend which is authoritative (proposal: herdr owns coarse
  blocked/working/done/idle; Dirigent's `claude_pty` still owns fine-grained events like
  permission-dialog contents and the Stop-hook diff summary).
- **Dependency footprint.** herdr becomes an optional external binary (like the existing
  `merman-cli` / `dirigent-mcp` optionals), discovered on `PATH` and configured in Settings —
  never a hard requirement.
- **Overlap ownership.** Long-term, Dirigent could retire parts of its own PTY/state code in
  favor of herdr — but only after the backend has proven parity across all four agent CLIs.

---

## 7. A "Needs Input" lane — a herdr status view that fits Dirigent's model

A fair objection: *Dirigent already has Inbox / Ready / Review / Done and a notion of
"running" — do we even need herdr's live status sidebar?* The answer is that the kanban
columns and herdr's sidebar track **two different axes**, and conflating them is exactly why
the gap feels awkward:

- **Kanban columns = lifecycle *intent*.** Where a cue *is in its journey*: not-yet-started,
  queued, awaiting-human-review, finished. This is durable and slow-moving.
- **herdr's sidebar = runtime *status* of live instances.** What each running agent *is doing
  right now*: working, done, idle, or **blocked waiting for an answer**. This is transient and
  fast-moving.

A cue sitting in "Running" tells you nothing about whether its agent is churning happily or
has been stuck for ten minutes on a permission prompt. That runtime axis is precisely what
herdr surfaces and what Dirigent's columns cannot express.

**The proposal:** rather than bolt a full herdr-style sidebar onto the GUI, add the *one*
runtime state that is genuinely actionable to a human — **"waiting for an answer"** — as its
own first-class surface. Concretely a **"Needs Input" lane** (a horizontal band above the
board, or a filtered virtual column) that automatically collects any running cue whose agent
is blocked:

```
┌──────────────────────────────────────────────────────────────────────┐
│ ⚠ NEEDS INPUT (2)   cue #18 "add retry" · Bash rm -rf?   cue #22 · plan?│  ← virtual lane, auto-populated
├──────────┬──────────┬──────────┬──────────┬──────────┬────────────────┤
│ Backlog  │ Inbox    │ Ready    │ Review   │ Done     │ Archived       │  ← existing kanban (intent)
└──────────┴──────────┴──────────┴──────────┴──────────┴────────────────┘
```

Why this is the right shape:

- **It maps to state we already compute.** A cue lands in the lane exactly when its agent hits
  `SessionState::WaitingForPermission` (herdr's 🔴 blocked) — see the §4.1 table. It leaves the
  moment the human answers and the agent returns to `Running`. No new detection logic.
- **It's the highest-value slice of herdr's view.** working/idle/done are already implied by
  the column a cue lives in; *blocked-on-a-question* is the only runtime state that demands a
  human *right now* and currently has nowhere to show up.
- **It's a virtual lane, not a real column.** The cue doesn't *move* to a new kanban stage
  (it's still logically Running/Ready) — the lane is a live filter over runtime state. When
  answered, it silently drops back. This keeps the intent axis and the runtime axis from
  fighting each other.
- **It's the natural home for the permission seam.** §6 flags that a blocked herdr pane must
  surface its prompt back to the GUI. That prompt *is* the Needs-Input lane's content: the cue
  card expands to show the permission dialog / `AskUserQuestion` body with the answer buttons,
  whether the agent runs in-process or in a herdr pane. Answering here also satisfies the
  "answer from your phone" story once the herdr backend lands.
- **It degrades gracefully.** With the in-process backend it's driven by `claude_pty`'s
  `PermissionDialogDetected` events — useful today, with zero herdr dependency. With the herdr
  backend it additionally aggregates blocked panes across the whole fleet, including non-Claude
  agents herdr detects.

A light **status glyph on every running cue card** (🟡/🔴/🔵/🟢) can accompany the lane so the
board doubles as a coarse fleet view — but the Needs-Input lane is the part that earns its
keep first. It's a good candidate to ship in **Phase 1** (read-only fleet panel), since it's
mostly a new view over state Dirigent already has.

---

## 8. Recommendation

Start with **Direction B, Phase 0** — it's almost free (the `dirigent-mcp` server already
exists), proves the two tools genuinely compose, and immediately gives herdr users a
structured-direction + review surface. In parallel, prototype the **read-only Fleet panel
(Phase 1)** to de-risk the herdr socket client. Commit to the **`HerdrBackend` (Phases 2–3)**
once the socket schema is confirmed stable — that's where Dirigent sheds its single-process
ceiling and becomes the conductor of a durable, observable, phone-reachable agent fleet, while
staying the place a human directs the work and signs off on the diff.
