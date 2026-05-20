use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{
    native_pty_system, Child as PtyChild, CommandBuilder, MasterPty, PtyPair, PtySize,
};
use regex::Regex;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::builder::ResolvedSpec;
use crate::event::{Event, PollEvent};
use crate::Error;

pub struct Session {
    #[allow(dead_code)]
    master: Box<dyn MasterPty + Send>,
    child: Mutex<Box<dyn PtyChild + Send + Sync>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    events: Option<mpsc::Receiver<Event>>,
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
        spawn_reader(reader, tx, spec.rows, spec.cols);

        Ok(Self {
            master,
            child: Mutex::new(child),
            writer: Arc::new(Mutex::new(writer)),
            events: Some(rx),
        })
    }

    /// Takes the event stream. Can only be called once per session.
    pub fn take_events(&mut self) -> Option<ReceiverStream<Event>> {
        self.events.take().map(ReceiverStream::new)
    }

    /// Writes `text\r` to the PTY — the TUI treats `\r` as "submit".
    pub fn send_line(&self, text: &str) -> Result<(), Error> {
        let mut buf = text.as_bytes().to_vec();
        buf.push(b'\r');
        self.write_raw(&buf)
    }

    pub fn send_user_message(&self, text: &str) -> Result<(), Error> {
        self.send_line(text)
    }

    pub fn write_raw(&self, data: &[u8]) -> Result<(), Error> {
        let mut w = self
            .writer
            .lock()
            .map_err(|_| Error::Io("writer poisoned".into()))?;
        w.write_all(data).map_err(|e| Error::Io(e.to_string()))?;
        w.flush().map_err(|e| Error::Io(e.to_string()))?;
        Ok(())
    }

    pub fn kill(&self) -> Result<(), Error> {
        let mut c = self
            .child
            .lock()
            .map_err(|_| Error::Io("child poisoned".into()))?;
        c.kill().map_err(|e| Error::Io(e.to_string()))?;
        Ok(())
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

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}

fn spawn_reader(reader: Box<dyn Read + Send>, tx: mpsc::Sender<Event>, rows: u16, cols: u16) {
    thread::spawn(move || read_tui(reader, tx, rows, cols));
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

fn read_tui(reader: Box<dyn Read + Send>, tx: mpsc::Sender<Event>, rows: u16, cols: u16) {
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
    // Dedup window for emitted chat lines. Bounded with FIFO eviction so
    // long sessions cannot grow it without limit, while still preventing
    // re-emission on transient screen clears (e.g. `\x1b[2J`) where the
    // viewport briefly empties before the same content reappears.
    const EMITTED_LINES_CAP: usize = 4096;
    let mut emitted_lines: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut emitted_order: std::collections::VecDeque<String> = std::collections::VecDeque::new();

    let idle_timeout = std::time::Duration::from_secs(5);
    let poll_interval = std::time::Duration::from_millis(100);
    let mut last_activity = std::time::Instant::now();
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
                last_activity = std::time::Instant::now();
                vt.process(&chunk);
                let (cursor_row, cursor_col) = vt.screen().cursor_position();

                let raw = String::from_utf8_lossy(&chunk).to_string();
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
                            let _ = tx.blocking_send(Event::TuiAssistantMessage {
                                content: content.to_string(),
                            });
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
                        let _ = tx.blocking_send(Event::TuiToolConfirmation {
                            message: msg.to_string(),
                        });
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
                        let _ = tx.blocking_send(Event::TuiPrompt);
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
                let new_plain: Vec<String> = chat_plain[common..].to_vec();
                let new_ansi: Vec<String> = chat_ansi[common..].to_vec();
                let meaningful: Vec<(String, String)> = new_plain
                    .into_iter()
                    .zip(new_ansi)
                    .filter(|(p, _)| !is_spinner_or_status(p))
                    .filter(|(p, _)| p.trim().is_empty() || !emitted_lines.contains(p))
                    .collect();
                if !meaningful.is_empty() {
                    let (lines, lines_ansi): (Vec<String>, Vec<String>) =
                        meaningful.into_iter().unzip();
                    for l in &lines {
                        if !l.trim().is_empty() && emitted_lines.insert(l.clone()) {
                            emitted_order.push_back(l.clone());
                            while emitted_order.len() > EMITTED_LINES_CAP {
                                if let Some(oldest) = emitted_order.pop_front() {
                                    emitted_lines.remove(&oldest);
                                }
                            }
                        }
                    }
                    let _ = tx.blocking_send(Event::TuiScreen {
                        lines,
                        lines_ansi,
                        cursor_row,
                        cursor_col,
                    });
                }
                prev_chat_lines = chat_plain;

                if !cleaned_chunk.is_empty()
                    && tx
                        .blocking_send(Event::TuiOutput {
                            text: cleaned_chunk,
                            cursor_row,
                            cursor_col,
                        })
                        .is_err()
                {
                    break;
                }
            }
            Ok(Err(err_msg)) => {
                reader_error = Some(err_msg);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if pending_prompt && last_activity.elapsed() >= idle_timeout {
                    let _ = tx.blocking_send(Event::TuiPrompt);
                    prompt_count += 1;
                    last_response_count = pending_response_count;
                    pending_prompt = false;
                }
                if reader_error.is_some() && last_activity.elapsed() >= idle_timeout {
                    let _ = tx.blocking_send(Event::LibError {
                        message: reader_error.take().unwrap(),
                    });
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if pending_prompt {
                    let _ = tx.blocking_send(Event::TuiPrompt);
                }
                if let Some(msg) = reader_error {
                    let _ = tx.blocking_send(Event::LibError { message: msg });
                } else {
                    let _ = tx.blocking_send(Event::LibDone);
                }
                break;
            }
        }
    }
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

    struct ChunkedReader {
        chunks: std::collections::VecDeque<Vec<u8>>,
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

    #[test]
    fn read_tui_reports_cursor_position() {
        let cursor = Cursor::new(b"abc\r\nxy".to_vec());

        let rt = Runtime::new().unwrap();
        let _g = rt.enter();
        let (tx, mut rx) = mpsc::channel::<Event>(16);
        let handle = std::thread::spawn(move || {
            let reader: Box<dyn Read + Send> = Box::new(cursor);
            read_tui(reader, tx, 24, 80);
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
            read_tui(reader, tx, 24, 80);
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
    fn read_tui_dedupes_lines_already_emitted() {
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
            read_tui(reader, tx, 24, 80);
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
        let unique: std::collections::HashSet<&String> = all_lines.iter().copied().collect();
        assert_eq!(
            all_lines.len(),
            unique.len(),
            "lines must not be emitted twice across TuiScreen events; saw {all_lines:?}"
        );
        assert!(
            unique.iter().any(|l| l.contains("gamma")),
            "gamma should be emitted; saw {unique:?}"
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
            read_tui(reader, tx, 40, 120);
        });
        rt.block_on(async { while rx.recv().await.is_some() {} });
        handle.join().expect("read_tui must not panic");
    }
}
