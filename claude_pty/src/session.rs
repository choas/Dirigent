use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{
    native_pty_system, Child as PtyChild, CommandBuilder, MasterPty, PtyPair, PtySize,
};
use regex::Regex;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::builder::ResolvedSpec;
use crate::event::{
    ActivitySource, Event, LineSource, PermissionDialog, PermissionDialogKind, PollEvent,
    SessionState, TerminalMode,
};
use crate::Error;

#[derive(Debug, Clone)]
pub struct RecordingConfig {
    pub path: PathBuf,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecordingEvent {
    Output { at_ms: u128, bytes: Vec<u8> },
    Input { at_ms: u128, bytes: Vec<u8> },
    Event { at_ms: u128, event: Event },
    State { at_ms: u128, state: SessionState },
    PtySize { at_ms: u128, rows: u16, cols: u16 },
}

#[derive(Clone)]
struct Recorder {
    file: Arc<Mutex<std::fs::File>>,
    started_at: Instant,
}

impl Recorder {
    fn new(path: PathBuf, rows: u16, cols: u16) -> Option<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        let recorder = Self {
            file: Arc::new(Mutex::new(file)),
            started_at: Instant::now(),
        };
        recorder.record(RecordingEvent::PtySize {
            at_ms: 0,
            rows,
            cols,
        });
        Some(recorder)
    }

    fn at_ms(&self) -> u128 {
        self.started_at.elapsed().as_millis()
    }

    fn record(&self, mut event: RecordingEvent) {
        let at_ms = self.at_ms();
        match &mut event {
            RecordingEvent::Output { at_ms: t, .. }
            | RecordingEvent::Input { at_ms: t, .. }
            | RecordingEvent::Event { at_ms: t, .. }
            | RecordingEvent::State { at_ms: t, .. }
            | RecordingEvent::PtySize { at_ms: t, .. } => *t = at_ms,
        }
        let Ok(mut file) = self.file.lock() else {
            return;
        };
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = writeln!(file, "{line}");
        }
    }
}

#[derive(Debug, Default)]
struct TerminalState {
    bracketed_paste: bool,
    alternate_screen: bool,
}

pub struct Session {
    master: Box<dyn MasterPty + Send>,
    child: Mutex<Box<dyn PtyChild + Send + Sync>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    events_tx: mpsc::Sender<Event>,
    events: Option<mpsc::Receiver<Event>>,
    terminal_state: Arc<Mutex<TerminalState>>,
    recorder: Option<Recorder>,
    rows: Mutex<u16>,
    cols: Mutex<u16>,
}

impl Session {
    pub(crate) fn spawn(spec: ResolvedSpec) -> Result<Self, Error> {
        let pty_system = native_pty_system();
        let PtyPair { master, slave } = pty_system
            .openpty(PtySize {
                rows: spec.rows,
                cols: spec.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::Spawn(e.to_string()))?;

        let mut cmd = CommandBuilder::new(&spec.binary);
        for arg in &spec.args {
            cmd.arg(arg);
        }
        for (key, value) in &spec.env_vars {
            cmd.env(key, value);
        }
        if let Some(cwd) = &spec.cwd {
            cmd.cwd(cwd);
        }

        let child = slave
            .spawn_command(cmd)
            .map_err(|e| Error::Spawn(e.to_string()))?;
        drop(slave);

        let writer = master
            .take_writer()
            .map_err(|e| Error::Spawn(e.to_string()))?;
        let reader = master
            .try_clone_reader()
            .map_err(|e| Error::Spawn(e.to_string()))?;

        let (tx, rx) = mpsc::channel::<Event>(1024);
        let terminal_state = Arc::new(Mutex::new(TerminalState::default()));
        let recorder = spec
            .recording
            .as_ref()
            .and_then(|cfg| Recorder::new(cfg.path.clone(), spec.rows, spec.cols));
        spawn_reader(
            reader,
            tx.clone(),
            spec.rows,
            spec.cols,
            terminal_state.clone(),
            recorder.clone(),
        );
        emit_critical(&tx, recorder.as_ref(), Event::ProcessStarted { pid: None });
        emit_critical(
            &tx,
            recorder.as_ref(),
            Event::SessionStateChanged {
                state: SessionState::Starting,
            },
        );

        Ok(Self {
            master,
            child: Mutex::new(child),
            writer: Arc::new(Mutex::new(writer)),
            events_tx: tx,
            events: Some(rx),
            terminal_state,
            recorder,
            rows: Mutex::new(spec.rows),
            cols: Mutex::new(spec.cols),
        })
    }

    /// Takes the event stream. Can only be called once per session.
    pub fn take_events(&mut self) -> Option<ReceiverStream<Event>> {
        self.events.take().map(ReceiverStream::new)
    }

    /// Writes `text\r` to the PTY — the TUI treats `\r` as "submit".
    pub fn send_line(&self, text: &str) -> Result<(), Error> {
        self.paste(text)?;
        self.submit()
    }

    pub fn send_user_message(&self, text: &str) -> Result<(), Error> {
        self.send_prompt(text)
    }

    pub fn write_raw(&self, data: &[u8]) -> Result<(), Error> {
        let mut w = self
            .writer
            .lock()
            .map_err(|_| Error::Io("writer poisoned".into()))?;
        w.write_all(data).map_err(|e| Error::Io(e.to_string()))?;
        w.flush().map_err(|e| Error::Io(e.to_string()))?;
        if let Some(recorder) = &self.recorder {
            recorder.record(RecordingEvent::Input {
                at_ms: 0,
                bytes: data.to_vec(),
            });
        }
        Ok(())
    }

    pub fn send_prompt(&self, prompt: &str) -> Result<(), Error> {
        self.paste(prompt)?;
        // Let the TUI absorb the paste before Enter. Without this gap the
        // submit `\r` lands in the same write batch as the bracketed-paste end
        // marker and Claude's paste handler swallows it, so the prompt is never
        // submitted.
        thread::sleep(Duration::from_millis(200));
        self.submit()?;
        emit_critical(
            &self.events_tx,
            self.recorder.as_ref(),
            Event::SessionStateChanged {
                state: SessionState::PromptSubmitted,
            },
        );
        Ok(())
    }

    pub fn paste(&self, text: &str) -> Result<(), Error> {
        let bracketed = self
            .terminal_state
            .lock()
            .map(|s| s.bracketed_paste)
            .unwrap_or(false);
        self.write_raw(&paste_bytes(text, bracketed))
    }

    pub fn send_key(&self, data: &[u8]) -> Result<(), Error> {
        self.write_raw(data)
    }

    pub fn submit(&self) -> Result<(), Error> {
        self.write_raw(b"\r")
    }

    pub fn select_permission_option(&self, index: usize) -> Result<(), Error> {
        let digit = b'1'.saturating_add(index.min(8) as u8);
        self.write_raw(&[digit])
    }

    pub fn confirm_permission_option(&self) -> Result<(), Error> {
        self.submit()
    }

    pub fn escape(&self) -> Result<(), Error> {
        self.write_raw(b"\x1b")
    }

    pub fn interrupt_turn(&self) -> Result<(), Error> {
        self.write_raw(b"\x03")
    }

    pub fn request_exit(&self) -> Result<(), Error> {
        self.interrupt_turn()?;
        std::thread::sleep(Duration::from_millis(500));
        self.interrupt_turn()
    }

    pub fn close_gracefully(&mut self, timeout: Duration) -> Result<(), Error> {
        self.close_gracefully_draining(timeout, |_| {})
    }

    /// Like [`close_gracefully`](Self::close_gracefully) but forwards every
    /// event polled while draining to `on_event`.
    ///
    /// Interrupting Claude's turn (the `Ctrl-C` sent by [`request_exit`]) makes
    /// Claude Code print an "Interrupted · What should Claude do instead?" line
    /// and record it in the session transcript. The plain `close_gracefully`
    /// discards those drained events, so embedders never see that line until
    /// they `--resume` the session in a real terminal. This variant lets the
    /// caller surface them in its own log instead.
    pub fn close_gracefully_draining(
        &mut self,
        timeout: Duration,
        mut on_event: impl FnMut(&Event),
    ) -> Result<(), Error> {
        emit_critical(
            &self.events_tx,
            self.recorder.as_ref(),
            Event::SessionStateChanged {
                state: SessionState::Exiting,
            },
        );
        let _ = self.request_exit();
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match self.poll_event() {
                PollEvent::Ready(event @ (Event::LibDone | Event::ProcessExited { .. })) => {
                    on_event(&event);
                    return Ok(());
                }
                PollEvent::Ready(event) => on_event(&event),
                PollEvent::Pending => std::thread::sleep(Duration::from_millis(50)),
                PollEvent::Closed => return Ok(()),
            }
        }
        self.force_kill()
    }

    pub fn force_kill(&self) -> Result<(), Error> {
        self.kill()
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), Error> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::Io(e.to_string()))?;
        if let Ok(mut r) = self.rows.lock() {
            *r = rows;
        }
        if let Ok(mut c) = self.cols.lock() {
            *c = cols;
        }
        if let Some(recorder) = &self.recorder {
            recorder.record(RecordingEvent::PtySize {
                at_ms: 0,
                rows,
                cols,
            });
        }
        emit_critical(
            &self.events_tx,
            self.recorder.as_ref(),
            Event::ParserWarning {
                message: format!("pty resized to {rows}x{cols}"),
            },
        );
        Ok(())
    }

    pub fn kill(&self) -> Result<(), Error> {
        let mut c = self
            .child
            .lock()
            .map_err(|_| Error::Io("child poisoned".into()))?;
        c.kill().map_err(|e| Error::Io(e.to_string()))?;
        emit_critical(
            &self.events_tx,
            self.recorder.as_ref(),
            Event::ProcessKilled,
        );
        Ok(())
    }

    pub fn wait(&self) -> Option<portable_pty::ExitStatus> {
        let status = self.child.lock().ok().and_then(|mut c| c.wait().ok());
        if let Some(s) = &status {
            emit_critical(
                &self.events_tx,
                self.recorder.as_ref(),
                Event::ProcessExited {
                    status: Some(s.to_string()),
                },
            );
        }
        status
    }

    /// Non-blocking check for whether the child has exited.
    /// Returns `Some(status)` if exited, `None` if still running.
    pub fn try_wait(&self) -> Option<portable_pty::ExitStatus> {
        self.child
            .lock()
            .ok()
            .and_then(|mut c| c.try_wait().ok().flatten())
    }

    /// Non-blocking event poll. Returns [`PollEvent::Ready`] with the
    /// next event if one is buffered, [`PollEvent::Pending`] if the
    /// channel is open but empty, or [`PollEvent::Closed`] when the
    /// reader thread has exited and no events remain.
    pub fn poll_event(&mut self) -> PollEvent {
        let rx = match self.events.as_mut() {
            Some(rx) => rx,
            None => return PollEvent::Closed,
        };
        match rx.try_recv() {
            Ok(event) => PollEvent::Ready(event),
            Err(mpsc::error::TryRecvError::Empty) => PollEvent::Pending,
            Err(mpsc::error::TryRecvError::Disconnected) => PollEvent::Closed,
        }
    }
}

fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let mut buf = Vec::with_capacity(text.len() + 12);
        buf.extend_from_slice(b"\x1b[200~");
        buf.extend_from_slice(text.as_bytes());
        buf.extend_from_slice(b"\x1b[201~");
        buf
    } else {
        text.as_bytes().to_vec()
    }
}

#[cfg(test)]
fn prompt_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let mut buf = paste_bytes(text, bracketed);
    buf.push(b'\r');
    buf
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}

fn spawn_reader(
    reader: Box<dyn Read + Send>,
    tx: mpsc::Sender<Event>,
    rows: u16,
    cols: u16,
    terminal_state: Arc<Mutex<TerminalState>>,
    recorder: Option<Recorder>,
) {
    thread::spawn(move || read_tui(reader, tx, rows, cols, terminal_state, recorder));
}

fn emit_critical(tx: &mpsc::Sender<Event>, recorder: Option<&Recorder>, event: Event) -> bool {
    if let Some(recorder) = recorder {
        recorder.record(RecordingEvent::Event {
            at_ms: 0,
            event: event.clone(),
        });
        if let Event::SessionStateChanged { state } = event {
            recorder.record(RecordingEvent::State { at_ms: 0, state });
            return tx
                .blocking_send(Event::SessionStateChanged { state })
                .is_ok();
        }
    }
    tx.blocking_send(event).is_ok()
}

fn emit_lossy(tx: &mpsc::Sender<Event>, recorder: Option<&Recorder>, event: Event) -> bool {
    if let Some(recorder) = recorder {
        recorder.record(RecordingEvent::Event {
            at_ms: 0,
            event: event.clone(),
        });
    }
    match tx.try_send(event) {
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Closed(_)) => false,
        Err(mpsc::error::TrySendError::Full(_)) => true,
    }
}

enum LossyEmit {
    Sent,
    Dropped,
    Closed,
}

fn emit_raw_output_lossy(
    tx: &mpsc::Sender<Event>,
    recorder: Option<&Recorder>,
    event: Event,
) -> LossyEmit {
    if let Some(recorder) = recorder {
        recorder.record(RecordingEvent::Event {
            at_ms: 0,
            event: event.clone(),
        });
    }
    match tx.try_send(event) {
        Ok(()) => LossyEmit::Sent,
        Err(mpsc::error::TrySendError::Full(_)) => LossyEmit::Dropped,
        Err(mpsc::error::TrySendError::Closed(_)) => LossyEmit::Closed,
    }
}

#[derive(Default, PartialEq, Eq, Clone)]
struct StyleState {
    fg: Option<String>,
    bg: Option<String>,
    bold: bool,
    italic: bool,
    underline: bool,
    inverse: bool,
}

impl StyleState {
    fn from_cell(cell: &vt100::Cell) -> Self {
        Self {
            fg: fg_sgr(&cell.fgcolor()),
            bg: bg_sgr(&cell.bgcolor()),
            bold: cell.bold(),
            italic: cell.italic(),
            underline: cell.underline(),
            inverse: cell.inverse(),
        }
    }
    fn is_default(&self) -> bool {
        self.fg.is_none()
            && self.bg.is_none()
            && !self.bold
            && !self.italic
            && !self.underline
            && !self.inverse
    }
    fn to_sgr(&self) -> String {
        if self.is_default() {
            return "\x1b[0m".to_string();
        }
        let mut parts: Vec<&str> = vec!["0"];
        if let Some(c) = &self.fg {
            parts.push(c);
        }
        if let Some(c) = &self.bg {
            parts.push(c);
        }
        if self.bold {
            parts.push("1");
        }
        if self.italic {
            parts.push("3");
        }
        if self.underline {
            parts.push("4");
        }
        if self.inverse {
            parts.push("7");
        }
        format!("\x1b[{}m", parts.join(";"))
    }
}

fn fg_sgr(c: &vt100::Color) -> Option<String> {
    match c {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => Some(if *i < 8 {
            format!("3{i}")
        } else if *i < 16 {
            format!("9{}", i - 8)
        } else {
            format!("38;5;{i}")
        }),
        vt100::Color::Rgb(r, g, b) => Some(format!("38;2;{r};{g};{b}")),
    }
}

fn bg_sgr(c: &vt100::Color) -> Option<String> {
    match c {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => Some(if *i < 8 {
            format!("4{i}")
        } else if *i < 16 {
            format!("10{}", i - 8)
        } else {
            format!("48;5;{i}")
        }),
        vt100::Color::Rgb(r, g, b) => Some(format!("48;2;{r};{g};{b}")),
    }
}

fn ansi_row(screen: &vt100::Screen, row: u16) -> String {
    let cols = screen.size().1;
    let mut out = String::new();
    let mut current = StyleState::default();
    let mut emitted_any_style = false;
    for col in 0..cols {
        if let Some(cell) = screen.cell(row, col) {
            let style = StyleState::from_cell(cell);
            if !emitted_any_style || style != current {
                let sgr = style.to_sgr();
                if !sgr.is_empty() {
                    out.push_str(&sgr);
                    emitted_any_style = true;
                }
                current = style;
            }
            let contents = cell.contents();
            if contents.is_empty() {
                out.push(' ');
            } else {
                out.push_str(contents.as_ref());
            }
        } else {
            out.push(' ');
        }
    }
    let trimmed = out.trim_end().to_string();
    if emitted_any_style && !current.is_default() {
        format!("{trimmed}\x1b[0m")
    } else {
        trimmed
    }
}

fn chat_region_bounds(lines: &[String]) -> (usize, usize) {
    let start = lines
        .iter()
        .position(|l| l.trim_end().ends_with('╯'))
        .map(|i| i + 1)
        .unwrap_or(0);
    let end = lines
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, l)| is_divider(l))
        .map(|(i, _)| i)
        .unwrap_or(lines.len());
    (start, end)
}

fn is_divider(line: &str) -> bool {
    let t = line.trim();
    let dash_count = t.chars().filter(|c| *c == '─').count();
    let total = t.chars().count();
    dash_count >= 30 && dash_count * 5 >= total * 4
}

#[cfg(test)]
fn trim_to_chat_region(mut lines: Vec<String>) -> Vec<String> {
    if let Some(end_of_box) = lines.iter().position(|l| l.trim_end().ends_with('╯')) {
        lines.drain(..=end_of_box);
    }
    if let Some(divider) = lines.iter().position(|l| is_divider(l)) {
        lines.truncate(divider);
    }
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines
}

#[cfg(test)]
fn delta_lines(prev: &[String], next: &[String]) -> Vec<String> {
    let common = prev
        .iter()
        .zip(next.iter())
        .take_while(|(a, b)| a == b)
        .count();
    next[common..].to_vec()
}

/// A box-drawing table *bottom* border, e.g. `└─────┴─────┘`.
fn is_table_bottom_border(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('└') && t.ends_with('┘')
}

/// A box-drawing table *header rule* (the line between the header and the
/// body), e.g. `├─────┼─────┤`.
fn is_table_mid_rule(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('├') && t.ends_with('┤')
}

fn is_spinner_or_status(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    if line.chars().all(|c| c.is_whitespace() || c == '─') {
        return true;
    }
    let count = t.chars().count();
    let first = t.chars().next().unwrap();
    if count == 1 {
        return true;
    }
    if matches!(first, '✶' | '✻' | '✽' | '⠁' | '✢' | '✳' | '✺' | '✦') {
        return true;
    }
    if first == '⏵' {
        return true;
    }
    if matches!(first, '·' | '*') && count < 80 {
        return true;
    }
    if t.contains("tokens)")
        || t.contains("· ↓")
        || t.contains("· ↑")
        || t.contains("thinking)")
        || t.contains("thinking some more")
    {
        return true;
    }
    if first == '⎿'
        && (t.contains("Tip:")
            || t.contains("Press ")
            || t.contains("Use /")
            || t.contains("Running… ")
            || t.contains("Running…("))
    {
        return true;
    }
    if t.contains("(ctrl+") && t.contains(" to ") {
        return true;
    }
    false
}

fn line_source(line: &str) -> LineSource {
    let t = line.trim_start();
    if t.starts_with('●') || t.starts_with('⏺') {
        LineSource::AssistantText
    } else if t.starts_with('⎿') {
        LineSource::ToolOutput
    } else if t.starts_with("→") || t.contains("Tool") {
        LineSource::ToolHeader
    } else if t.starts_with('❯') {
        LineSource::PromptEcho
    } else if t.contains("Do you want to") || t.contains("Trust this folder") {
        LineSource::PermissionUi
    } else if t.starts_with("Claude Code") {
        LineSource::SystemLine
    } else {
        LineSource::Unknown
    }
}

fn activity_source(line: &str) -> ActivitySource {
    if line.trim().is_empty() {
        ActivitySource::EmptyLine
    } else if is_spinner_or_status(line) {
        ActivitySource::Spinner
    } else {
        ActivitySource::StatusChrome
    }
}

fn detect_permission_dialog(
    normalized: &str,
    plain_rows: &[String],
    frame_generation: u64,
) -> Option<PermissionDialog> {
    let kind = if normalized.contains("1.Yes")
        && (normalized.contains("trustthisfolder") || normalized.contains("projectyoucreated"))
    {
        PermissionDialogKind::TrustFolder
    } else if normalized.contains("1.Yes") && normalized.contains("Doyouwantto") {
        PermissionDialogKind::ToolUse
    } else if normalized.contains("ExitPlanMode") || normalized.contains("exitplanmode") {
        PermissionDialogKind::PlanExit
    } else if normalized.contains("AskUserQuestion") || normalized.contains("askuserquestion") {
        PermissionDialogKind::AskUserQuestion
    } else if normalized.contains("1.") && normalized.contains("2.") && normalized.contains("?") {
        PermissionDialogKind::Unknown
    } else {
        return None;
    };

    let visible: Vec<String> = plain_rows
        .iter()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !is_spinner_or_status(l))
        .collect();
    let title = visible
        .iter()
        .find(|l| {
            l.contains("Trust")
                || l.contains("Do you want")
                || l.contains("permission")
                || l.contains("Permission")
        })
        .cloned()
        .unwrap_or_else(|| match kind {
            PermissionDialogKind::TrustFolder => "Trust folder dialog".to_string(),
            PermissionDialogKind::ToolUse => "Tool permission dialog".to_string(),
            PermissionDialogKind::PlanExit => "Plan permission dialog".to_string(),
            PermissionDialogKind::AskUserQuestion => "Ask user question dialog".to_string(),
            PermissionDialogKind::PlanEnter | PermissionDialogKind::Unknown => {
                "Permission dialog".to_string()
            }
        });
    let body = visible.join("\n");
    let options: Vec<String> = visible
        .iter()
        .filter(|l| {
            l.starts_with("1.")
                || l.starts_with("2.")
                || l.starts_with("3.")
                || l.starts_with("[1]")
                || l.starts_with("[2]")
                || l.starts_with("[3]")
        })
        .cloned()
        .collect();
    let tool_name = visible
        .iter()
        .find_map(|l| l.split_once('(').map(|(head, _)| head.trim().to_string()))
        .filter(|s| !s.is_empty() && s.len() < 80);
    let path_or_command = visible
        .iter()
        .find(|l| l.contains('/') || l.contains("git ") || l.contains("cargo "))
        .cloned();

    Some(PermissionDialog {
        kind,
        title,
        body,
        tool_name,
        path_or_command,
        options,
        selected_option: Some(0),
        frame_generation,
    })
}

struct ControlSequenceObserver {
    alternate_screen: bool,
    bracketed_paste: bool,
    focus_events: bool,
    mouse_tracking: bool,
    synchronized_output: bool,
    cursor_visible: bool,
}

impl Default for ControlSequenceObserver {
    fn default() -> Self {
        Self {
            alternate_screen: false,
            bracketed_paste: false,
            focus_events: false,
            mouse_tracking: false,
            synchronized_output: false,
            cursor_visible: true,
        }
    }
}

impl ControlSequenceObserver {
    fn observe(&mut self, raw: &str) -> Vec<(TerminalMode, bool)> {
        let mut out = Vec::new();
        self.set_if_changed(
            raw,
            "\x1b[?1049h",
            "\x1b[?1049l",
            TerminalMode::AlternateScreen,
            &mut out,
        );
        self.set_if_changed(
            raw,
            "\x1b[?2004h",
            "\x1b[?2004l",
            TerminalMode::BracketedPaste,
            &mut out,
        );
        self.set_if_changed(
            raw,
            "\x1b[?1004h",
            "\x1b[?1004l",
            TerminalMode::FocusEvents,
            &mut out,
        );
        let mouse_on = raw.contains("\x1b[?1000h")
            || raw.contains("\x1b[?1002h")
            || raw.contains("\x1b[?1003h")
            || raw.contains("\x1b[?1006h");
        let mouse_off = raw.contains("\x1b[?1000l")
            || raw.contains("\x1b[?1002l")
            || raw.contains("\x1b[?1003l")
            || raw.contains("\x1b[?1006l");
        if mouse_on && !self.mouse_tracking {
            self.mouse_tracking = true;
            out.push((TerminalMode::MouseTracking, true));
        } else if mouse_off && self.mouse_tracking {
            self.mouse_tracking = false;
            out.push((TerminalMode::MouseTracking, false));
        }
        self.set_if_changed(
            raw,
            "\x1b[?2026h",
            "\x1b[?2026l",
            TerminalMode::SynchronizedOutput,
            &mut out,
        );
        self.set_if_changed(
            raw,
            "\x1b[?25h",
            "\x1b[?25l",
            TerminalMode::CursorVisible,
            &mut out,
        );
        out
    }

    fn set_if_changed(
        &mut self,
        raw: &str,
        on_seq: &str,
        off_seq: &str,
        mode: TerminalMode,
        out: &mut Vec<(TerminalMode, bool)>,
    ) {
        let slot = match mode {
            TerminalMode::AlternateScreen => &mut self.alternate_screen,
            TerminalMode::BracketedPaste => &mut self.bracketed_paste,
            TerminalMode::FocusEvents => &mut self.focus_events,
            TerminalMode::MouseTracking => &mut self.mouse_tracking,
            TerminalMode::SynchronizedOutput => &mut self.synchronized_output,
            TerminalMode::CursorVisible => &mut self.cursor_visible,
        };
        if raw.contains(on_seq) && !*slot {
            *slot = true;
            out.push((mode, true));
        } else if raw.contains(off_seq) && *slot {
            *slot = false;
            out.push((mode, false));
        }
    }
}

fn read_tui(
    reader: Box<dyn Read + Send>,
    tx: mpsc::Sender<Event>,
    rows: u16,
    cols: u16,
    terminal_state: Arc<Mutex<TerminalState>>,
    recorder: Option<Recorder>,
) {
    let ansi_regex = Regex::new(
        r"\x1b\[[0-9;?<>=]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[0-?@-Z\\^-~]",
    )
    .expect("valid ansi regex");

    let mut vt = vt100::Parser::new(rows, cols, 0);

    let mut accumulated = String::new();
    let mut active_confirmation: Option<&'static str> = None;
    let mut prompt_count: usize = 0;
    let mut last_response_count: usize = 0;
    let mut prev_chat_lines: Vec<String> = Vec::new();
    let mut frame_generation: u64 = 0;
    let mut observer = ControlSequenceObserver::default();
    let mut dropped_raw_events: u64 = 0;

    let idle_timeout = Duration::from_secs(5);
    let poll_interval = Duration::from_millis(100);
    let mut last_activity = Instant::now();
    let mut pending_prompt = false;
    let mut pending_response_count: usize = 0;
    let mut reader_error: Option<String> = None;

    // Decouple blocking reads from timeout-based prompt detection:
    // a reader thread sends raw chunks through a channel, and the
    // processing loop uses recv_timeout to check idle timers.
    let (data_tx, data_rx) = std::sync::mpsc::sync_channel::<Result<Vec<u8>, String>>(64);
    thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if data_tx.send(Ok(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = data_tx.send(Err(e.to_string()));
                    break;
                }
            }
        }
    });

    loop {
        match data_rx.recv_timeout(poll_interval) {
            Ok(Ok(chunk)) => {
                last_activity = Instant::now();
                frame_generation = frame_generation.saturating_add(1);
                if let Some(recorder) = &recorder {
                    recorder.record(RecordingEvent::Output {
                        at_ms: 0,
                        bytes: chunk.clone(),
                    });
                }
                vt.process(&chunk);
                let (cursor_row, cursor_col) = vt.screen().cursor_position();

                let raw = String::from_utf8_lossy(&chunk).to_string();
                for (mode, enabled) in observer.observe(&raw) {
                    if mode == TerminalMode::BracketedPaste {
                        if let Ok(mut state) = terminal_state.lock() {
                            state.bracketed_paste = enabled;
                        }
                    }
                    if mode == TerminalMode::AlternateScreen {
                        if let Ok(mut state) = terminal_state.lock() {
                            state.alternate_screen = enabled;
                        }
                        prev_chat_lines.clear();
                        frame_generation = frame_generation.saturating_add(1);
                    }
                    emit_critical(
                        &tx,
                        recorder.as_ref(),
                        Event::PtyModeChanged {
                            mode,
                            enabled,
                            frame_generation,
                        },
                    );
                }
                accumulated.push_str(&raw);
                if accumulated.len() > 30_000 {
                    let start = accumulated.ceil_char_boundary(20_000);
                    let cut = accumulated[start..]
                        .find('\n')
                        .map(|i| start + i + 1)
                        .unwrap_or(start);
                    let cut = accumulated.ceil_char_boundary(cut);
                    accumulated = accumulated[cut..].to_string();
                }
                let cleaned_chunk = ansi_regex.replace_all(&raw, "").to_string();
                let cleaned_acc = ansi_regex.replace_all(&accumulated, "").to_string();

                if cleaned_chunk.contains('●') {
                    if let Some(content) = cleaned_chunk.split('●').next_back() {
                        if !content.trim().is_empty() {
                            emit_critical(
                                &tx,
                                recorder.as_ref(),
                                Event::TuiAssistantMessage {
                                    content: content.to_string(),
                                },
                            );
                        }
                    }
                }

                let normalized = cleaned_acc.replace([' ', '\u{a0}'], "");
                let current_confirmation: Option<&'static str> = if normalized.contains("1.Yes")
                    && (normalized.contains("trustthisfolder")
                        || normalized.contains("projectyoucreated"))
                {
                    Some("Trust folder dialog")
                } else if normalized.contains("1.Yes") && normalized.contains("Doyouwantto") {
                    Some("Tool confirmation dialog")
                } else {
                    None
                };
                if current_confirmation != active_confirmation {
                    if let Some(msg) = current_confirmation {
                        let screen_ref = vt.screen();
                        let plain_rows: Vec<String> = screen_ref
                            .contents()
                            .lines()
                            .map(|s| s.to_string())
                            .collect();
                        if let Some(dialog) =
                            detect_permission_dialog(&normalized, &plain_rows, frame_generation)
                        {
                            emit_critical(
                                &tx,
                                recorder.as_ref(),
                                Event::PermissionDialogDetected { dialog },
                            );
                            emit_critical(
                                &tx,
                                recorder.as_ref(),
                                Event::SessionStateChanged {
                                    state: SessionState::WaitingForPermission,
                                },
                            );
                        }
                        emit_critical(
                            &tx,
                            recorder.as_ref(),
                            Event::TuiToolConfirmation {
                                message: msg.to_string(),
                            },
                        );
                    }
                    active_confirmation = current_confirmation;
                }

                let screen_ref = vt.screen();
                let contents = screen_ref.contents();
                let plain_rows: Vec<String> = contents.lines().map(|s| s.to_string()).collect();
                let ansi_rows: Vec<String> = (0..plain_rows.len() as u16)
                    .map(|r| ansi_row(screen_ref, r))
                    .collect();

                let (start, end) = chat_region_bounds(&plain_rows);

                let prompt_visible = plain_rows[end..]
                    .iter()
                    .any(|l| l.contains("❯\u{a0}") || l.trim_end() == "❯");
                let response_count = plain_rows[start..end]
                    .iter()
                    .filter(|l| l.contains('⏺') || l.contains('●'))
                    .count();
                let should_fire =
                    prompt_visible && (prompt_count == 0 || response_count > last_response_count);
                if should_fire {
                    if prompt_count == 0 {
                        emit_critical(
                            &tx,
                            recorder.as_ref(),
                            Event::SessionStateChanged {
                                state: SessionState::ReadyForInput,
                            },
                        );
                        emit_critical(
                            &tx,
                            recorder.as_ref(),
                            Event::PtyActivity {
                                source: ActivitySource::Prompt,
                            },
                        );
                        emit_critical(&tx, recorder.as_ref(), Event::TuiPrompt);
                        prompt_count += 1;
                        last_response_count = response_count;
                    } else if !pending_prompt {
                        pending_prompt = true;
                        pending_response_count = response_count;
                    }
                } else {
                    pending_prompt = false;
                }

                let mut chat_plain: Vec<String> = plain_rows[start..end].to_vec();
                let mut chat_ansi: Vec<String> = ansi_rows[start..end].to_vec();
                while chat_plain.first().is_some_and(|l| l.trim().is_empty()) {
                    chat_plain.remove(0);
                    chat_ansi.remove(0);
                }
                while chat_plain.last().is_some_and(|l| l.trim().is_empty()) {
                    chat_plain.pop();
                    chat_ansi.pop();
                }

                let common = prev_chat_lines
                    .iter()
                    .zip(chat_plain.iter())
                    .take_while(|(a, b)| a == b)
                    .count();
                let mut meaningful: Vec<(String, String)> = Vec::new();
                for idx in common..chat_plain.len() {
                    if is_spinner_or_status(&chat_plain[idx]) {
                        emit_lossy(
                            &tx,
                            recorder.as_ref(),
                            Event::PtyActivity {
                                source: activity_source(&chat_plain[idx]),
                            },
                        );
                        continue;
                    }
                    // Suppress the closing border of an *empty* table: a
                    // `└──┘` bottom rule immediately after a `├──┤` header
                    // rule means Claude rendered the table skeleton before any
                    // body rows arrived. Keep it out of this frame without
                    // poisoning later real table output.
                    if is_table_bottom_border(&chat_plain[idx])
                        && idx > 0
                        && is_table_mid_rule(&chat_plain[idx - 1])
                    {
                        emit_lossy(
                            &tx,
                            recorder.as_ref(),
                            Event::PtyActivity {
                                source: ActivitySource::Redraw,
                            },
                        );
                        continue;
                    }
                    if chat_plain[idx].trim().is_empty() {
                        emit_lossy(
                            &tx,
                            recorder.as_ref(),
                            Event::PtyActivity {
                                source: ActivitySource::EmptyLine,
                            },
                        );
                        continue;
                    }
                    meaningful.push((chat_plain[idx].clone(), chat_ansi[idx].clone()));
                }
                if !meaningful.is_empty() {
                    let (lines, lines_ansi): (Vec<String>, Vec<String>) =
                        meaningful.into_iter().unzip();
                    for l in &lines {
                        emit_critical(
                            &tx,
                            recorder.as_ref(),
                            Event::FilteredLine {
                                text: l.clone(),
                                ansi: lines_ansi
                                    .get(lines.iter().position(|x| x == l).unwrap_or(0))
                                    .cloned()
                                    .unwrap_or_else(|| l.clone()),
                                source: line_source(l),
                                frame_generation,
                            },
                        );
                    }
                    emit_critical(
                        &tx,
                        recorder.as_ref(),
                        Event::TuiScreen {
                            lines,
                            lines_ansi,
                            cursor_row,
                            cursor_col,
                        },
                    );
                }
                prev_chat_lines = chat_plain;

                if !cleaned_chunk.is_empty() {
                    match emit_raw_output_lossy(
                        &tx,
                        recorder.as_ref(),
                        Event::TuiOutput {
                            text: cleaned_chunk.clone(),
                            cursor_row,
                            cursor_col,
                        },
                    ) {
                        LossyEmit::Sent => {}
                        LossyEmit::Dropped => {
                            dropped_raw_events = dropped_raw_events.saturating_add(1);
                        }
                        LossyEmit::Closed => break,
                    }
                }
            }
            Ok(Err(err_msg)) => {
                reader_error = Some(err_msg);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if pending_prompt && last_activity.elapsed() >= idle_timeout {
                    emit_critical(
                        &tx,
                        recorder.as_ref(),
                        Event::SessionStateChanged {
                            state: SessionState::ReadyForInput,
                        },
                    );
                    emit_critical(&tx, recorder.as_ref(), Event::TuiPrompt);
                    prompt_count += 1;
                    last_response_count = pending_response_count;
                    pending_prompt = false;
                }
                if reader_error.is_some() && last_activity.elapsed() >= idle_timeout {
                    emit_critical(
                        &tx,
                        recorder.as_ref(),
                        Event::LibError {
                            message: reader_error.take().unwrap(),
                        },
                    );
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if pending_prompt {
                    emit_critical(&tx, recorder.as_ref(), Event::TuiPrompt);
                }
                if let Some(msg) = reader_error {
                    emit_critical(&tx, recorder.as_ref(), Event::LibError { message: msg });
                } else {
                    if dropped_raw_events > 0 {
                        emit_critical(
                            &tx,
                            recorder.as_ref(),
                            Event::ParserWarning {
                                message: format!("dropped {dropped_raw_events} raw PTY events"),
                            },
                        );
                    }
                    emit_critical(
                        &tx,
                        recorder.as_ref(),
                        Event::SessionStateChanged {
                            state: SessionState::Exited,
                        },
                    );
                    emit_critical(
                        &tx,
                        recorder.as_ref(),
                        Event::ProcessExited { status: None },
                    );
                    emit_critical(&tx, recorder.as_ref(), Event::LibDone);
                }
                break;
            }
        }
    }
}

struct ChunkedReader {
    chunks: VecDeque<Vec<u8>>,
}

impl ChunkedReader {
    fn new(chunks: Vec<Vec<u8>>) -> Self {
        Self {
            chunks: chunks.into(),
        }
    }
}

impl Read for ChunkedReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let Some(chunk) = self.chunks.pop_front() else {
            return Ok(0);
        };
        let n = chunk.len().min(buf.len());
        buf[..n].copy_from_slice(&chunk[..n]);
        if n < chunk.len() {
            let mut rest = chunk;
            rest.drain(..n);
            self.chunks.push_front(rest);
        }
        Ok(n)
    }
}

pub fn replay_chunks(rows: u16, cols: u16, chunks: Vec<Vec<u8>>) -> Vec<Event> {
    let reader: Box<dyn Read + Send> = Box::new(ChunkedReader::new(chunks));
    let (tx, mut rx) = mpsc::channel::<Event>(1024);
    let terminal_state = Arc::new(Mutex::new(TerminalState::default()));
    let handle = thread::spawn(move || read_tui(reader, tx, rows, cols, terminal_state, None));
    let mut events = Vec::new();
    while let Some(event) = rx.blocking_recv() {
        events.push(event);
    }
    let _ = handle.join();
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tokio::runtime::Runtime;

    #[test]
    fn trim_to_chat_region_drops_header_and_footer() {
        let lines = vec![
            "╭─────────── Claude Code v2.1.143 ─────────────╮".to_string(),
            "│ Welcome back!                                │".to_string(),
            "╰──────────────────────────────────────────────╯".to_string(),
            "".to_string(),
            "  Claudio stood at the edge of the pitch".to_string(),
            "  He raised both arms".to_string(),
            "".to_string(),
            "────────────────────────────────────────────────".to_string(),
            "❯ ".to_string(),
            "────────────────────────────────────────────────".to_string(),
            "  auto mode on (shift+tab to cycle)".to_string(),
        ];
        let trimmed = trim_to_chat_region(lines);
        assert_eq!(
            trimmed,
            vec![
                "  Claudio stood at the edge of the pitch".to_string(),
                "  He raised both arms".to_string(),
            ],
        );
    }

    #[test]
    fn trim_to_chat_region_handles_divider_with_slash_command_suffix() {
        let lines = vec![
            "  some chat".to_string(),
            "─────────────────────────────────────────────────────────────────────────────────────────────────────────────· /effort".to_string(),
            "  ⏵⏵ bypass permissions on (shift+tab to cycle) · esc to interrupt".to_string(),
        ];
        assert_eq!(trim_to_chat_region(lines), vec!["  some chat".to_string()]);
    }

    #[test]
    fn trim_to_chat_region_handles_missing_header() {
        let lines = vec![
            "".to_string(),
            "  some paragraph".to_string(),
            "────────────────────────────────────────────────".to_string(),
            "❯  ".to_string(),
        ];
        assert_eq!(
            trim_to_chat_region(lines),
            vec!["  some paragraph".to_string()]
        );
    }

    #[test]
    fn delta_lines_returns_only_new_suffix() {
        let prev: Vec<String> = ["a", "b", "c"].into_iter().map(String::from).collect();
        let next: Vec<String> = ["a", "b", "c", "d", "e"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(
            delta_lines(&prev, &next),
            vec!["d".to_string(), "e".to_string()]
        );
    }

    #[test]
    fn delta_lines_handles_divergence() {
        let prev: Vec<String> = ["a", "b", "c"].into_iter().map(String::from).collect();
        let next: Vec<String> = ["a", "X", "c"].into_iter().map(String::from).collect();
        assert_eq!(
            delta_lines(&prev, &next),
            vec!["X".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn read_tui_reports_cursor_position() {
        let cursor = Cursor::new(b"abc\r\nxy".to_vec());

        let rt = Runtime::new().unwrap();
        let _g = rt.enter();
        let (tx, mut rx) = mpsc::channel::<Event>(16);
        let handle = std::thread::spawn(move || {
            let reader: Box<dyn Read + Send> = Box::new(cursor);
            read_tui(
                reader,
                tx,
                24,
                80,
                Arc::new(Mutex::new(TerminalState::default())),
                None,
            );
        });
        let mut last_row = 0;
        let mut last_col = 0;
        let mut saw_output = false;
        rt.block_on(async {
            while let Some(evt) = rx.recv().await {
                if let Event::TuiOutput {
                    cursor_row,
                    cursor_col,
                    ..
                } = evt
                {
                    last_row = cursor_row;
                    last_col = cursor_col;
                    saw_output = true;
                }
            }
        });
        handle.join().unwrap();
        assert!(saw_output, "expected at least one TuiOutput event");
        assert_eq!(last_row, 1);
        assert_eq!(last_col, 2);
    }

    #[test]
    fn is_spinner_or_status_recognizes_real_examples() {
        for s in [
            "*",
            "✶",
            "✻ Cerebrating…",
            "✽ Cerebrating… (1s · ↓ 1 tokens)",
            "(2s · thinking)",
            "✶ Befuddling… (2s · ↓ 34 tokens)",
            "✻ Ionizing… (40s · ↑ 5.3k tokens · thinking some more)",
            "✳ Thundering… ",
            "  ⎿  Tip: Use /theme to change the color theme",
            "  ⎿  Press ? for help",
            "  ⎿  Running… (9s)",
            "     … +114 lines (ctrl+o to expand)",
            "… +3 lines (ctrl+o to expand)",
            "     (ctrl+b ctrl+b (twice) to run in background)",
            "                                                                                                               ─────────",
            "  ⏵⏵ bypass permissions on (shift+tab to cycle) · esc to interrupt                                   ◉ xhigh · /effort",
        ] {
            assert!(is_spinner_or_status(s), "{s:?} should be spinner/status");
        }
        for s in [
            "● Hi",
            "❯ say hi in one word",
            "  Claudio stood at the edge of the pitch in Bremen",
            "abc",
            "program Sum;",
        ] {
            assert!(!is_spinner_or_status(s), "{s:?} should NOT be spinner");
        }
    }

    #[test]
    fn prompt_bytes_wraps_bracketed_paste_and_submits_once() {
        let prompt = "line one\nline two\nline three";
        let bytes = prompt_bytes(prompt, true);
        assert!(bytes.starts_with(b"\x1b[200~"));
        assert!(bytes.ends_with(b"\x1b[201~\r"));
        assert_eq!(bytes.iter().filter(|b| **b == b'\r').count(), 1);
        assert!(String::from_utf8_lossy(&bytes).contains(prompt));
    }

    #[test]
    fn prompt_bytes_plain_fallback_submits_once() {
        let prompt = "line one\nline two";
        let bytes = prompt_bytes(prompt, false);
        assert_eq!(bytes, b"line one\nline two\r");
    }

    #[test]
    fn typed_events_serialize() {
        let event = Event::FilteredLine {
            text: "hello".to_string(),
            ansi: "\x1b[32mhello\x1b[0m".to_string(),
            source: LineSource::AssistantText,
            frame_generation: 7,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("filtered_line"));
        assert!(json.contains("assistant_text"));

        let dialog = Event::PermissionDialogDetected {
            dialog: PermissionDialog {
                kind: PermissionDialogKind::ToolUse,
                title: "Do you want to run Bash?".to_string(),
                body: "Do you want to run Bash?".to_string(),
                tool_name: Some("Bash".to_string()),
                path_or_command: Some("git status".to_string()),
                options: vec!["1. Yes".to_string(), "2. No".to_string()],
                selected_option: Some(0),
                frame_generation: 3,
            },
        };
        let json = serde_json::to_string(&dialog).unwrap();
        assert!(json.contains("permission_dialog_detected"));
        assert!(json.contains("tool_use"));
    }

    #[test]
    fn permission_dialog_detection_classifies_known_and_unknown_dialogs() {
        let trust = detect_permission_dialog(
            "1.Yestrustthisfolder",
            &[
                "Trust this folder?".to_string(),
                "1. Yes".to_string(),
                "2. No".to_string(),
            ],
            1,
        )
        .unwrap();
        assert_eq!(trust.kind, PermissionDialogKind::TrustFolder);

        let tool = detect_permission_dialog(
            "Doyouwantto1.Yes2.No",
            &[
                "Do you want to run Bash(git status)?".to_string(),
                "git status".to_string(),
                "1. Yes".to_string(),
                "2. No".to_string(),
            ],
            2,
        )
        .unwrap();
        assert_eq!(tool.kind, PermissionDialogKind::ToolUse);
        assert_eq!(tool.selected_option, Some(0));

        let unknown = detect_permission_dialog(
            "Proceed?1.Continue2.Cancel",
            &[
                "Proceed?".to_string(),
                "1. Continue".to_string(),
                "2. Cancel".to_string(),
            ],
            3,
        )
        .unwrap();
        assert_eq!(unknown.kind, PermissionDialogKind::Unknown);
    }

    #[test]
    fn control_sequence_observer_tracks_modes() {
        let mut observer = ControlSequenceObserver::default();
        let changes = observer.observe("\x1b[?2004h\x1b[?1049h\x1b[?25l");
        assert!(changes.contains(&(TerminalMode::BracketedPaste, true)));
        assert!(changes.contains(&(TerminalMode::AlternateScreen, true)));
        assert!(changes.contains(&(TerminalMode::CursorVisible, false)));

        let changes = observer.observe("\x1b[?2004l\x1b[?1049l");
        assert!(changes.contains(&(TerminalMode::BracketedPaste, false)));
        assert!(changes.contains(&(TerminalMode::AlternateScreen, false)));
    }

    #[test]
    fn replay_chunks_emits_filtered_lines_without_spawning_claude() {
        let events = replay_chunks(
            24,
            80,
            vec![b"same\r\nsame\r\n\xe2\x97\x8f done\r\n".to_vec()],
        );
        let lines: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                Event::FilteredLine { text, .. } => Some(text.trim().to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(
            lines.iter().filter(|line| line.as_str() == "same").count(),
            2
        );
        assert!(lines.iter().any(|line| line.contains("done")));
    }

    #[test]
    fn fake_claude_replay_fixture_covers_prompt_usage_unicode_and_exit() {
        let events = replay_chunks(
            24,
            100,
            vec![
                "╭ Claude Code ╮\r\n╰─────────────╯\r\n\r\n────────────────────────────────────────\r\n❯ "
                    .as_bytes()
                    .to_vec(),
                "\x1b[2J\x1b[H╭ Claude Code ╮\r\n╰─────────────╯\r\nYou are out of usage for today\r\n表 emoji 😀\r\n────────────────────────────────────────\r\n❯ "
                    .as_bytes()
                    .to_vec(),
            ],
        );
        assert!(events.iter().any(|e| matches!(e, Event::TuiPrompt)));
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::ProcessExited { .. })));
        assert!(events.iter().any(|e| matches!(e, Event::LibDone)));
        let lines: Vec<&str> = events
            .iter()
            .filter_map(|event| match event {
                Event::FilteredLine { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(lines.iter().any(|line| line.contains("out of usage")));
        assert!(lines.iter().any(|line| line.contains('表')));
    }

    #[test]
    fn read_tui_emits_only_when_content_delta_has_real_lines() {
        let reader = ChunkedReader {
            chunks: vec![
                b"first line\r\n".to_vec(),
                b"\r\x1b[K\xe2\x9c\xb6".to_vec(),
                b"\r\x1b[Ksecond line\r\n".to_vec(),
            ]
            .into(),
        };
        let rt = Runtime::new().unwrap();
        let _g = rt.enter();
        let (tx, mut rx) = mpsc::channel::<Event>(64);
        let handle = std::thread::spawn(move || {
            let reader: Box<dyn Read + Send> = Box::new(reader);
            read_tui(
                reader,
                tx,
                24,
                80,
                Arc::new(Mutex::new(TerminalState::default())),
                None,
            );
        });
        let mut screens: Vec<Vec<String>> = Vec::new();
        rt.block_on(async {
            while let Some(evt) = rx.recv().await {
                if let Event::TuiScreen { lines, .. } = evt {
                    screens.push(lines);
                }
            }
        });
        handle.join().unwrap();
        assert_eq!(
            screens,
            vec![
                vec!["first line".to_string()],
                vec!["second line".to_string()],
            ],
        );
    }

    #[test]
    fn read_tui_keeps_table_body_after_empty_skeleton() {
        // Claude's TUI first paints the table skeleton (header + rule + a
        // closing border, no body), then repaints it with the body rows. The
        // empty closing border must NOT be emitted, otherwise it (a) shows an
        // empty table and (b) poisons the dedup set so the real footer is
        // dropped once the rows arrive.
        let reader = ChunkedReader {
            chunks: vec![
                concat!(
                    "┌─────────────┬────────┐\r\n",
                    "│ Requirement │ Status │\r\n",
                    "├─────────────┼────────┤\r\n",
                    "└─────────────┴────────┘\r\n",
                )
                .as_bytes()
                .to_vec(),
                concat!(
                    "\x1b[2J\x1b[H",
                    "┌─────────────┬────────┐\r\n",
                    "│ Requirement │ Status │\r\n",
                    "├─────────────┼────────┤\r\n",
                    "│ ADR present │ ✓      │\r\n",
                    "│ Title       │ ✓      │\r\n",
                    "└─────────────┴────────┘\r\n",
                )
                .as_bytes()
                .to_vec(),
            ]
            .into(),
        };
        let rt = Runtime::new().unwrap();
        let _g = rt.enter();
        let (tx, mut rx) = mpsc::channel::<Event>(64);
        let handle = std::thread::spawn(move || {
            let reader: Box<dyn Read + Send> = Box::new(reader);
            read_tui(
                reader,
                tx,
                24,
                80,
                Arc::new(Mutex::new(TerminalState::default())),
                None,
            );
        });
        let mut screens: Vec<Vec<String>> = Vec::new();
        rt.block_on(async {
            while let Some(evt) = rx.recv().await {
                if let Event::TuiScreen { lines, .. } = evt {
                    screens.push(lines);
                }
            }
        });
        handle.join().unwrap();
        let all_lines: Vec<String> = screens.into_iter().flatten().collect();

        // The body rows must survive.
        assert!(
            all_lines.iter().any(|l| l.contains("ADR present")),
            "table body row missing; saw {all_lines:?}"
        );
        assert!(
            all_lines.iter().any(|l| l.contains("Title")),
            "table body row missing; saw {all_lines:?}"
        );
        // Exactly one closing border, and it must come *after* the body rows.
        let footer_idxs: Vec<usize> = all_lines
            .iter()
            .enumerate()
            .filter(|(_, l)| is_table_bottom_border(l))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            footer_idxs.len(),
            1,
            "expected one closing border; saw {all_lines:?}"
        );
        let body_idx = all_lines
            .iter()
            .position(|l| l.contains("ADR present"))
            .unwrap();
        assert!(
            footer_idxs[0] > body_idx,
            "closing border must follow the body; saw {all_lines:?}"
        );
    }

    #[test]
    fn read_tui_preserves_repeated_lines_in_different_positions() {
        let reader = ChunkedReader {
            chunks: vec![
                b"alpha\r\nbeta\r\n".to_vec(),
                b"\x1b[2J\x1b[H".to_vec(),
                b"beta\r\ngamma\r\n".to_vec(),
            ]
            .into(),
        };
        let rt = Runtime::new().unwrap();
        let _g = rt.enter();
        let (tx, mut rx) = mpsc::channel::<Event>(64);
        let handle = std::thread::spawn(move || {
            let reader: Box<dyn Read + Send> = Box::new(reader);
            read_tui(
                reader,
                tx,
                24,
                80,
                Arc::new(Mutex::new(TerminalState::default())),
                None,
            );
        });
        let mut screens: Vec<Vec<String>> = Vec::new();
        rt.block_on(async {
            while let Some(evt) = rx.recv().await {
                if let Event::TuiScreen { lines, .. } = evt {
                    screens.push(lines);
                }
            }
        });
        handle.join().unwrap();
        let all_lines: Vec<&String> = screens.iter().flatten().collect();
        assert_eq!(
            all_lines.iter().filter(|l| l.trim() == "beta").count(),
            2,
            "legitimate repeated lines must be preserved; saw {all_lines:?}"
        );
        assert!(
            all_lines.iter().any(|l| l.contains("gamma")),
            "gamma should be emitted; saw {all_lines:?}"
        );
    }

    #[test]
    fn read_tui_handles_multibyte_at_accumulator_boundary() {
        let multibyte: String = "─".repeat(20_000);
        let cursor = Cursor::new(multibyte.into_bytes());

        let rt = Runtime::new().unwrap();
        let _g = rt.enter();
        let (tx, mut rx) = mpsc::channel::<Event>(8192);
        let handle = std::thread::spawn(move || {
            let reader: Box<dyn Read + Send> = Box::new(cursor);
            read_tui(
                reader,
                tx,
                40,
                120,
                Arc::new(Mutex::new(TerminalState::default())),
                None,
            );
        });
        rt.block_on(async { while rx.recv().await.is_some() {} });
        handle.join().expect("read_tui must not panic");
    }
}
