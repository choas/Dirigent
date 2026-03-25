# Dead Code Analysis — Dirigent

All instances of `#[allow(dead_code)]` in the codebase, analyzed for whether the suppressed items are truly unused.

---

## 1. ~~`src/db.rs` — `Cue::source_ref` (line 107)~~ ✅ FIXED

```rust
#[allow(dead_code)]  // ← removed
pub source_ref: Option<String>,
```

**Verdict: FALSE POSITIVE — field IS used.**

The `source_ref` field is actively read in multiple locations:
- `src/app/mod.rs:1330` — extracts PR reference from `cue.source_ref`
- `src/app/mod.rs:1376` — filters cues by `source_ref` for PR notifications
- `src/app/cue_pool/cue_card.rs:571` — checks if cue is PR-sourced
- `src/app/cue_pool/mod.rs:492, 901` — PR source checking
- `src/db.rs:1210` — test assertion

**Resolution:** Removed the stale `#[allow(dead_code)]` annotation.

---

## 2. `src/db.rs` — `Execution` struct fields (lines 117–127)

```rust
#[allow(dead_code)]
pub id: i64,
#[allow(dead_code)]
pub cue_id: i64,
#[allow(dead_code)]
pub prompt: String,
// ...
#[allow(dead_code)]
pub status: ExecutionStatus,
```

**Verdict: TRULY DEAD — these 4 fields are never read after deserialization.**

These fields (`id`, `cue_id`, `prompt`, `status`) are populated when loading an `Execution` from SQLite but are never accessed afterwards. The struct is only consumed via its other fields (`response`, `diff`, `log`, `cost_usd`, `duration_ms`, `num_turns`, `provider`).

**Recommendation:** These fields exist because they are columns in the DB row and are deserialized as part of the full `Execution` struct. Two options:
- **Keep as-is** — they document the DB schema and may be needed in future UI features (e.g., showing execution history details).
- **Remove from struct** — adjust the SQL `SELECT` to omit these columns, but this adds fragility for little gain.

Keeping them is reasonable since they reflect the DB schema, but the `#[allow(dead_code)]` annotations are honest about their status.

---

## 3. ~~`src/sources.rs` — `PrFinding::file_path` and `PrFinding::line_number` (lines 489–493)~~ ✅ FIXED

```rust
#[allow(dead_code)]  // ← removed
pub file_path: String,
#[allow(dead_code)]  // ← removed
pub line_number: usize,
```

**Verdict: TRULY DEAD — fields were constructed but never read.**

These fields were populated when parsing PR review comments but never consumed downstream. The `PrFinding` struct was only used for its `text` and `external_id` fields when creating cues from PR reviews.

**Resolution:** Implemented the "Use them" option — `file_path` and `line_number` from PR findings are now passed through to `insert_cue_from_source()` and stored on the cue. PR review comments that reference a specific file and line now create file-specific cues instead of global ones. Also removed the redundant text prefixing (`In \`file\` (line N): ...`) from `process_inline_comments()` since the location is now stored structurally.

---

## 4. `src/agents.rs` — `AgentKind::builtins()` (line 57)

```rust
#[allow(dead_code)]
pub fn builtins() -> &'static [AgentKind] {
    &[AgentKind::Format, AgentKind::Lint, AgentKind::Build, AgentKind::Test, AgentKind::Outdated]
}
```

**Verdict: USED IN TESTS ONLY.**

Called once in `src/agents.rs:1565` in the `agent_kind_roundtrip` test. The `#[allow(dead_code)]` suppresses the warning because it's only used in `#[cfg(test)]` code.

**Recommendation:** Either:
- Move the function into the `#[cfg(test)]` module since it's test-only.
- Or mark it `#[cfg(test)]` directly.

---

## 5. `src/opencode.rs` — `OpenCodeResponse` struct (line 43)

```rust
#[allow(dead_code)] // Metric fields are stubs for future OpenCode metrics support (§1)
pub(crate) struct OpenCodeResponse {
    pub stdout: String,
    pub edited_files: Vec<String>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    pub num_turns: Option<u64>,
}
```

**Verdict: PARTIALLY DEAD — 3 of 5 fields are never read.**

- `stdout` — **used** in `src/app/claude_run.rs:232, 235, 241`
- `edited_files` — **used** in `src/app/claude_run.rs:231, 234`
- `cost_usd` — **dead**, always constructed as `None`, never read
- `duration_ms` — **dead**, always constructed as `None`, never read
- `num_turns` — **dead**, always constructed as `None`, never read

The consumer (`claude_run.rs:243`) ignores these metrics entirely, using `RunMetrics::default()` instead.

**Recommendation:** Remove the three stub fields. If/when OpenCode metrics parsing is implemented, the fields can be added back. Keeping `None`-only fields that are never read adds noise.

---

## Summary Table

| Location | Item | Status | Recommendation |
|---|---|---|---|
| `db.rs:107` | `Cue::source_ref` | ~~**Used** (false positive)~~ | ✅ Fixed — removed `#[allow(dead_code)]` |
| `db.rs:117` | `Execution::id` | Dead (schema field) | Keep, acceptable |
| `db.rs:119` | `Execution::cue_id` | Dead (schema field) | Keep, acceptable |
| `db.rs:121` | `Execution::prompt` | Dead (schema field) | Keep, acceptable |
| `db.rs:126` | `Execution::status` | Dead (schema field) | Keep, acceptable |
| `sources.rs:489` | `PrFinding::file_path` | ~~Dead (unused feature)~~ | ✅ Fixed — now passed to cue |
| `sources.rs:492` | `PrFinding::line_number` | ~~Dead (unused feature)~~ | ✅ Fixed — now passed to cue |
| `agents.rs:57` | `AgentKind::builtins()` | Test-only | Move to `#[cfg(test)]` |
| `opencode.rs:43` | `OpenCodeResponse::cost_usd` | Dead (stub) | Remove |
| `opencode.rs:43` | `OpenCodeResponse::duration_ms` | Dead (stub) | Remove |
| `opencode.rs:43` | `OpenCodeResponse::num_turns` | Dead (stub) | Remove |

**Total: 11 items annotated, 1 false positive (fixed), 2 implemented (fixed), 4 acceptable schema fields, 4 actionable (3 remaining).**

No additional dead code warnings exist beyond these explicitly suppressed items — the rest of the codebase compiles clean.
