# PR 37 Dirigent Inbox Cue Triage

Reviewed the current Dirigent Inbox on 2026-05-28. The Inbox contains six
`PR37` cues: `2`, `5`, `7`, `8`, `9`, and `13`. Cue `11` is tagged `PR37` but
is currently in `Ready`, so it is not included here.

## Useful Cues

| Cue | Source ref | Topic | Assessment |
| --- | --- | --- | --- |
| 5 | `pr37:comment:3316935584` | Parse Codex JSON text from `msg.content` | Useful. `extract_text_from_event` still only checks top-level `text`, top-level `message`, `item.text`, and `part.text`. It does not read Codex JSON events shaped as `msg.content`, and turn counting still looks for `turn.completed` rather than `turn_complete`. This can leave successful Codex runs with an empty saved response. |
| 13 | `pr37:review:4379998697_0` | Release the child mutex before blocking I/O and wait | Useful. `invoke_codex_streaming` still holds the child mutex while processing stdout, joining the stderr reader, and waiting for exit. The watchdog also needs that mutex to call `kill()`, so cancellation can be blocked until the process exits on its own. |

## Partially Useful Or Better Deferred

| Cue | Source ref | Topic | Assessment |
| --- | --- | --- | --- |
| 7 | `pr37:comment:3316983734` | Trust-gate Codex hook scripts | The security concern is real: settings are loaded from project-local `.Dirigent/settings.json`, and `run_hook_script` executes configured strings through `sh -c`. However, the same pattern exists for Gemini and OpenCode, so this should be handled as a cross-provider hardening task rather than a narrow Codex-only PR37 fix. |
| 9 | `pr37:comment:3316983761` | Preserve optional metrics | Partially useful. `cost_usd` and `num_turns` should probably remain `None` when Codex emits no corresponding metric, instead of being stored as `Some(0)`. The duration part is less convincing: the app already measures wall-clock duration for other providers, and that is useful even when the CLI does not emit a duration metric. |

## Cues To Skip

| Cue | Source ref | Why skip |
| --- | --- | --- |
| 2 | `pr37:comment:3316442878` | Already addressed in the current code. `CodexRunConfig` has `skip_permissions`, `run_codex_provider` passes `req.config.skip_permissions`, and `invoke_codex_streaming` only appends `--yolo` when that flag is true. |
| 8 | `pr37:comment:3316983754` | Already addressed as written. `invoke_codex_streaming` now starts a background stderr reader before processing stdout, so the original "stderr is only read after stdout" deadlock is no longer present. The remaining cancellation/mutex problem is covered more precisely by cue `13`. |

## Suggested Fix Set

1. Fix Codex JSON event parsing for `msg.content` and `turn_complete`.
2. Rework Codex child ownership so stdout/stderr processing and `wait()` happen without holding the child mutex needed by the watchdog.
3. Preserve optional Codex cost and turn metrics; keep measured wall-clock duration unless the product explicitly wants duration only from CLI-emitted events.
4. Track hook-script trust gating separately as a cross-provider security hardening task.
