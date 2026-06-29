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
    /// Semantic line extracted from the TUI transcript. This is the preferred
    /// app integration event; `TuiScreen` remains available as compatibility
    /// output during migration.
    FilteredLine {
        text: String,
        ansi: String,
        source: LineSource,
        frame_generation: u64,
    },
    /// PTY activity that should tick heartbeats but not enter the response
    /// transcript.
    PtyActivity { source: ActivitySource },
    /// High-level session state derived from the terminal screen.
    SessionStateChanged { state: SessionState },
    /// Structured permission dialog state. Consumers should prefer this over
    /// the legacy `TuiToolConfirmation` event.
    PermissionDialogDetected { dialog: PermissionDialog },
    /// Terminal mode changed while processing output.
    PtyModeChanged {
        mode: TerminalMode,
        enabled: bool,
        frame_generation: u64,
    },
    /// Non-fatal parser or recorder warning.
    ParserWarning { message: String },
    /// A Claude Stop hook payload was observed by the embedding app.
    StopHookCompleted {
        payload_path: String,
        summary: StopHookSummary,
    },
    /// PTY child process was spawned.
    ProcessStarted { pid: Option<u32> },
    /// PTY child process exited, or the PTY output stream closed.
    ProcessExited { status: Option<String> },
    /// PTY child process was force-killed.
    ProcessKilled,

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineSource {
    AssistantText,
    ToolHeader,
    ToolOutput,
    PermissionUi,
    PromptEcho,
    SystemLine,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivitySource {
    RawOutput,
    EmptyLine,
    StatusChrome,
    Spinner,
    Redraw,
    Prompt,
    Permission,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Starting,
    ReadyForInput,
    PromptSubmitted,
    Running,
    WaitingForPermission,
    DrainingAfterStop,
    Exiting,
    Exited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDialogKind {
    TrustFolder,
    ToolUse,
    PlanEnter,
    PlanExit,
    AskUserQuestion,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionDialog {
    pub kind: PermissionDialogKind,
    pub title: String,
    pub body: String,
    pub tool_name: Option<String>,
    pub path_or_command: Option<String>,
    pub options: Vec<String>,
    pub selected_option: Option<usize>,
    pub frame_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalMode {
    AlternateScreen,
    BracketedPaste,
    FocusEvents,
    MouseTracking,
    SynchronizedOutput,
    CursorVisible,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopHookSummary {
    pub event_name: Option<String>,
    pub last_assistant_message: Option<String>,
    pub failure: Option<String>,
    pub session_id: Option<String>,
    pub timestamp: Option<String>,
}
