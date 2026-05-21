//! `claude_pty` — run the interactive Claude Code TUI under a PTY
//! and stream its output as typed events.
//!
//! The library spawns plain `claude` (no `--print`) under a real PTY via
//! `portable-pty`, ANSI-strips and accumulates the bytes, and emits
//! typed [`Event`]s for the interesting transitions: the workspace-trust
//! / tool-permission dialogs, the prompt indicator, assistant messages,
//! and — most usefully — a [`Event::TuiScreen`] snapshot whenever
//! genuinely new content appears on screen.
//!
//! `TuiScreen.lines` is a **delta**: it carries only the lines that are
//! new since the previous `TuiScreen` event.
//!
//! ## Example
//!
//! ```no_run
//! use claude_pty::{ClaudeCode, Event};
//! use tokio_stream::StreamExt;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut session = ClaudeCode::builder().open()?;
//!     let mut stream = session.take_events().unwrap();
//!     let mut prompt_sent = false;
//!     while let Some(evt) = stream.next().await {
//!         match &evt {
//!             Event::TuiToolConfirmation { message } if message == "Trust folder dialog" => {
//!                 session.write_raw(b"\r")?; // accept
//!             }
//!             Event::TuiPrompt if !prompt_sent => {
//!                 session.write_raw(b"say hi in one word")?;
//!                 tokio::time::sleep(std::time::Duration::from_millis(200)).await;
//!                 session.write_raw(b"\r")?;
//!                 prompt_sent = true;
//!             }
//!             Event::TuiScreen { lines, .. } => {
//!                 for line in lines { println!("{line}"); }
//!             }
//!             Event::LibDone => break,
//!             _ => {}
//!         }
//!     }
//!     Ok(())
//! }
//! ```

pub mod builder;
pub mod event;
pub mod session;

pub use builder::{ClaudeCode, ClaudeCodeBuilder, PermissionMode};
pub use event::{Event, PollEvent};
pub use portable_pty::ExitStatus;
pub use session::Session;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("`claude` binary not found in $PATH; pass .binary(path) on the builder")]
    BinaryNotFound,
    #[error("failed to spawn claude process: {0}")]
    Spawn(String),
    #[error("I/O error: {0}")]
    Io(String),
}
