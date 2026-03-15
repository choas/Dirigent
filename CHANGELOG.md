# Changelog

All notable changes to Dirigent are documented in this file.

## [0.2.3] - 2026-03-16

### Added
- Combined folders in File tree when a folder contains only a single subfolder (e.g. Java-style nested packages)
- "Create release" playbook play now accepts a version number input and creates a git tag

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
