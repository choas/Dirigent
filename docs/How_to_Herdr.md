# How to run cues in herdr

[herdr](https://herdr.dev) is a terminal multiplexer built for coding agents. With the **Herdr** provider, Dirigent runs **native, interactive Claude Code in a herdr tab**. You can watch it work and type into it, as in any terminal. Dirigent hands over the prompt and collects the result: the diff, the transcript and the metrics. The cue then lands in Review as usual.

Status: **experimental**. Verified with herdr 0.9.0 and Claude Code 2.1.302. The wider direction (a herdr fleet, a Needs-Input lane) is in [Dirigent_Herdr_Integration_Concept.md](Dirigent_Herdr_Integration_Concept.md).

## When to use it

- **Claude provider (PTY)**: the default. Dirigent owns the Claude process and shows its output live in the running log.
- **Herdr**: use this when you want to see and steer the real Claude Code TUI while it works, for example to answer a question it asks or to nudge it. The run lives in herdr's server, not in Dirigent's process.

## Setup

1. Install herdr and start it (`herdr`), so its server is running.
2. Open **Settings → General** and set **CLI Provider** to **Herdr**. Alternatively, keep your provider and use the **H** button next to the prompt field to send a single prompt to herdr.
3. Check the fields:

| Field | Meaning |
|---|---|
| Herdr CLI Path | The `herdr` binary. It is auto-detected from `PATH`. A path that does not resolve is an error, not a silent fallback to `PATH` |
| Claude CLI Path, Model, Extra Arguments, Pre-/Post-run Script | Shared with the Claude provider |
| Yolo | Starts Claude with `--dangerously-skip-permissions` (shared setting) |

4. Click **Test Connection**. It checks that herdr resolves, that its server runs and answers on its socket, and that Claude replies.

## What a run does

1. Installs Dirigent's Stop hook in `.claude/settings.local.json`, scoped to this run's session id. If the hook cannot be installed (an unwritable directory, or a file that is not valid JSON, which is never overwritten), the run fails right away.
2. Opens a tab `Dirigent #<cue id>` in the project's herdr workspace (the workspace labelled like the project folder, e.g. `Dirigent`; created if missing) and starts `claude` there with your model, args and a generated `--session-id` (`--resume` for replies).
3. Waits until herdr reports Claude as idle, then submits the prompt. If Claude shows a dialog first, such as the folder-trust question in a new directory, the running log says so. Answer the dialog in the herdr tab and the run continues.
4. Waits for the Stop hook. Meanwhile it watches the pane: if the pane is closed, or Claude exits (`/exit`, a crash, Ctrl-C) before the hook fires, the run fails with that reason.
5. Reads the result from Claude's session transcript. It looks first at the hook's `transcript_path`, then scans `~/.claude/projects/*/<session-id>.jsonl`, then reads the pane's text, and last uses the hook's `last_assistant_message`. Turns and token counts come from the transcript; cost is unknown. The diff comes from the working tree, as for every provider.

The tab stays open after the run, so you can scroll back or keep talking to Claude there.

**Cancel** stops Claude in the pane, not just Dirigent's wait. It sends Esc and Ctrl-C twice, and closes the pane if Claude is still running afterwards. The running log says which of these happened.

Analysis prompts (cue splitting, workflow planning, theme generation, change-set analysis) run headless Claude instead, because they need captured output and no UI.

## Environment and secrets

The pane's shell is started by the herdr server, so by default it inherits **herdr's** environment, not Dirigent's. On top of that, Dirigent passes:

- the Claude provider's `claude_env_vars` (variable names, resolved from Dirigent's environment), then
- `.Dirigent/.env`, which wins on duplicate keys.

These values go to herdr in the `tab.create` socket request (herdr's per-launch `env` map). They never appear on a command line, in `ps`, in the pane's shell history or in its scrollback. The prompt is sent over the socket too.

Limitations:

- The values live in the environment of the pane's shell and Claude process, as they do for the in-process providers.
- The pane's shell runs its startup files (`.zshrc`, …), which can override a forwarded variable.
- herdr's own variables (`HERDR_*`) take precedence over forwarded ones.

## Limitations

- Needs a running herdr server and a Unix socket (macOS, Linux).
- No live streaming into Dirigent's running log: watch the herdr tab instead.
- Cost per run is unknown (Claude Code's TUI does not report it).
- A reply resumes the session in a **new** tab. The previous tab, still open and idle, then shares that session, so close it if you keep working through Dirigent.
- herdr's agent detection drives readiness. If herdr never detects Claude, the prompt is pasted only when a program other than the shell is in the pane's foreground; otherwise the run fails rather than typing the prompt into a shell.

## Testing

- Unit tests cover command building, transcript parsing, session args, metrics, and the cancel and pane-exit decisions against a scripted herdr.
- End-to-end against your running herdr (costs a few tokens, and opens a tab where you answer the folder-trust question):

  ```sh
  cargo test herdr_smoke -- --ignored --nocapture
  ```
