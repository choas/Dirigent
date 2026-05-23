use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::types::*;

/// An active connection to an ACP agent subprocess.
pub(super) struct AcpConnection {
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    child: Child,
    next_id: u64,
    session_id: Option<String>,
    #[allow(dead_code)]
    agent_capabilities: Option<AgentCapabilities>,
}

impl AcpConnection {
    /// Spawn an ACP agent subprocess and perform the initialization handshake.
    pub fn spawn_and_initialize(
        binary: &str,
        args: &[&str],
        cwd: &Path,
        on_log: &mut dyn FnMut(&str),
    ) -> Result<Self, AcpError> {
        let mut cmd = std::process::Command::new(binary);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = cmd.spawn().map_err(AcpError::SpawnFailed)?;
        let stdin = child.stdin.take().expect("stdin must be piped");
        let stdout = child.stdout.take().expect("stdout must be piped");
        let reader = BufReader::new(stdout);

        let mut conn = AcpConnection {
            stdin,
            reader,
            child,
            next_id: 1,
            session_id: None,
            agent_capabilities: None,
        };

        on_log("[ACP] Initializing connection...");
        conn.do_initialize(on_log)?;
        Ok(conn)
    }

    /// Send the `initialize` request and process the response.
    fn do_initialize(&mut self, on_log: &mut dyn FnMut(&str)) -> Result<(), AcpError> {
        let params = InitializeParams {
            protocol_version: PROTOCOL_VERSION,
            client_info: ClientInfo {
                name: "Dirigent".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            capabilities: ClientCapabilities {
                fs: FsCapabilities {
                    read_text_file: true,
                    write_text_file: true,
                },
            },
        };

        let result = self.send_request("initialize", Some(serde_json::to_value(params).unwrap()))?;

        if let Some(val) = result {
            if let Ok(init_result) = serde_json::from_value::<InitializeResult>(val) {
                if let Some(ref info) = init_result.agent_info {
                    if let Some(ref name) = info.name {
                        on_log(&format!("[ACP] Connected to agent: {name}"));
                    }
                }
                self.agent_capabilities = init_result.capabilities;
            }
        }

        Ok(())
    }

    /// Create a new session with the given working directory.
    pub fn create_session(&mut self, cwd: &Path) -> Result<String, AcpError> {
        let params = SessionNewParams {
            cwd: cwd.to_string_lossy().to_string(),
        };

        let result =
            self.send_request("session/new", Some(serde_json::to_value(params).unwrap()))?;

        let session_id = result
            .and_then(|v| serde_json::from_value::<SessionNewResult>(v).ok())
            .map(|r| r.session_id)
            .ok_or_else(|| AcpError::ProtocolError("session/new returned no session_id".into()))?;

        self.session_id = Some(session_id.clone());
        Ok(session_id)
    }

    /// Send a prompt and stream updates back via the callback.
    /// Returns the final response when the turn completes.
    pub fn send_prompt(
        &mut self,
        text: &str,
        cancel: &Arc<AtomicBool>,
        on_log: &mut dyn FnMut(&str),
        on_diff: &mut dyn FnMut(DiffContent),
        on_edited_file: &mut dyn FnMut(&str),
    ) -> Result<AcpResponse, AcpError> {
        let session_id = self
            .session_id
            .clone()
            .ok_or_else(|| AcpError::ProtocolError("no active session".into()))?;

        let params = SessionPromptParams {
            session_id: session_id.clone(),
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
        };

        let id = self.next_id;
        self.next_id += 1;
        let request = JsonRpcRequest::new(id, "session/prompt", Some(serde_json::to_value(params).unwrap()));
        self.write_message(&request)?;

        let mut response_text = String::new();
        let mut diffs: Vec<DiffContent> = Vec::new();
        let mut edited_files: Vec<String> = Vec::new();
        let mut tool_calls_completed: u64 = 0;

        loop {
            if cancel.load(Ordering::Relaxed) {
                self.send_cancel(&session_id)?;
                return Err(AcpError::Cancelled);
            }

            let msg = self.read_message()?;

            if let Some(resp_id) = msg.id {
                if resp_id == id {
                    if let Some(err) = msg.error {
                        return Err(AcpError::ProtocolError(format!(
                            "session/prompt error {}: {}",
                            err.code, err.message
                        )));
                    }
                    break;
                }
            }

            if let Some(method) = &msg.method {
                match method.as_str() {
                    "session/update" => {
                        if let Some(params) = msg.params {
                            self.handle_session_update(
                                params,
                                &mut response_text,
                                &mut diffs,
                                &mut edited_files,
                                &mut tool_calls_completed,
                                on_log,
                                on_diff,
                                on_edited_file,
                            );
                        }
                    }
                    "fs/readTextFile" | "fs/read_text_file" => {
                        self.handle_fs_read(msg.id, msg.params)?;
                    }
                    "fs/writeTextFile" | "fs/write_text_file" => {
                        self.handle_fs_write(msg.id, msg.params, on_edited_file)?;
                    }
                    "session/requestPermission" | "session/request_permission" => {
                        self.handle_permission_request(msg.id, on_log)?;
                    }
                    _ => {}
                }
            }
        }

        Ok(AcpResponse {
            text: response_text,
            diffs,
            edited_files,
            tool_calls_completed,
        })
    }

    fn handle_session_update(
        &self,
        params: serde_json::Value,
        response_text: &mut String,
        diffs: &mut Vec<DiffContent>,
        edited_files: &mut Vec<String>,
        tool_calls_completed: &mut u64,
        on_log: &mut dyn FnMut(&str),
        on_diff: &mut dyn FnMut(DiffContent),
        on_edited_file: &mut dyn FnMut(&str),
    ) {
        let update: SessionUpdateParams = match serde_json::from_value(params) {
            Ok(u) => u,
            Err(_) => return,
        };

        match update.kind {
            SessionUpdateKind::AgentMessageChunk { text } => {
                if let Some(chunk) = text {
                    response_text.push_str(&chunk);
                    on_log(&chunk);
                }
            }
            SessionUpdateKind::ToolCall(ref tc) | SessionUpdateKind::ToolCallUpdate(ref tc) => {
                self.process_tool_call(
                    tc,
                    diffs,
                    edited_files,
                    tool_calls_completed,
                    on_log,
                    on_diff,
                    on_edited_file,
                );
            }
            SessionUpdateKind::Plan(ref plan) => {
                if let Some(entries) = &plan.entries {
                    for entry in entries {
                        if let Some(content) = &entry.content {
                            let status = entry.status.as_deref().unwrap_or("pending");
                            let marker = match status {
                                "completed" => "[done]",
                                "in_progress" => "[...]",
                                _ => "[ ]",
                            };
                            on_log(&format!("{marker} {content}"));
                        }
                    }
                }
            }
            SessionUpdateKind::Unknown => {}
        }
    }

    fn process_tool_call(
        &self,
        tc: &ToolCallUpdate,
        diffs: &mut Vec<DiffContent>,
        edited_files: &mut Vec<String>,
        tool_calls_completed: &mut u64,
        on_log: &mut dyn FnMut(&str),
        on_diff: &mut dyn FnMut(DiffContent),
        on_edited_file: &mut dyn FnMut(&str),
    ) {
        let title = tc.title.as_deref().unwrap_or("tool");
        let status_str = match &tc.status {
            Some(ToolCallStatus::Pending) => "pending",
            Some(ToolCallStatus::InProgress) => "running",
            Some(ToolCallStatus::Completed) => {
                *tool_calls_completed += 1;
                "done"
            }
            Some(ToolCallStatus::Failed) => "failed",
            _ => "",
        };

        if !status_str.is_empty() {
            let kind_str = match &tc.kind {
                Some(ToolCallKind::Edit) => "edit",
                Some(ToolCallKind::Read) => "read",
                Some(ToolCallKind::Execute) => "exec",
                Some(ToolCallKind::Search) => "search",
                Some(ToolCallKind::Delete) => "delete",
                Some(ToolCallKind::Think) => "think",
                Some(ToolCallKind::Fetch) => "fetch",
                _ => "tool",
            };
            on_log(&format!("[{kind_str}:{status_str}] {title}"));
        }

        if let Some(contents) = &tc.content {
            for content in contents {
                match content {
                    ToolCallContent::Diff(diff) => {
                        on_diff(diff.clone());
                        if !edited_files.contains(&diff.path) {
                            edited_files.push(diff.path.clone());
                            on_edited_file(&diff.path);
                        }
                        diffs.push(diff.clone());
                    }
                    ToolCallContent::Text { text } => {
                        if !text.is_empty() {
                            on_log(text);
                        }
                    }
                    ToolCallContent::Unknown => {}
                }
            }
        }
    }

    /// Handle an `fs/readTextFile` request from the agent.
    fn handle_fs_read(
        &mut self,
        req_id: Option<u64>,
        params: Option<serde_json::Value>,
    ) -> Result<(), AcpError> {
        let id = req_id.unwrap_or(0);
        let path = params
            .and_then(|p| serde_json::from_value::<FsReadTextFileParams>(p).ok())
            .map(|p| p.path)
            .unwrap_or_default();

        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "content": content }
        });
        self.write_raw(&response)
    }

    /// Handle an `fs/writeTextFile` request from the agent.
    fn handle_fs_write(
        &mut self,
        req_id: Option<u64>,
        params: Option<serde_json::Value>,
        on_edited_file: &mut dyn FnMut(&str),
    ) -> Result<(), AcpError> {
        let id = req_id.unwrap_or(0);
        let write_params = params
            .and_then(|p| serde_json::from_value::<FsWriteTextFileParams>(p).ok());

        let success = if let Some(wp) = write_params {
            on_edited_file(&wp.path);
            std::fs::write(&wp.path, &wp.content).is_ok()
        } else {
            false
        };

        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "success": success }
        });
        self.write_raw(&response)
    }

    /// Auto-approve permission requests (matching Dirigent's skip-permissions behavior).
    fn handle_permission_request(
        &mut self,
        req_id: Option<u64>,
        on_log: &mut dyn FnMut(&str),
    ) -> Result<(), AcpError> {
        let id = req_id.unwrap_or(0);
        on_log("[ACP] Permission requested — auto-approving");
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "decision": "allow_always" }
        });
        self.write_raw(&response)
    }

    /// Send a `session/cancel` notification to abort the current turn.
    fn send_cancel(&mut self, session_id: &str) -> Result<(), AcpError> {
        let params = SessionCancelParams {
            session_id: session_id.to_string(),
        };
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": params,
        });
        self.write_raw(&notification)
    }

    /// Send a JSON-RPC request and wait for the matching response.
    fn send_request(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, AcpError> {
        let id = self.next_id;
        self.next_id += 1;
        let request = JsonRpcRequest::new(id, method, params);
        self.write_message(&request)?;

        loop {
            let msg = self.read_message()?;
            if msg.id == Some(id) {
                if let Some(err) = msg.error {
                    return Err(AcpError::ProtocolError(format!(
                        "{} error {}: {}",
                        method, err.code, err.message
                    )));
                }
                return Ok(msg.result);
            }
            // Skip notifications that arrive before the response.
        }
    }

    /// Write a JSON-RPC message (newline-delimited).
    fn write_message(&mut self, request: &JsonRpcRequest) -> Result<(), AcpError> {
        let json = serde_json::to_string(request)
            .map_err(|e| AcpError::ProtocolError(format!("serialize error: {e}")))?;
        writeln!(self.stdin, "{json}").map_err(AcpError::IoError)?;
        self.stdin.flush().map_err(AcpError::IoError)?;
        Ok(())
    }

    /// Write a raw JSON value as a message.
    fn write_raw(&mut self, value: &serde_json::Value) -> Result<(), AcpError> {
        let json = serde_json::to_string(value)
            .map_err(|e| AcpError::ProtocolError(format!("serialize error: {e}")))?;
        writeln!(self.stdin, "{json}").map_err(AcpError::IoError)?;
        self.stdin.flush().map_err(AcpError::IoError)?;
        Ok(())
    }

    /// Read a single JSON-RPC message from stdout.
    fn read_message(&mut self) -> Result<JsonRpcResponse, AcpError> {
        let mut line = String::new();
        loop {
            line.clear();
            let bytes_read = self.reader.read_line(&mut line).map_err(AcpError::IoError)?;
            if bytes_read == 0 {
                return Err(AcpError::ProtocolError("agent closed stdout".into()));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<JsonRpcResponse>(trimmed) {
                Ok(msg) => return Ok(msg),
                Err(_) => continue,
            }
        }
    }

    /// Close the session and kill the subprocess.
    pub fn shutdown(mut self) {
        if let Some(session_id) = self.session_id.take() {
            let _ = self.send_request(
                "session/close",
                Some(serde_json::json!({ "sessionId": session_id })),
            );
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for AcpConnection {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
