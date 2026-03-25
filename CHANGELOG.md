# Changelog

All notable changes to Dirigent are documented in this file.

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
