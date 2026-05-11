# Pi Integration Concept

Dirigent supports [Claude Code](https://claude.ai/) and [OpenCode](https://github.com/nicepkg/opencode) as CLI backends. This document proposes adding [Pi](https://pi.dev) as a third provider. Pi is a minimal terminal coding agent harness that supports 15+ LLM providers through a single CLI, giving Dirigent access to Anthropic, OpenAI, Google, Azure, Bedrock, Mistral, Groq, Cerebras, xAI, Hugging Face, and more.

## Why Pi

| Aspect | Claude Code | OpenCode | Pi |
|---|---|---|---|
| Model providers | Anthropic only | OpenAI, Anthropic | 15+ (Anthropic, OpenAI, Google, Azure, Bedrock, Mistral, Groq, xAI, ...) |
| Output format | `--output-format stream-json` | `--format json` | `--mode json` (JSONL event stream) |
| Non-interactive mode | `-p <prompt>` | `run "<prompt>"` | `-p <prompt>` (print mode) |
| Permission system | `--dangerously-skip-permissions` | Yolo mode / ruleset | TBD — needs investigation |
| Extensions | N/A | N/A | Extensions, skills, prompt templates, themes |
| Context files | CLAUDE.md | .opencode config | AGENTS.md, SYSTEM.md, skills |
| File inclusion | N/A (tool reads files) | N/A | `@file` syntax in prompts |

Adding Pi would let users choose their preferred model provider without Dirigent needing direct API integrations for each one. Pi acts as a provider-agnostic harness.

## CLI Invocation

### Installation

```sh
# npm
npm install -g @anthropic-ai/pi

# curl
curl -fsSL https://pi.dev/install | sh

# bun
bun install -g @anthropic-ai/pi
```

### Invocation Pattern

Dirigent would invoke Pi in **print mode with JSON events**:

```sh
pi -p "<prompt>" --mode json --provider <provider> --model <model> [extra_args]
```

Flags:
- `-p "<prompt>"` — non-interactive print mode (send prompt, get response, exit)
- `--mode json` — emit all events as JSON lines on stdout (instead of TUI)
- `--provider <provider>` — e.g. `anthropic`, `openai`, `google`, `bedrock`
- `--model <model>` — model identifier, supports `provider/id` syntax (e.g. `anthropic/claude-sonnet-4-6`)
- `--thinking <level>` — thinking budget: `off`, `minimal`, `low`, `medium`, `high`, `xhigh`
- `--no-context-files` — skip loading AGENTS.md / SYSTEM.md (Dirigent builds its own prompt)
- `--tools <list>` / `--no-tools` — control which tools Pi can use
- `@file` args — include file contents in the prompt context

### File Context via @ Syntax

Pi's `@file` syntax could replace or complement Dirigent's current approach of inlining file content in the prompt:

```sh
pi -p "Fix the bug described in this cue" --mode json @src/main.rs @src/lib.rs
```

This is more efficient than embedding file content in the prompt string, as Pi handles the context window management.

## Stream Parsing

### Event Format

Pi with `--mode json` emits newline-delimited JSON events on stdout. The event types need to be mapped to Dirigent's existing `StreamState`:

```rust
StreamState {
    final_result: String,      // accumulated response text
    edited_files: Vec<String>, // files modified by tool_use
    metrics: RunMetrics,       // cost, duration, tokens, turns
}
```

### Expected Event Mapping

Based on Pi's documentation, the event types are likely similar to Claude Code's stream-json format since Pi wraps the same underlying APIs. The implementation should handle:

| Pi Event | Dirigent Mapping | Notes |
|---|---|---|
| Text content | `final_result` accumulation | Streamed text blocks |
| Tool use (Edit/Write) | `edited_files` tracking | Track file-modifying tools |
| Completion/result | `metrics` extraction | Cost, tokens, duration |
| Rate limit | Log retry delay | Same as existing handling |
| Error | `CliError` propagation | Surface in UI |

**Open question:** Pi's exact JSON event schema for `--mode json` is not fully documented yet. Before implementation, we need to:
1. Run `pi -p "hello" --mode json` and capture the actual JSONL output
2. Compare event shapes with Claude Code's `stream-json` and OpenCode's `--format json`
3. Determine if Pi emits tool_use events with file paths (for `edited_files` tracking)
4. Check if Pi emits cost/token metrics in a result event

### Diff Extraction

Same strategy as the existing providers:
1. **Primary:** Track `edited_files` from tool_use events, then run `git::get_working_diff()` on those paths
2. **Fallback:** Parse fenced `` ```diff `` blocks from the response text
3. **Last resort:** Scoped working diff (files changed during the run)

## Settings & Configuration

### New Settings Fields

Add Pi-specific fields to `Settings`, following the existing pattern:

```rust
// In settings/app_settings.rs
pub pi_provider: String,       // e.g. "anthropic", "openai", "google"
pub pi_model: String,          // e.g. "claude-sonnet-4-6", "gpt-4.1"
pub pi_cli_path: String,       // path to `pi` binary (empty = auto-detect)
pub pi_extra_args: String,     // additional CLI flags
pub pi_env_vars: String,       // env var names to forward
pub pi_pre_run_script: String,
pub pi_post_run_script: String,
pub pi_thinking: String,       // thinking level: off/minimal/low/medium/high/xhigh
```

### Provider Enum Extension

```rust
// In settings/providers.rs
pub(crate) enum CliProvider {
    Claude,
    OpenCode,
    Pi,  // new
}
```

### ProviderFields Extension

```rust
// In settings/app_settings.rs — provider_fields()
CliProvider::Pi => ProviderFields {
    model: &self.pi_model,
    cli_path: &self.pi_cli_path,
    extra_args: &self.pi_extra_args,
    env_vars: &self.pi_env_vars,
    pre_run_script: &self.pi_pre_run_script,
    post_run_script: &self.pi_post_run_script,
},
```

### Authentication

Pi authenticates via API keys set as environment variables:
- `ANTHROPIC_API_KEY` — for Anthropic models
- `OPENAI_API_KEY` — for OpenAI models
- `GOOGLE_API_KEY` — for Google models
- Other provider-specific keys

These can be configured via:
1. `.Dirigent/.env` file (existing mechanism, automatically injected)
2. Pi env vars setting (forward specific env var names)
3. Pi's own `/login` mechanism (subscription-based, stored by Pi itself)

### Settings UI

The Settings dialog should show when Pi is selected:
- **Provider** dropdown (anthropic, openai, google, ...) — unique to Pi
- **Model** dropdown — populated dynamically or from a default list per provider
- **Thinking Level** dropdown — off / minimal / low / medium / high / xhigh
- **CLI Path**, **Extra Args**, **Env Vars**, **Pre/Post Run Scripts** — same pattern as Claude/OpenCode

## Implementation Plan

### Phase 1: Basic Integration

1. **Add `Pi` variant to `CliProvider`** (`settings/providers.rs`)
2. **Add Pi settings fields** (`settings/app_settings.rs`)
3. **Build Pi command** — new function in `claude/cli.rs` or a new `pi/` module:
   ```rust
   fn build_pi_command(
       pi_bin: &str,
       prompt: &str,
       provider: &str,
       model: &str,
       thinking: &str,
       extra_args: &str,
       env_vars: &str,
       skip_context_files: bool,
   ) -> Command
   ```
4. **Parse Pi's JSON event stream** — extend `stream.rs` or add a Pi-specific parser
5. **Wire into `invoke_claude_streaming`** — dispatch to Pi command builder based on provider
6. **Settings UI** — add Pi section to the Settings dialog

### Phase 2: Pi-Specific Features

7. **`@file` context** — when auto-context is enabled, pass cue-adjacent files via `@path` instead of inlining in the prompt
8. **Provider-aware model list** — fetch or maintain per-provider model lists for the dropdown
9. **Thinking level UI** — expose Pi's thinking budget control in the toolbar or settings
10. **Extension loading** — allow users to specify Pi extensions via `--extension` in extra args

### Phase 3: Advanced

11. **RPC mode** — instead of one-shot `-p` invocation, keep a persistent Pi process via `--mode rpc` for faster turnaround (skip startup overhead)
12. **Multi-model routing** — use Pi's `--models` flag to let the model switch providers mid-session
13. **Session continuity** — leverage Pi's `--continue` / `--session` flags to maintain context across cues

## Architecture Decision: Module Structure

Two options:

### Option A: Extend `claude/` module

Add Pi-specific command building and stream parsing alongside Claude and OpenCode. Minimal code duplication since the streaming and diff extraction logic is shared.

**Pro:** Less code, reuses existing infrastructure.
**Con:** The `claude/` module name becomes misleading with three providers.

### Option B: New `cli/` module hierarchy

Rename `claude/` to `cli/` and create provider-specific submodules:

```text
src/cli/
  mod.rs          // shared types, invoke function
  claude.rs       // Claude command builder + stream quirks
  opencode.rs     // OpenCode command builder + stream quirks
  pi.rs           // Pi command builder + stream quirks
  stream.rs       // shared stream parsing
  diff_parser.rs  // shared diff extraction
  types.rs        // ClaudeResponse -> CliResponse, ClaudeError -> CliError, etc.
```

**Pro:** Clean separation, provider name in module path.
**Con:** Larger refactor, rename `ClaudeResponse` -> `CliResponse`, `ClaudeError` -> `CliError`, etc.

**Recommendation:** Option A for Phase 1 (ship fast), Option B as a follow-up refactor if a fourth provider is ever added.

## Permission Handling

Pi's permission model is not yet fully documented. Key questions:

1. Does Pi prompt for file write/read permissions in `-p` mode?
2. Is there a `--dangerously-skip-permissions` equivalent or a Yolo config?
3. How does Pi handle tool permissions when running non-interactively?

If Pi blocks on permission prompts in `-p` mode, the Dirigent watchdog thread will detect a stall. Mitigations:
- Pass `--no-tools` and rely on Pi generating diffs in the response text
- Configure Pi's permission system via a project-level config file
- Use `--tools Edit,Write,Read,Bash,Glob,Grep` to allowlist only needed tools

## Risks and Open Questions

| Risk | Mitigation |
|---|---|
| Pi's JSON event schema is not fully documented | Capture real output before implementing parser |
| Pi may not emit tool_use events with file paths | Fall back to response-text diff parsing |
| Pi may not report cost/token metrics | Use duration-only metrics, or query Pi's session history |
| Permission prompts in non-interactive mode | Investigate Pi's config, add to extra_args if needed |
| Pi is newer/less mature than Claude Code | Start as opt-in third provider, don't change defaults |
| `@file` syntax changes prompt construction | Keep it behind `auto_context_file` setting |

## Summary

Pi integration adds multi-provider LLM access to Dirigent through a single CLI tool. The implementation follows the existing provider pattern (settings fields, command builder, stream parser, diff extraction) and can be shipped incrementally. Phase 1 delivers basic functionality; Phase 2 leverages Pi-specific features like `@file` context and thinking control; Phase 3 explores persistent RPC mode for performance.
