# What's New: Code Understanding & Navigation

## Multi-File Tabs

Files now open in tabs instead of replacing each other. The tab bar appears above the code viewer when multiple files are open. Clicking a file that's already open switches to its tab. Each tab preserves its own selection, cue input, scroll position, and markdown state independently.

- Click tab to switch, `×` to close
- `Cmd+W` closes the active tab
- Soft cap at 20 tabs (oldest auto-closed)

## Breadcrumb Navigation

The file header is now a clickable breadcrumb: `src › app › mod.rs › fn update`. Clicking a directory segment expands it in the file tree. Clicking the filename scrolls to top. The current enclosing symbol (function, struct, etc.) is shown at the end based on selection position.

## Symbol Outline

A collapsible "Outline" section appears below the file tree showing all symbols (functions, structs, enums, traits, classes, etc.) in the current file. Click any symbol to scroll to it. Supports 13 languages: Rust, Python, JS/TS, Go, Java, Kotlin, C/C++, Ruby, Swift, C#, Elixir, Zig, Lua.

## Go-to-Definition

`Cmd+click` on a symbol name jumps to its definition. Searches the current file first, then all project files. When hovering with Cmd held, the code underlines like a link. Uses regex-based pattern matching (e.g., `fn name`, `struct Name`, `class Name`, `def name`).

## Quick File Open (Cmd+P)

`Cmd+P` opens a fuzzy-search overlay for quickly opening any file in the project. Type to filter, Enter to open, Escape to dismiss.

## Navigation History (Cmd+[/])

Every navigation action (file open, search result click, cue navigation, go-to-definition) pushes to a history stack. `Cmd+[` goes back, `Cmd+]` goes forward. Stack depth capped at 50.

## Architecture Changes

- `CodeViewerState` refactored: single-file fields moved into `TabState` struct, viewer now holds `Vec<TabState>` + `active_tab` index
- New `src/app/symbols.rs` module: regex-based symbol parsing, definition pattern generation, word-at-offset extraction
- All call sites updated: file reload after agent runs, reverts, and file-watcher events now iterate over all open tabs
- `NavigationHistory` struct added for back/forward navigation
