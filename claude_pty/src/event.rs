use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Result of a non-blocking [`Session::poll_event`](crate::Session::poll_event) call.
#[derive(Debug)]
pub enum PollEvent {
    /// An event was available.
    Ready(Event),
    /// Channel open but no event buffered yet.
    Pending,
    /// Reader thread exited and no events remain.
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Raw chunk read from the PTY master, ANSI-stripped. Includes the
    /// vt100 cursor position after the chunk was processed.
    TuiOutput {
        text: String,
        cursor_row: u16,
        cursor_col: u16,
    },
    /// Smart event: fires when genuinely new content appears on
    /// screen — only the lines that are new since the previous
    /// `TuiScreen` event are included, with TUI header / footer
    /// stripped and spinner / status noise filtered out.
    ///
    /// `lines` is plain text (ANSI-stripped). `lines_ansi` is the
    /// same text with embedded SGR escapes so the colors Claude's TUI
    /// used can be reproduced.
    TuiScreen {
        lines: Vec<String>,
        lines_ansi: Vec<String>,
        cursor_row: u16,
        cursor_col: u16,
    },
    /// The TUI showed an `❯` prompt indicator (welcome screen ready
    /// for input).
    TuiPrompt,
    /// The TUI showed a yes/no confirmation dialog. `message` is
    /// either `"Trust folder dialog"` or `"Tool confirmation dialog"`.
    TuiToolConfirmation { message: String },
    /// Heuristic: chunk contained the `●` assistant-message marker.
    TuiAssistantMessage { content: String },

    /// Library: a stdout line we couldn't interpret as a known event.
    LibUnknown { value: Value },
    /// Library: an operational error (IO, parse, etc.).
    LibError { message: String },
    /// Library: the PTY child exited and stdout closed.
    LibDone,
}
