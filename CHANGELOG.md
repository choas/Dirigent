# Changelog

All notable changes to Dirigent are documented in this file.

## [Unreleased]

### Added
- Trust gate for `.Dirigent/settings.json`: execution-capable fields (LSP server commands, source commands, pre/post-run scripts, agent commands, extra CLI args, program paths, LLM endpoints, …) from a repository's settings file are disabled — and `--dangerously-skip-permissions` is forced off — until the user explicitly approves exactly those fields in a first-open dialog. Approvals are remembered per repository by content hash in the user profile (`~/Library/Application Support/Dirigent/trusted_repos.json`); only cosmetic fields (theme, fonts, layout, notifications) apply without approval, and any future settings field is gated by default
- Git graph view with branch/commit visualization and incremental history loading
- "Tidy my working tree" — the coding-agent CLI groups the working diff into logical change sets, which become a reviewable, reorderable commit queue
- Hunk-level staging: stage, unstage, and discard individual hunks from the diff review
- "Analyze changes over…" — cross-branch/range review that imports findings as cues
- Folder comparison, with support for an external diff tool (`external_diff_tool`)
- Pull request creation on Codeberg via the Forgejo REST API, alongside GitHub
- In-app log viewer and a Mermaid diagram view
- CI workflow running `cargo test`, `cargo clippy -D warnings`, `cargo fmt --check` and `cargo audit` on every push and pull request — previously only tag pushes were built, and nothing verified them
- Recovery dialog when `.Dirigent/Dirigent.db` cannot be opened (locked, corrupt, permission-denied), offering retry, "open another repo", "move aside and recreate", or continuing on a temporary in-memory database
- `PRAGMA user_version` schema stamping; a database written by a newer Dirigent is now refused at open instead of being silently half-understood

### Changed
- All four AI CLI backends (Claude, Gemini, OpenCode, Codex) now run through a single `AiBackend` trait with one canonical sequence — pre-run hook, CLI invocation, post-run hook (always executed, even on error or cancel), cancellation winning over the CLI result — and one shared metrics mapping, error type, and request/response shape
- Conversation view groups each CLI run under a foldable header (run number, provider, prompt preview, and duration/cost) — the latest and in-flight runs stay expanded while earlier runs collapse for easier scanning of long conversations
- Commit flows include staged changes, and staged queue groups stay queued until they are committed
- Hook scripts now follow one policy across all four backends, derived from the repository trust gate rather than hard-coded per backend: an unapproved script is skipped with a warning (never fatal), an approved one runs. Previously Codex refused *every* hook — disabling the backend outright whenever one was configured — while the other three handed the same configuration straight to `sh -c`
- Analysis runs ("Tidy my working tree", "Analyze changes over…", cue splitting, workflow planning, theme generation) no longer inherit `--dangerously-skip-permissions`, and the diff they embed is explicitly delimited as untrusted data
- Claude Code's trust-folder dialog is only auto-accepted for a repository already approved in Dirigent's trust gate; accepting it activates that repository's `.claude/settings.json`
- `git` and `jj` are invoked with `LC_ALL=C`, so conflict/diverged/stale detection keeps working under a non-English locale
- `sources/external.rs` split into one module per provider under `sources/providers/`, with parser tests against captured fixtures for all seven
- `app/claude_run.rs` split into `provider_dispatch`, `mcp_inject`, `run_diff` and `result_pipeline`; PR-findings import moved out of `app/git_operations.rs` into `db/pr_finding_ops.rs`
- Unknown run metrics are recorded as NULL instead of zero, so cost totals no longer count a run whose cost the backend never reported as free

### Fixed
- Claude and Codex runs recorded a zero duration; the run duration is now measured at the backend seam when the CLI stream doesn't report one
- Gemini and OpenCode runs panicked instead of erroring when the child's stdout/stderr pipes were missing, and Gemini could block indefinitely on a stream read error (the child is now killed first, as OpenCode already did)
- Cancelling a Gemini/OpenCode run skipped the post-run hook, and a cancel racing run completion could leave the child process unkilled
- Remote code execution via LSP environment variables in an attacker-controlled `.Dirigent/settings.json`
- Two advisories that failed `cargo audit`: `h2` 0.4.13 → 0.4.18 (RUSTSEC-2026-0258, unbounded empty DATA frames) and `webbrowser` 1.2.0 → 1.2.4 (RUSTSEC-2026-0257, argument injection via the Unix `BROWSER` variable)
- Orphaned PTY processes are cleaned up, and stale Stop-hook entries are pruned on install
- File renames made outside Dirigent are followed instead of closing the tab as deleted
- Only the *first* tool-permission dialog of a PTY run was auto-accepted; later ones went unanswered until the 120 s idle timeout killed the run. Dialog state is now read from the live terminal screen instead of an append-only buffer
- Repository or model output containing "How is Claude doing this session" or "Resume from summary" could make Dirigent type into the PTY at a moment the content chose. Those answers are now driven only by a screen-verified dialog
- A permission dialog nothing could answer blocked a run forever (no timeout, no UI); the stall guard now stays armed while waiting
- A corrupt `.Dirigent/settings.json` silently reset every preference; it is now backed up to `settings.json.bak-<timestamp>` and the failure is reported
- `jj` partial commits snapshot the other dirty files to disk before reverting them, so a crash in that window no longer discards uncommitted work; write-back failures are reported instead of ignored
- Discarding a hunk now asks for confirmation and leaves a `git stash` undo point
- Unbounded buffers on child-process output and LSP `Content-Length` bodies are capped
- PTY, LSP and agent processes are killed by process group and reaped, instead of leaving zombies and orphaned descendants; agents are cancelled on quit
- Staging refresh ran two `git diff` subprocesses per file on the UI thread (~100 spawns for a 50-file diff); one `git status --porcelain=v2` call replaces them
- The graph view re-walked the whole history every frame while parked at the bottom of a large repository
- Starting a split-commit analysis or opening the commit dialog mid-queue silently discarded queued groups
- "Analyze changes over…" reported a bad ref as "Nothing to analyze between those refs", and had no oversized-diff guard

## [0.6.1] - 2026-05-28

### Changed
- Version bump release

## [0.6.0] - 2026-05-28

### Added
- jj (Jujutsu) VCS backend with bookmark management, log viewer, commit workflow, squash, and workspace support
- Claude PTY integration for pseudo-terminal Claude Code interaction with idle-timeout, auto-accept, and session reuse
- Auto-continue for runs that stop mid-task (reuses session ID)
- Dirigent MCP server for external tool integration
- Gemini CLI as an alternative CLI provider
- PR status indicator in status bar with clickable link to GitHub
- Custom Dock logo selection in Settings
- In-app log viewer with structured logging
- Running animation selector (Lava Lamp, Claude character, Desert Dino) with customizable heartbeat styles (Wave, Morse Code)
- Frame timing performance monitor in status bar (toggle in Settings)
- Diff view "Open File" button per file
- Sentry as a cue source
- Project logo detection (logo.png in project root)
- Scrollable tab bar for many open files

### Fixed
- Claude PTY prompt detection stuck after first response
- PTY timeout and stop detection reliability
- JSON extraction balanced-bracket parsing inside string literals
- UTF-8 slice panics in workflow JSON parse failures
- PR creation from linked worktrees nested inside the main repo
- Tab context menu not appearing on right-click
- Git push error messages now show concrete error details
- Diff file paths preserved when subdirectory matches repo name
- MCP server crash on database access and config write without mkdir
- Auto-commit race condition with concurrent agents
- Stale git status from wrong repo overwriting current repo
- Side-by-side diff bounds-checked lookups
- Numerous code review findings (CodeRabbit, Qodo) verified and fixed

### Changed
- Claude Code invocation refactored to use PTY by default (with fallback toggle in Settings)
- Unified per-provider diff fallback order between Claude and Gemini
- jj operations integrated into existing Git menus and status bar
- Documentation reorganized into docs/ directory
- Removed macOS build workflow (CI simplification)

## [0.5.1] - 2026-05-12

### Fixed
- GitHub Actions release build: updated Info.plist and bundle script to fix code signing validation

## [0.5.0] - 2026-05-11

### Added
- SSH remote server connection with dedicated background thread
- Git Changes view showing only modified files with Delete and Discard actions
- Git pull dialog with Merge, Rebase, and Fast-forward options
- Branch listing and inspection across all branches
- Diff color customization in Settings
- Custom color theme import and export
- Bun agent presets in Settings (with Swift agent support)
- Inline prompt button for whole-document context
- Follow-up log message appended to commit messages
- Auto-commit toggle for workflow cue processing
- macOS file detection for new projects (`.DS_Store`, etc.)
- Git init defaults to "main" branch instead of "master"

### Fixed
- SSH host key verification (security hardening)
- SSH connection test no longer blocks the UI
- Auto-commit race condition with concurrent agents
- Push error dialog rendering failure
- Notion source "Fetch Now" not returning results
- Settings page losing changes on refresh
- Git Log view sometimes showing empty screen
- Git push error handling and display
- Playbook preset cleanup (removed stale plays)
- 15+ code review findings verified and fixed against current code
- Quality loop prompt logging, comment filtering, and commit flow hardened

### Changed
- SSH operations refactored to dedicated background thread
- Bun agent commands standardized to `bun run` format

## [0.4.0] - 2026-04-27

### Added
- File context menu with right-click actions (use in prompt, copy path)
- "Delete All Archived" cue action for bulk cleanup
- Delete button on individual archived cues
- Split cues feature for breaking multi-task cues into separate items
- Font weight adjustment in Settings
- Custom theme color picker in Settings
- Search button in code viewer
- Toggle to show .gitignore-ignored files in Open File dialog
- Runnable source items ({runnable} tag support for Notion and other sources)
- Database index audit and optimization
- Installable CLI command (`dirigent`) for launching from any folder
- Theme generator via CLI using the selected code generator
- Temporary prompt log entries before sending
- Document position persistence when switching between files

### Fixed
- Plan actions blocked when processing view in certain states
- Swift agent preset limited to Xcode-only configuration
- Iterator chain logic correctness issues
- Panic surfaces across codebase (audit and hardening)
- Dead code and unreachable branches removed
- Quality loop iteration counter drift and orphan detection
- Archive-all-done only archiving currently loaded Done cues
- File close behavior when multiple files are open
- Verified and fixed 60+ PR code review findings against actual code

### Changed
- Comprehensive code review: UTF-8/byte boundary safety, per-frame allocation audit, error silencing audit, keyboard shortcut conflict detection, state machine completeness, security/path traversal review, accessibility improvements
- Resource leak and unbounded growth prevention
- Duplicated construction logic consolidated (DRY refactor)
- Regex correctness review and fixes
- Test coverage gap analysis with new tests (355 total)
- Settings auto-reload from disk on file changes and after Claude runs
- Quality loop resilience improvements
- Workflow Plan UI improvements (conditional re-plan button)

## [0.3.6] - 2026-04-07

### Added
- Git graph visualization with lane coloring, hover highlighting, lane collapsing (max 6 lanes), and octopus merge support
- Image preview in Code View when clicking image files
- Switch Branch feature for moving uncommitted changes to a new branch
- Commit diff viewer (click commit ID in Git Log to see diff)
- OpenTelemetry tracing and agent audit polling
- Diataxis-style documentation structure
- Playbook plays for dependency version pinning and zero-day security testing

### Fixed
- PR auto-detect returning incorrect results
- Prompt field slow typing and cursor movement
- Slow loading with many cues or commits (performance optimization)
- Red borders in Nord theme code viewer
- Workflow Plan close button not working
- Notion source not splitting todo list items
- PR creation failing with too many commits
- Casting `bool` as `f32` compiler errors
- All compiler warnings resolved

### Changed
- Archived cues limit reduced from 50 to 10 for performance
- Extensive SonarQube-driven code quality improvements (cognitive complexity, parameter counts, code duplication)
- Security hardening: Trello API key exposure, Sentry PII settings, ObjC interop safety, command validation
- Unwrap audit across codebase (replaced unsafe `.unwrap()` calls)
- Quality loop script improvements (multi-language, CodeRabbit integration, resilience)
- Dependency pinning for reproducible builds
- Updated README documentation

## [0.3.5] - 2026-04-02

### Added
- Native Notion integration as a cue source with configurable settings
- LSP (Language Server Protocol) support with server settings and diagnostics
- PR findings filter dialog for selective import
- Sentry error tracking integration
- Trello and Asana source options in Settings
- Automated quality loop script for bulk cue processing
- ANSI escape sequence stripping for OpenCode output

### Fixed
- Notion source reliability and non-blocking UI during sync
- OpenCode output streaming (real-time instead of end-of-run)
- OpenCode error formatting (ANSI escape cleanup)
- Worktree manager showing deleted branches
- Font size slider missing in Settings
- Scheduled runs not starting when computer is locked
- Notification click action at status bar
- No-changes runs incorrectly marking cues as Done
- Quality loop prompt logging, CR comment filtering, and commit flow

### Changed
- Extensive SonarQube-driven code quality improvements (cognitive complexity, parameter counts, wildcard imports)
- Split large source files into module directories for maintainability
- SQL identifier validation in migrations to prevent injection
- Updated README documentation
- Updated dependencies (eframe 0.34, rusqlite 0.39, git2 0.20)

## [0.3.4] - 2026-03-26

### Fixed
- PR findings import now preserves file path and line number on cues
- Done/Archived cues no longer reset on PR re-import when content is unchanged
- Clamped line numbers to 1 for file-specific PR findings (avoids invalid navigation)
- HTML tags stripped from imported PR finding text (Qodo, CodeRabbit)
- Qodo review summary headers filtered out as non-actionable
- Trailing PR context hints normalised to prevent false "text changed" detection

### Changed
- Dead code cleanup: removed `#[allow(dead_code)]` annotations from actively used fields
- Refactored `update_cue_text_by_source_ref` to also update file path and line number
- Cognitive complexity reduction in `fetch_pr_findings`
- Updated `num-conv` transitive dependency (0.2.0 to 0.2.1)
- Minor formatting cleanup in agents and app modules

## [0.3.3] - 2026-03-25

### Added
- Outdated dependency check agent
- Markdown file outline with heading navigation
- File rename via right-click context menu in Files panel
- SonarQube integration script and source support
- Agent log analysis button for diagnosing failed runs
- Function identification across programming languages
- Follow-up messages for running cues

### Fixed
- Clicking "Run" sometimes not starting Claude Code
- Worktree dialog closing unexpectedly on click
- Main branch removal protection in Worktree manager
- SonarQube source URL display
- Rename tab update logic for directories vs files

### Changed
- Extensive SonarQube-driven code quality improvements (cognitive complexity reduction, parameter count reduction)
- Resolved all clippy warnings (40+ fixes)
- Added `Screenshot*` to `.gitignore`

## [0.3.2] - 2026-03-23

### Added
- Run metrics tracking (cost, duration, tokens) for executions
- Prompt auto-context and refinement hints
- Execution history search

### Fixed
- Left-clicking a file tab not switching to that file
- First character lost when typing in the global prompt
- Clicking "Change" opening incorrect dialog
- Worktree dialog click handling
- Git pull merge conflict resolution

### Changed
- Removed unused `search_executions` database method
- Various code quality fixes from PR findings analysis
- Resolved all compiler warnings
- Updated README documentation

## [0.3.1] - 2026-03-22

### Added
- Close-all, close-others, close-to-right tab operations
- Unpushed commit indicators in Git Log
- Commit All includes all cue texts in the commit message

### Fixed
- Outline not jumping to the correct line for symbols like `fn drop`
- Various code quality fixes from PR findings analysis
- Compilation error fix

## [0.3.0] - 2026-03-22

### Added
- Code understanding: symbol parsing, quick-open (Cmd+P), file tabs, breadcrumbs, and go-to-definition
- GitHub Pull Request creation dialog
- PR findings import with analyze-and-fix workflow
- Highlighted Push button on Done cues when local commits are ahead of remote
- Keyboard navigation (arrow keys) in code viewer
- Pagination for Archived column (show 50 with load more)

### Fixed
- Cancel superseded go-to-definition requests
- Worktree creation when no remote repo is configured
- Various code quality fixes from PR findings analysis
- Resolved all compiler warnings

## [0.2.6] - 2026-03-20

### Changed
- Version bump release

## [0.2.5] - 2026-03-20

### Added
- Markdown viewer integration using pulldown-cmark
- Copy filename (with path) button in code viewer
- Lua language option in Agent Settings
- Git pull support

### Fixed
- Font size inconsistency between Files and Git Log panels
- Git Log entries cut off even when space was available
- Prompt input styling inconsistency in CLI logs
- Claude Code attempting to read folders from the user's filesystem

## [0.2.4] - 2026-03-16

### Changed
- Version bump release

## [0.2.3] - 2026-03-16

### Added
- Combined folders in File tree when a folder contains only a single subfolder (e.g. Java-style nested packages)
- "Create release" playbook play now accepts a version number input and creates a git tag
- Tag support for cues via three-dot menu, with bulk tagging for Review column
- Collapsible toggle for long cue text (more than 50 words or 10 lines)
- Context hints about Dirigent and SQLite cue editing when prompting outside the project

### Fixed
- Claude Code log view not updating during and after runs
- Resolved all compiler warnings from `cargo check`

## [0.2.1] - 2026-03-15

### Added
- Manual workflow dispatch for GitHub Actions build

## [0.2.0] - 2026-03-14

### Added
- Animated lava lamp overlay shown while cues are running
- Run button dropdown with multiple actions in Inbox
- Cancel button for agent timeout in Settings

### Fixed
- String slicing panics on multi-byte UTF-8 characters
- Claude Code log output role label formatting

### Changed
- Refactored cue pool module into smaller submodules
- Updated agent configurations

## [0.1.2] - 2026-02-18

### Added
- Post-run agents system (Format, Lint, Build, Test) with configurable triggers
- Agent Initialize button with language-specific defaults
- Agent log viewer and cue activity logbook with expandable output
- Agent last run display in Settings
- Activity timestamps for cue lifecycle events
- OpenCode as alternative CLI provider (OpenAI, Anthropic, Google models)
- New Window support (Cmd+N) for multiple instances
- Reply field and send button in conversation log view
- Commit All button at Review stage

### Fixed
- File view not refreshing after commits
- Diff review green text readability on light themes
- macOS "would like to access Apple Music" TCC dialog
- macOS window title showing full path for installed .app
- Cues running with OpenCode ending at Done instead of Review

### Changed
- Cues sorted latest-on-top
- Claude CLI auto-detected via `which claude`
- Language Initialize moved below Agents definition in Settings
- Java Maven agent commands updated
- Project handle renamed to net.choas.macos.dirigent
- Import from document defaults to project folder
- Theme palette unified with visual refinements
- Dialog windows redesigned with proper windowed appearance

## [0.1.1] - 2026-01-28

### Added
- Semantic color palette and visual polish
- Rounded corners, consistent spacing, streamlined status bar
- Full conversation history in Claude run log view
- Reply to Claude Code from any cue (not just Review)
- Image attachment support for cues
- Search in Diff Review (Cmd+F)
- Playbook feature with predefined prompts
- CLI path and extra arguments settings
- macOS .app bundle, code signing, DMG packaging, GitHub Actions release
- Backlog column for long-term cue management
- Ignored files (.gitignore) visualized in grey
- Markdown document import with upsert
- In-file search (Cmd+F) and project-wide search (Cmd+Shift+F)
- Source integration (GitHub Issues, Notion, MCP, custom commands)
- File close support
- Dirty-file tracking indicators in file tree
- macOS notifications with click-to-activate

### Fixed
- Unicode icon inconsistencies
- Commit not including all files
- Notification opening Scripting Editor instead of Dirigent
- Null pointer crash in macOS notification on modern macOS
- Button clicks blocked by open dialogs
- Git commit messages missing full cue text

### Changed
- App modules split for maintainability (mod.rs, dialogs.rs)
- Named constants, deduplication, unified error handling
- Settings page scrollable

## [0.1.0] - 2025-12-20

### Added
- Initial release
- File tree with ignore patterns
- Code viewer with syntect syntax highlighting and cue markers
- SQLite persistence (cues, executions, migrations)
- Cue system (create, edit, archive, status transitions)
- Cue pool / kanban (Inbox, Ready, Review, Done, Archived)
- Claude Code CLI integration with streaming progress
- Diff view (inline and side-by-side) with per-file collapsing
- Git integration (status, history, commit, worktrees)
- 20 themes (10 dark, 10 light)
- Settings (theme, model, font family and size)
- Repository picker with recent repos
- macOS native About panel and dock icon
- File system watching with debounced rescan
