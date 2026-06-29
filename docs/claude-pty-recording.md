# Claude PTY recording and replay

`claude_pty` supports optional JSON-lines recording through `ClaudeCodeBuilder::record_to(path)`.
Recordings are diagnostic artifacts and are not enabled by default.

Each recording line is a `RecordingEvent` containing one of:
- PTY size
- raw PTY output bytes
- input bytes written by Dirigent
- emitted PTY events
- session state transitions

Use `claude_pty::replay_chunks(rows, cols, chunks)` for deterministic parser tests that do not spawn Claude Code. Real recordings can contain prompts, file paths, repository content, terminal output, command text, and assistant responses. Sanitize or replace sensitive content before committing any fixture derived from a real session.
