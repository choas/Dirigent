//! A single ACP agent subprocess, spoken to as a client over newline-delimited
//! JSON-RPC 2.0 on stdin/stdout.
//!
//! A reader thread owns the agent's stdout and forwards parsed messages over a
//! channel, so the calling thread can poll the cancel flag while a turn is in
//! flight instead of blocking on a read that may never return.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::types::*;

/// How often the call loop wakes up to check the cancel flag.
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// How long a cancelled turn may take to wind down before the agent is killed.
const CANCEL_GRACE: Duration = Duration::from_secs(5);
/// How long a closed-stdin agent gets to exit on its own before SIGTERM.
const EXIT_GRACE: Duration = Duration::from_millis(500);
/// Bytes of agent stderr kept for error messages.
const STDERR_TAIL_BYTES: usize = 4096;

/// What the agent may do to the project during a run.
#[derive(Debug, Clone, Copy)]
pub(super) struct ClientPolicy {
    /// Serve `fs/write_text_file` and approve edit/execute permission requests.
    /// False for one-shot analysis prompts (split cue, workflow, theme).
    pub allow_writes: bool,
    /// Approve permission requests without asking. Mirrors the global
    /// "skip permissions" setting the other providers honour.
    pub auto_approve: bool,
}

enum ReaderEvent {
    Message(IncomingMessage),
    Eof,
    Failed(std::io::Error),
}

/// Per-tool-call state; `tool_call_update`s only carry changed fields.
#[derive(Default)]
struct ToolState {
    title: String,
    kind: Option<ToolKind>,
    finished: bool,
}

/// Everything accumulated while the agent works on a prompt.
pub(super) struct Turn<'a> {
    on_log: &'a mut dyn FnMut(&str),
    session_id: Option<String>,
    cancelled: bool,
    at_line_start: bool,
    pub text: String,
    pub edited_files: Vec<String>,
    pub tool_calls_completed: u64,
    tools: HashMap<String, ToolState>,
    last_plan: String,
}

impl<'a> Turn<'a> {
    pub fn new(on_log: &'a mut dyn FnMut(&str)) -> Self {
        Self {
            on_log,
            session_id: None,
            cancelled: false,
            at_line_start: true,
            text: String::new(),
            edited_files: Vec::new(),
            tool_calls_completed: 0,
            tools: HashMap::new(),
            last_plan: String::new(),
        }
    }

    /// Stream agent text verbatim.
    fn log_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        (self.on_log)(text);
        self.at_line_start = text.ends_with('\n');
    }

    /// Log a status line, starting it on a fresh line if agent text was
    /// mid-sentence.
    pub fn log_line(&mut self, line: &str) {
        if !self.at_line_start {
            (self.on_log)("\n");
        }
        (self.on_log)(line);
        (self.on_log)("\n");
        self.at_line_start = true;
    }

    fn note_edit(&mut self, path: &str) {
        if !path.is_empty() && !self.edited_files.iter().any(|p| p == path) {
            self.edited_files.push(path.to_string());
        }
    }
}

pub(super) struct AcpConnection {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: mpsc::Receiver<ReaderEvent>,
    stderr_tail: Arc<Mutex<String>>,
    root: PathBuf,
    policy: ClientPolicy,
    /// Names from the `initialize` result, for the auth-required error.
    auth_methods: Vec<String>,
    next_id: u64,
}

impl AcpConnection {
    /// Spawn the agent and start the stdout/stderr reader threads. `cmd` must
    /// already carry program, args, env and working directory.
    pub fn spawn(mut cmd: Command, root: &Path, policy: ClientPolicy) -> Result<Self, AcpError> {
        let root = std::fs::canonicalize(root).map_err(AcpError::Io)?;
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Own process group so shutdown also reaps the agent's children
        // (adapters such as claude-agent-acp spawn the real CLI underneath).
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd.spawn().map_err(AcpError::SpawnFailed)?;
        let stdin = child.stdin.take().expect("stdin must be piped");
        let stdout = child.stdout.take().expect("stdout must be piped");
        let stderr = child.stderr.take().expect("stderr must be piped");

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || read_messages(stdout, tx));
        let stderr_tail = Arc::new(Mutex::new(String::new()));
        let tail = Arc::clone(&stderr_tail);
        std::thread::spawn(move || collect_stderr_tail(stderr, tail));

        Ok(Self {
            child,
            stdin: Some(stdin),
            rx,
            stderr_tail,
            root,
            policy,
            auth_methods: Vec::new(),
            next_id: 1,
        })
    }

    pub fn initialize(&mut self, turn: &mut Turn, cancel: &AtomicBool) -> Result<(), AcpError> {
        let params = InitializeParams {
            protocol_version: PROTOCOL_VERSION,
            client_capabilities: ClientCapabilities {
                fs: FsCapabilities {
                    read_text_file: true,
                    write_text_file: self.policy.allow_writes,
                },
                terminal: false,
            },
            client_info: Implementation {
                name: "dirigent".to_string(),
                title: Some("Dirigent".to_string()),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };
        let result = self.call("initialize", to_value(params), turn, cancel)?;
        let init: InitializeResult = serde_json::from_value(result)
            .map_err(|e| AcpError::Protocol(format!("invalid initialize result: {e}")))?;
        if let Some(version) = init.protocol_version.filter(|v| *v != PROTOCOL_VERSION) {
            return Err(AcpError::Protocol(format!(
                "agent speaks ACP v{version}, Dirigent supports v{PROTOCOL_VERSION}"
            )));
        }
        let agent = init
            .agent_info
            .map(|info| {
                let name = info.title.filter(|t| !t.is_empty()).unwrap_or(info.name);
                if info.version.is_empty() {
                    name
                } else {
                    format!("{name} {}", info.version)
                }
            })
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "ACP agent".to_string());
        turn.log_line(&format!("[ACP: {agent}]"));
        self.auth_methods = init
            .auth_methods
            .into_iter()
            .map(|m| m.name)
            .filter(|n| !n.is_empty())
            .collect();
        Ok(())
    }

    pub fn new_session(&mut self, turn: &mut Turn, cancel: &AtomicBool) -> Result<(), AcpError> {
        let params = SessionNewParams {
            cwd: self.root.to_string_lossy().to_string(),
            mcp_servers: Vec::new(),
        };
        let result = match self.call("session/new", to_value(params), turn, cancel) {
            Err(AcpError::Agent { code, .. }) if code == AUTH_REQUIRED_CODE => {
                let methods = if self.auth_methods.is_empty() {
                    "no method advertised".to_string()
                } else {
                    self.auth_methods.join(", ")
                };
                return Err(AcpError::AuthRequired(methods));
            }
            other => other?,
        };
        let session: SessionNewResult = serde_json::from_value(result)
            .map_err(|e| AcpError::Protocol(format!("invalid session/new result: {e}")))?;
        turn.session_id = Some(session.session_id);
        Ok(())
    }

    pub fn prompt(
        &mut self,
        text: &str,
        turn: &mut Turn,
        cancel: &AtomicBool,
    ) -> Result<StopReason, AcpError> {
        let session_id = turn
            .session_id
            .clone()
            .ok_or_else(|| AcpError::Protocol("prompt sent before session/new".into()))?;
        let params = SessionPromptParams {
            session_id,
            prompt: vec![PromptContent::Text {
                text: text.to_string(),
            }],
        };
        let result = self.call("session/prompt", to_value(params), turn, cancel)?;
        let result: SessionPromptResult = serde_json::from_value(result)
            .map_err(|e| AcpError::Protocol(format!("invalid session/prompt result: {e}")))?;
        Ok(result.stop_reason)
    }

    /// Send a request and pump messages until its response arrives, serving
    /// agent requests and session updates in the meantime.
    fn call(
        &mut self,
        method: &'static str,
        params: Value,
        turn: &mut Turn,
        cancel: &AtomicBool,
    ) -> Result<Value, AcpError> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;

        let mut cancel_sent_at: Option<Instant> = None;
        loop {
            if cancel.load(Ordering::Relaxed) && cancel_sent_at.is_none() {
                // Before a session exists there is nothing to wind down.
                let Some(session_id) = turn.session_id.clone() else {
                    return Err(AcpError::Cancelled);
                };
                turn.cancelled = true;
                let params = to_value(SessionCancelParams { session_id });
                self.send(
                    &json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": params }),
                )?;
                cancel_sent_at = Some(Instant::now());
            }
            if cancel_sent_at.is_some_and(|t| t.elapsed() > CANCEL_GRACE) {
                return Err(AcpError::Cancelled);
            }

            let msg = match self.rx.recv_timeout(POLL_INTERVAL) {
                Ok(ReaderEvent::Message(msg)) => msg,
                Ok(ReaderEvent::Failed(e)) => return Err(AcpError::Io(e)),
                Ok(ReaderEvent::Eof) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(AcpError::AgentExited(self.stderr_tail()));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
            };

            if msg.method.is_none() {
                if msg.id.as_ref().and_then(Value::as_u64) != Some(id) {
                    continue; // stale response to an earlier request
                }
                if let Some(err) = msg.error {
                    return Err(AcpError::Agent {
                        method,
                        code: err.code,
                        message: err.message,
                    });
                }
                if turn.cancelled {
                    return Err(AcpError::Cancelled);
                }
                return Ok(msg.result.unwrap_or(Value::Null));
            }
            self.dispatch(msg, turn)?;
        }
    }

    fn dispatch(&mut self, msg: IncomingMessage, turn: &mut Turn) -> Result<(), AcpError> {
        let method = msg.method.unwrap_or_default();
        let params = msg.params.unwrap_or(Value::Null);
        match msg.id {
            Some(id) => {
                let reply = match self.serve_request(&method, params, turn) {
                    Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    Err((code, message)) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": code, "message": message }
                    }),
                };
                self.send(&reply)
            }
            None => {
                if method == "session/update" {
                    handle_session_update(params, turn);
                }
                Ok(())
            }
        }
    }

    fn serve_request(
        &mut self,
        method: &str,
        params: Value,
        turn: &mut Turn,
    ) -> Result<Value, (i64, String)> {
        match method {
            "fs/read_text_file" => {
                let p: ReadTextFileParams = parse_params(params)?;
                let path = resolve_in_root(&self.root, &p.path, true)
                    .map_err(|e| (INVALID_PARAMS_CODE, e))?;
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| (INVALID_PARAMS_CODE, format!("{}: {e}", p.path)))?;
                Ok(json!({ "content": slice_lines(&content, p.line, p.limit) }))
            }
            "fs/write_text_file" => {
                let p: WriteTextFileParams = parse_params(params)?;
                if !self.policy.allow_writes {
                    return Err((INVALID_PARAMS_CODE, "file writes are disabled".into()));
                }
                let path = resolve_in_root(&self.root, &p.path, false)
                    .map_err(|e| (INVALID_PARAMS_CODE, e))?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| (INVALID_PARAMS_CODE, format!("{}: {e}", p.path)))?;
                }
                std::fs::write(&path, p.content)
                    .map_err(|e| (INVALID_PARAMS_CODE, format!("{}: {e}", p.path)))?;
                turn.note_edit(&path.to_string_lossy());
                Ok(json!({}))
            }
            "session/request_permission" => {
                let p: RequestPermissionParams = parse_params(params)?;
                Ok(self.decide_permission(p, turn))
            }
            _ => Err((METHOD_NOT_FOUND_CODE, format!("method not found: {method}"))),
        }
    }

    fn decide_permission(&self, p: RequestPermissionParams, turn: &mut Turn) -> Value {
        if turn.cancelled {
            return json!({ "outcome": { "outcome": "cancelled" } });
        }
        let kind = p.tool_call.kind.or_else(|| {
            turn.tools
                .get(&p.tool_call.tool_call_id)
                .and_then(|t| t.kind)
        });
        let title = p
            .tool_call
            .title
            .clone()
            .or_else(|| {
                turn.tools
                    .get(&p.tool_call.tool_call_id)
                    .map(|t| t.title.clone())
            })
            .unwrap_or_else(|| "tool call".to_string());
        let allow = permission_allowed(self.policy, kind);
        let preference: &[PermissionOptionKind] = if allow {
            &[
                PermissionOptionKind::AllowOnce,
                PermissionOptionKind::AllowAlways,
            ]
        } else {
            &[
                PermissionOptionKind::RejectOnce,
                PermissionOptionKind::RejectAlways,
            ]
        };
        let chosen = preference
            .iter()
            .find_map(|want| p.options.iter().find(|o| o.kind == *want));
        match chosen {
            Some(option) => {
                if !allow {
                    let why = if self.policy.allow_writes {
                        "enable skip permissions to auto-approve"
                    } else {
                        "read-only run"
                    };
                    turn.log_line(&format!("\u{2717} permission denied: {title} ({why})"));
                }
                json!({ "outcome": { "outcome": "selected", "optionId": option.option_id } })
            }
            None => json!({ "outcome": { "outcome": "cancelled" } }),
        }
    }

    fn send(&mut self, value: &Value) -> Result<(), AcpError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| AcpError::Protocol("agent stdin already closed".into()))?;
        let line = serde_json::to_string(value)
            .map_err(|e| AcpError::Protocol(format!("serialize error: {e}")))?;
        writeln!(stdin, "{line}")
            .and_then(|_| stdin.flush())
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::BrokenPipe => AcpError::AgentExited(self.stderr_tail()),
                _ => AcpError::Io(e),
            })
    }

    fn stderr_tail(&self) -> String {
        // Give the stderr thread a moment to drain what the agent wrote last.
        std::thread::sleep(Duration::from_millis(50));
        match self.stderr_tail.lock() {
            Ok(tail) => tail.trim().to_string(),
            Err(poisoned) => poisoned.into_inner().trim().to_string(),
        }
    }
}

impl Drop for AcpConnection {
    /// ACP v1 has no session teardown request: closing stdin is the signal.
    /// Whatever is left of the process group afterwards gets SIGTERM, then
    /// SIGKILL.
    fn drop(&mut self) {
        self.stdin.take();
        let deadline = Instant::now() + EXIT_GRACE;
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        terminate_process_group(&mut self.child);
    }
}

/// Signal the agent's whole process group, not just the leader: a wrapper
/// such as `npx` can exit on EOF while the adapter it started keeps running.
fn terminate_process_group(child: &mut Child) {
    #[cfg(unix)]
    {
        let pgid = child.id() as i32;
        // Reap the leader first so a zombie doesn't count as a live member.
        let group_alive = |child: &mut Child| {
            let _ = child.try_wait();
            unsafe { libc::kill(-pgid, 0) == 0 }
        };
        if !group_alive(child) {
            return;
        }
        unsafe {
            libc::kill(-pgid, libc::SIGTERM);
        }
        let deadline = Instant::now() + EXIT_GRACE;
        while Instant::now() < deadline {
            if !group_alive(child) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        unsafe {
            libc::kill(-pgid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn read_messages(stdout: impl Read, tx: mpsc::Sender<ReaderEvent>) {
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                let _ = tx.send(ReaderEvent::Eof);
                return;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<IncomingMessage>(trimmed) {
                    Ok(msg) => {
                        if tx.send(ReaderEvent::Message(msg)).is_err() {
                            return;
                        }
                    }
                    // Stray non-protocol output; stdout is reserved for
                    // JSON-RPC, but some agents leak banners onto it.
                    Err(e) => log::debug!("ACP: skipping non-JSON-RPC line ({e}): {trimmed}"),
                }
            }
            Err(e) => {
                let _ = tx.send(ReaderEvent::Failed(e));
                return;
            }
        }
    }
}

fn collect_stderr_tail(stderr: impl Read, tail: Arc<Mutex<String>>) {
    for line in BufReader::new(stderr).lines() {
        let Ok(line) = line else { return };
        log::debug!("ACP agent stderr: {line}");
        let mut buf = tail.lock().unwrap_or_else(|e| e.into_inner());
        buf.push_str(&line);
        buf.push('\n');
        if buf.len() > STDERR_TAIL_BYTES {
            let mut cut = buf.len() - STDERR_TAIL_BYTES;
            while !buf.is_char_boundary(cut) {
                cut += 1;
            }
            buf.drain(..cut);
        }
    }
}

fn handle_session_update(params: Value, turn: &mut Turn) {
    let notification: SessionNotification = match serde_json::from_value(params) {
        Ok(n) => n,
        Err(e) => {
            log::debug!("ACP: ignoring unparseable session/update: {e}");
            return;
        }
    };
    if turn.session_id.as_deref() != Some(notification.session_id.as_str()) {
        return;
    }
    match notification.update {
        SessionUpdate::AgentMessageChunk {
            content: ContentBlock::Text { text },
        } => {
            turn.text.push_str(&text);
            turn.log_text(&text);
        }
        SessionUpdate::AgentMessageChunk { .. } | SessionUpdate::Other => {}
        SessionUpdate::ToolCall(call) | SessionUpdate::ToolCallUpdate(call) => {
            handle_tool_call(call, turn);
        }
        SessionUpdate::Plan { entries } => {
            let rendered: String = entries
                .iter()
                .map(|e| {
                    let mark = match e.status {
                        Some(PlanEntryStatus::Completed) => "[x]",
                        Some(PlanEntryStatus::InProgress) => "[~]",
                        _ => "[ ]",
                    };
                    format!("  {mark} {}\n", e.content)
                })
                .collect();
            // Agents resend the whole plan on every change; only log changes.
            if !rendered.is_empty() && rendered != turn.last_plan {
                turn.log_line(&format!("Plan:\n{}", rendered.trim_end()));
                turn.last_plan = rendered;
            }
        }
    }
}

fn handle_tool_call(call: ToolCall, turn: &mut Turn) {
    let is_new = !turn.tools.contains_key(&call.tool_call_id);
    let state = turn.tools.entry(call.tool_call_id.clone()).or_default();
    if let Some(title) = call.title.filter(|t| !t.is_empty()) {
        state.title = title;
    }
    if call.kind.is_some() {
        state.kind = call.kind;
    }
    let title = if state.title.is_empty() {
        "tool".to_string()
    } else {
        state.title.clone()
    };
    let kind = state.kind;
    let newly_finished = !state.finished
        && matches!(
            call.status,
            Some(ToolCallStatus::Completed | ToolCallStatus::Failed)
        );
    if newly_finished {
        state.finished = true;
    }

    if is_new {
        turn.log_line(&format!("\u{2192} {title}"));
    }
    if newly_finished {
        match call.status {
            Some(ToolCallStatus::Failed) => turn.log_line(&format!("\u{2717} {title} failed")),
            _ => turn.tool_calls_completed += 1,
        }
    }

    for content in call.content.into_iter().flatten() {
        match content {
            ToolCallContent::Diff { path } => turn.note_edit(&path),
            ToolCallContent::Content {
                content: ContentBlock::Text { text },
            } if call.status == Some(ToolCallStatus::Failed) => {
                turn.log_line(text.trim_end());
            }
            _ => {}
        }
    }
    if matches!(
        kind,
        Some(ToolKind::Edit | ToolKind::Delete | ToolKind::Move)
    ) {
        for location in call.locations.into_iter().flatten() {
            turn.note_edit(&location.path);
        }
    }
}

/// Permission policy: read-only runs may only read; otherwise everything is
/// allowed when auto-approve is on and nothing is when it is off.
fn permission_allowed(policy: ClientPolicy, kind: Option<ToolKind>) -> bool {
    let read_like = matches!(
        kind,
        Some(ToolKind::Read | ToolKind::Search | ToolKind::Think | ToolKind::Fetch)
    );
    if !policy.allow_writes {
        return read_like;
    }
    policy.auto_approve
}

/// Resolve an agent-supplied path and require it to stay inside `root`
/// (already canonical). Symlinks are resolved on the existing part of the path,
/// so a link pointing out of the project is rejected; the not-yet-existing tail
/// (for new files) may not contain `..`.
fn resolve_in_root(root: &Path, raw: &str, must_exist: bool) -> Result<PathBuf, String> {
    let candidate = Path::new(raw);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };

    let mut existing = joined.as_path();
    let mut tail = Vec::new();
    while existing.symlink_metadata().is_err() {
        let name = existing
            .file_name()
            .ok_or_else(|| format!("invalid path: {raw}"))?;
        tail.push(name.to_owned());
        existing = existing
            .parent()
            .ok_or_else(|| format!("invalid path: {raw}"))?;
    }
    if must_exist && !tail.is_empty() {
        return Err(format!("no such file: {raw}"));
    }
    let mut resolved =
        std::fs::canonicalize(existing).map_err(|e| format!("cannot resolve {raw}: {e}"))?;
    if !resolved.starts_with(root) {
        return Err(format!("path is outside the project: {raw}"));
    }
    for name in tail.iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

/// Apply ACP's optional 1-based `line` / `limit` window to file content.
fn slice_lines(content: &str, line: Option<u32>, limit: Option<u32>) -> String {
    if line.is_none() && limit.is_none() {
        return content.to_string();
    }
    let skip = line.map_or(0, |l| l.saturating_sub(1) as usize);
    let take = limit.map_or(usize::MAX, |l| l as usize);
    content
        .split_inclusive('\n')
        .skip(skip)
        .take(take)
        .collect()
}

fn parse_params<T: serde::de::DeserializeOwned>(params: Value) -> Result<T, (i64, String)> {
    serde_json::from_value(params)
        .map_err(|e| (INVALID_PARAMS_CODE, format!("invalid params: {e}")))
}

fn to_value(v: impl serde::Serialize) -> Value {
    serde_json::to_value(v).expect("ACP request types always serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn_with_session(log: &mut dyn FnMut(&str)) -> Turn<'_> {
        let mut turn = Turn::new(log);
        turn.session_id = Some("s1".into());
        turn
    }

    fn update(update: Value) -> Value {
        json!({ "sessionId": "s1", "update": update })
    }

    #[test]
    fn message_chunks_accumulate_and_stream() {
        let mut logged = String::new();
        let mut log = |s: &str| logged.push_str(s);
        let mut turn = turn_with_session(&mut log);
        for chunk in ["Hello", " world"] {
            handle_session_update(
                update(json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": chunk }
                })),
                &mut turn,
            );
        }
        assert_eq!(turn.text, "Hello world");
        drop(turn);
        assert_eq!(logged, "Hello world");
    }

    #[test]
    fn updates_for_other_sessions_are_ignored() {
        let mut log = |_: &str| {};
        let mut turn = turn_with_session(&mut log);
        handle_session_update(
            json!({
                "sessionId": "other",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "x" }
                }
            }),
            &mut turn,
        );
        assert!(turn.text.is_empty());
    }

    #[test]
    fn unknown_update_kinds_are_ignored() {
        let mut log = |_: &str| {};
        let mut turn = turn_with_session(&mut log);
        handle_session_update(
            update(
                json!({ "sessionUpdate": "available_commands_update", "availableCommands": [] }),
            ),
            &mut turn,
        );
        assert!(turn.text.is_empty());
    }

    #[test]
    fn tool_calls_merge_updates_and_track_edits() {
        let mut logged = String::new();
        let mut log = |s: &str| logged.push_str(s);
        let mut turn = turn_with_session(&mut log);
        handle_session_update(
            update(json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "t1",
                "title": "Edit src/main.rs",
                "kind": "edit",
                "status": "pending",
                "locations": [{ "path": "/repo/src/main.rs" }]
            })),
            &mut turn,
        );
        handle_session_update(
            update(json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": "t1",
                "status": "completed",
                "content": [{
                    "type": "diff",
                    "path": "/repo/src/lib.rs",
                    "oldText": "a",
                    "newText": "b"
                }]
            })),
            &mut turn,
        );
        // A repeated completion must not be double-counted.
        handle_session_update(
            update(
                json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed" }),
            ),
            &mut turn,
        );
        assert_eq!(turn.tool_calls_completed, 1);
        assert_eq!(
            turn.edited_files,
            vec!["/repo/src/main.rs", "/repo/src/lib.rs"]
        );
        drop(turn);
        assert_eq!(logged, "\u{2192} Edit src/main.rs\n");
    }

    #[test]
    fn read_locations_are_not_edits() {
        let mut log = |_: &str| {};
        let mut turn = turn_with_session(&mut log);
        handle_session_update(
            update(json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "t1",
                "title": "Read README.md",
                "kind": "read",
                "locations": [{ "path": "/repo/README.md" }]
            })),
            &mut turn,
        );
        assert!(turn.edited_files.is_empty());
    }

    #[test]
    fn status_lines_start_on_a_fresh_line() {
        let mut logged = String::new();
        let mut log = |s: &str| logged.push_str(s);
        let mut turn = turn_with_session(&mut log);
        turn.log_text("thinking");
        turn.log_line("\u{2192} Run tests");
        drop(turn);
        assert_eq!(logged, "thinking\n\u{2192} Run tests\n");
    }

    #[test]
    fn unchanged_plans_are_logged_once() {
        let mut logged = String::new();
        let mut log = |s: &str| logged.push_str(s);
        let mut turn = turn_with_session(&mut log);
        let plan = update(json!({
            "sessionUpdate": "plan",
            "entries": [{ "content": "Write code", "priority": "high", "status": "in_progress" }]
        }));
        handle_session_update(plan.clone(), &mut turn);
        handle_session_update(plan, &mut turn);
        drop(turn);
        assert_eq!(logged, "Plan:\n  [~] Write code\n");
    }

    #[test]
    fn permission_policy() {
        let rw_auto = ClientPolicy {
            allow_writes: true,
            auto_approve: true,
        };
        let rw_manual = ClientPolicy {
            allow_writes: true,
            auto_approve: false,
        };
        let read_only = ClientPolicy {
            allow_writes: false,
            auto_approve: true,
        };
        assert!(permission_allowed(rw_auto, Some(ToolKind::Execute)));
        assert!(permission_allowed(rw_auto, None));
        assert!(!permission_allowed(rw_manual, Some(ToolKind::Edit)));
        assert!(!permission_allowed(rw_manual, Some(ToolKind::Read)));
        assert!(permission_allowed(read_only, Some(ToolKind::Read)));
        assert!(!permission_allowed(read_only, Some(ToolKind::Edit)));
        assert!(!permission_allowed(read_only, None));
    }

    #[test]
    fn resolve_in_root_rejects_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("a.txt"), "x").unwrap();

        assert_eq!(
            resolve_in_root(&root, "a.txt", true).unwrap(),
            root.join("a.txt")
        );
        let abs = root.join("a.txt").to_string_lossy().to_string();
        assert!(resolve_in_root(&root, &abs, true).is_ok());
        assert_eq!(
            resolve_in_root(&root, "new/dir/b.txt", false).unwrap(),
            root.join("new/dir/b.txt")
        );
        assert!(resolve_in_root(&root, "missing.txt", true).is_err());
        assert!(resolve_in_root(&root, "../outside.txt", false).is_err());
        assert!(resolve_in_root(&root, "new/../../outside.txt", false).is_err());
        assert!(resolve_in_root(&root, "/etc/hosts", true).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn resolve_in_root_rejects_symlink_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("link")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("nope"), root.join("dangling")).unwrap();

        assert!(resolve_in_root(&root, "link/file.txt", false).is_err());
        assert!(resolve_in_root(&root, "dangling", false).is_err());
    }

    #[test]
    fn slice_lines_windows() {
        let text = "one\ntwo\nthree\n";
        assert_eq!(slice_lines(text, None, None), text);
        assert_eq!(slice_lines(text, Some(2), None), "two\nthree\n");
        assert_eq!(slice_lines(text, Some(1), Some(2)), "one\ntwo\n");
        assert_eq!(slice_lines(text, Some(9), None), "");
    }
}
