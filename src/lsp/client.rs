use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;

use std::process::ChildStderr;

use lsp_types::{
    ClientCapabilities, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentSymbolClientCapabilities, DocumentSymbolParams,
    DynamicRegistrationClientCapabilities, GotoCapability, GotoDefinitionParams,
    HoverClientCapabilities, HoverParams, InitializeParams, MarkupKind, Position,
    PublishDiagnosticsClientCapabilities, ReferenceContext, ReferenceParams, ServerCapabilities,
    TextDocumentClientCapabilities, TextDocumentContentChangeEvent, TextDocumentIdentifier,
    TextDocumentItem, TextDocumentPositionParams, TextDocumentSyncClientCapabilities, Uri,
    VersionedTextDocumentIdentifier, WorkspaceFolder,
};
use std::str::FromStr;

/// A message received from the language server.
#[derive(Debug, Clone)]
pub(crate) enum LspMessage {
    /// A response to a request we sent.
    Response {
        id: u64,
        result: Option<serde_json::Value>,
        error: Option<serde_json::Value>,
    },
    /// A server-initiated request (has id and method, expects a response).
    Request {
        id: u64,
        method: String,
        #[allow(dead_code)]
        params: serde_json::Value,
    },
    /// A notification from the server (no id).
    Notification {
        method: String,
        params: serde_json::Value,
    },
    /// The reader thread encountered an error or the server exited.
    ServerExited(String),
}

/// A running LSP client connected to a single language server process.
#[allow(dead_code)]
pub(crate) struct LspClient {
    /// The server config name (for logging).
    pub name: String,
    /// The project root this server was initialized with.
    pub root: PathBuf,
    /// Writer to server stdin (behind Mutex for thread safety).
    writer: Arc<Mutex<BufWriter<std::process::ChildStdin>>>,
    /// Receiver for messages from the reader thread.
    pub rx: mpsc::Receiver<LspMessage>,
    /// Auto-incrementing request ID.
    next_id: AtomicU64,
    /// The server process (kept alive; dropped on shutdown).
    process: Arc<Mutex<Option<Child>>>,
    /// The reader thread handle.
    _reader_thread: JoinHandle<()>,
    /// Server capabilities received from initialize response.
    pub capabilities: Option<ServerCapabilities>,
    /// Files currently open (URI -> version).
    open_files: HashMap<String, i32>,
}

/// Read and log stderr lines from an LSP server process.
fn drain_stderr(name: String, stderr: ChildStderr) {
    let reader = BufReader::new(stderr);
    for line in reader.lines() {
        match line {
            Ok(l) => eprintln!("[lsp:{}:stderr] {}", name, l),
            Err(_) => break,
        }
    }
}

/// Read JSON-RPC messages from stdout and forward them via the channel.
fn drain_stdout(name: String, stdout: std::process::ChildStdout, tx: mpsc::Sender<LspMessage>) {
    let mut reader = BufReader::new(stdout);
    loop {
        match read_lsp_message(&mut reader) {
            Ok(Some(msg)) => {
                if tx.send(msg).is_err() {
                    break;
                }
            }
            Ok(None) => {
                let _ = tx.send(LspMessage::ServerExited(format!(
                    "{} exited normally",
                    name
                )));
                break;
            }
            Err(e) => {
                let _ = tx.send(LspMessage::ServerExited(format!(
                    "{} read error: {}",
                    name, e
                )));
                break;
            }
        }
    }
}

#[allow(dead_code)]
impl LspClient {
    /// Spawn a language server process and start the reader thread.
    /// Does NOT send `initialize` yet — call `initialize()` after construction.
    pub fn spawn(
        name: &str,
        command: &str,
        args: &[String],
        env: &[String],
        root: &Path,
        shell_init: &str,
    ) -> Result<Self, String> {
        // Resolve command path via shell (handles GUI-app PATH issues on macOS).
        let resolved_cmd = resolve_command(command, shell_init)
            .ok_or_else(|| format!("LSP server '{}' not found: {}", name, command))?;

        let mut cmd = Command::new(&resolved_cmd);
        cmd.args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Apply environment variables.
        // Split on newlines to handle legacy entries where multiple KEY=VALUE
        // pairs were collapsed into a single string (join/split mismatch bug).
        for pair in env {
            for line in pair.split('\n') {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    cmd.env(k.trim(), v.trim());
                }
            }
        }

        let mut child = cmd.spawn().map_err(|e| {
            format!(
                "Failed to spawn LSP server '{}' ({}): {}",
                name, resolved_cmd, e
            )
        })?;

        let stdin = child.stdin.take().ok_or("Failed to capture LSP stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to capture LSP stdout")?;

        let writer = Arc::new(Mutex::new(BufWriter::new(stdin)));
        let (tx, rx) = mpsc::channel();
        let server_name = name.to_string();

        // Spawn stderr reader (just log to eprintln for now)
        if let Some(stderr) = child.stderr.take() {
            let name_clone = server_name.clone();
            std::thread::spawn(move || drain_stderr(name_clone, stderr));
        }

        // Spawn stdout reader: parse JSON-RPC messages
        let reader_thread = {
            let name_clone = server_name.clone();
            std::thread::spawn(move || drain_stdout(name_clone, stdout, tx))
        };

        let process = Arc::new(Mutex::new(Some(child)));

        Ok(LspClient {
            name: name.to_string(),
            root: root.to_path_buf(),
            writer,
            rx,
            next_id: AtomicU64::new(1),
            process,
            _reader_thread: reader_thread,
            capabilities: None,
            open_files: HashMap::new(),
        })
    }

    /// Send the `initialize` request. Returns the request ID.
    #[allow(deprecated)]
    pub fn initialize(&self) -> u64 {
        let root_uri = file_uri(&self.root);

        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(root_uri.clone()),
            capabilities: ClientCapabilities {
                text_document: Some(TextDocumentClientCapabilities {
                    hover: Some(HoverClientCapabilities {
                        dynamic_registration: Some(false),
                        content_format: Some(vec![MarkupKind::PlainText, MarkupKind::Markdown]),
                    }),
                    definition: Some(GotoCapability {
                        dynamic_registration: Some(false),
                        link_support: Some(false),
                    }),
                    references: Some(DynamicRegistrationClientCapabilities {
                        dynamic_registration: Some(false),
                    }),
                    document_symbol: Some(DocumentSymbolClientCapabilities {
                        dynamic_registration: Some(false),
                        ..Default::default()
                    }),
                    synchronization: Some(TextDocumentSyncClientCapabilities {
                        dynamic_registration: Some(false),
                        will_save: Some(false),
                        will_save_wait_until: Some(false),
                        did_save: Some(true),
                    }),
                    publish_diagnostics: Some(PublishDiagnosticsClientCapabilities {
                        related_information: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: root_uri,
                name: self
                    .root
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
            }]),
            ..Default::default()
        };

        self.send_request(
            "initialize",
            serde_json::to_value(params).expect("LSP params must be serializable"),
        )
    }

    /// Send the `initialized` notification (must be called after receiving initialize response).
    pub fn initialized(&self) {
        self.send_notification("initialized", serde_json::json!({}));
    }

    /// Send `textDocument/didOpen` for a file.
    pub fn did_open(&mut self, file_path: &Path) {
        let uri = file_uri(file_path);
        let uri_str = uri.as_str().to_string();

        // Don't re-open if already tracked
        if self.open_files.contains_key(&uri_str) {
            return;
        }

        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => return,
        };

        let language_id = language_id_from_path(file_path);

        let params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri,
                language_id,
                version: 1,
                text: content,
            },
        };

        self.send_notification(
            "textDocument/didOpen",
            serde_json::to_value(params).expect("LSP params must be serializable"),
        );
        self.open_files.insert(uri_str, 1);
    }

    /// Send `textDocument/didClose` for a file.
    pub fn did_close(&mut self, file_path: &Path) {
        let uri = file_uri(file_path);
        let uri_str = uri.as_str().to_string();

        if self.open_files.remove(&uri_str).is_none() {
            return; // wasn't open
        }

        let params = DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier { uri },
        };

        self.send_notification(
            "textDocument/didClose",
            serde_json::to_value(params).expect("LSP params must be serializable"),
        );
    }

    /// Send `textDocument/didChange` for a file (full content sync).
    pub fn did_change(&mut self, file_path: &Path) {
        let uri = file_uri(file_path);
        let uri_str = uri.as_str().to_string();

        // Ensure didOpen has been sent before any didChange
        if !self.open_files.contains_key(&uri_str) {
            self.did_open(file_path);
            // did_open may fail (e.g. unreadable file) — check again
            if !self.open_files.contains_key(&uri_str) {
                return;
            }
        }

        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => return,
        };

        let Some(version) = self.open_files.get_mut(&uri_str) else {
            return;
        };
        *version += 1;
        let ver = *version;

        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier { uri, version: ver },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: content,
            }],
        };

        self.send_notification(
            "textDocument/didChange",
            serde_json::to_value(params).expect("LSP params must be serializable"),
        );
    }

    /// Request `textDocument/hover`.
    pub fn hover(&self, file_path: &Path, line: u32, character: u32) -> u64 {
        let uri = file_uri(file_path);
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
        };
        self.send_request(
            "textDocument/hover",
            serde_json::to_value(params).expect("LSP params must be serializable"),
        )
    }

    /// Request `textDocument/definition`.
    pub fn definition(&self, file_path: &Path, line: u32, character: u32) -> u64 {
        let uri = file_uri(file_path);
        let params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.send_request(
            "textDocument/definition",
            serde_json::to_value(params).expect("LSP params must be serializable"),
        )
    }

    /// Request `textDocument/documentSymbol`.
    pub fn document_symbols(&self, file_path: &Path) -> u64 {
        let uri = file_uri(file_path);
        let params = DocumentSymbolParams {
            text_document: TextDocumentIdentifier { uri },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.send_request(
            "textDocument/documentSymbol",
            serde_json::to_value(params).expect("LSP params must be serializable"),
        )
    }

    /// Request `textDocument/references`.
    pub fn references(&self, file_path: &Path, line: u32, character: u32) -> u64 {
        let uri = file_uri(file_path);
        let params = ReferenceParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position { line, character },
            },
            context: ReferenceContext {
                include_declaration: true,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        self.send_request(
            "textDocument/references",
            serde_json::to_value(params).expect("LSP params must be serializable"),
        )
    }

    /// Send `shutdown` request and then `exit` notification.
    pub fn shutdown(&self) -> u64 {
        self.send_request("shutdown", serde_json::Value::Null)
    }

    /// Send `exit` notification (call after shutdown response).
    pub fn exit(&self) {
        self.send_notification("exit", serde_json::Value::Null);
    }

    /// Kill the server process.
    pub fn kill(&self) {
        if let Ok(mut guard) = self.process.lock() {
            if let Some(ref mut child) = *guard {
                let _ = child.kill();
            }
        }
    }

    /// Check if the server process is still running.
    pub fn is_alive(&self) -> bool {
        if let Ok(mut guard) = self.process.lock() {
            if let Some(ref mut child) = *guard {
                return child.try_wait().ok().flatten().is_none();
            }
        }
        false
    }

    // -- Internal helpers --

    fn send_request(&self, method: &str, params: serde_json::Value) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_message(&msg);
        id
    }

    fn send_notification(&self, method: &str, params: serde_json::Value) {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.write_message(&msg);
    }

    fn write_message(&self, msg: &serde_json::Value) {
        let body = serde_json::to_string(msg).expect("LSP message must be serializable");
        let mut writer = match self.writer.lock() {
            Ok(w) => w,
            Err(e) => {
                eprintln!("[lsp] failed to acquire writer lock: {e}");
                return;
            }
        };
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        if let Err(e) = writer.write_all(header.as_bytes()) {
            eprintln!("[lsp] failed to write header: {e}");
            return;
        }
        if let Err(e) = writer.write_all(body.as_bytes()) {
            eprintln!("[lsp] failed to write body: {e}");
            return;
        }
        if let Err(e) = writer.flush() {
            eprintln!("[lsp] failed to flush: {e}");
        }
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        // Best-effort graceful shutdown
        self.shutdown();
        std::thread::sleep(std::time::Duration::from_millis(100));
        self.exit();
        self.kill();
    }
}

/// Read a single LSP JSON-RPC message from the stream.
/// Returns None on EOF.
fn read_lsp_message<R: BufRead>(reader: &mut R) -> Result<Option<LspMessage>, String> {
    // Read headers
    let mut content_length: Option<usize> = None;
    loop {
        let mut header_line = String::new();
        let bytes_read = reader
            .read_line(&mut header_line)
            .map_err(|e| e.to_string())?;
        if bytes_read == 0 {
            return Ok(None); // EOF
        }
        let trimmed = header_line.trim();
        if trimmed.is_empty() {
            break; // End of headers
        }
        if let Some(len_str) = trimmed.strip_prefix("Content-Length:") {
            content_length = len_str.trim().parse().ok();
        }
    }

    let length = content_length.ok_or("Missing Content-Length header")?;

    // Read body
    let mut body = vec![0u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|e| format!("Failed to read LSP body: {}", e))?;

    let json: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| format!("Invalid LSP JSON: {}", e))?;

    // Distinguish response vs notification
    if let Some(id) = json.get("id") {
        // It's a response (has "id" and either "result" or "error")
        if json.get("method").is_some() {
            // Server-initiated request (e.g. window/showMessageRequest) — preserve id so callers can respond
            Ok(Some(LspMessage::Request {
                id: id.as_u64().unwrap_or(0),
                method: json["method"].as_str().unwrap_or("").to_string(),
                params: json
                    .get("params")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            }))
        } else {
            Ok(Some(LspMessage::Response {
                id: id.as_u64().unwrap_or(0),
                result: json.get("result").cloned(),
                error: json.get("error").cloned(),
            }))
        }
    } else {
        // Notification
        Ok(Some(LspMessage::Notification {
            method: json["method"].as_str().unwrap_or("").to_string(),
            params: json
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        }))
    }
}

/// Returns `true` if `name` contains only characters safe for shell interpolation
/// (alphanumeric, hyphen, underscore, dot, forward slash).
/// Rejects shell metacharacters like `;`, `|`, `$`, backtick, etc.
fn is_safe_command_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
}

/// Resolve a command name using the login shell (handles macOS GUI PATH issues).
fn resolve_command(command: &str, shell_init: &str) -> Option<String> {
    // Reject command names with shell metacharacters to prevent command injection.
    // The command field comes from settings.json which may be attacker-controlled
    // (e.g. a malicious .Dirigent/settings.json checked into a repository).
    if !is_safe_command_name(command) {
        eprintln!(
            "[lsp] refusing to resolve command with unsafe characters: {:?}",
            command
        );
        return None;
    }

    // If it's already an absolute path, use it directly
    if Path::new(command).is_absolute() {
        if Path::new(command).exists() {
            return Some(command.to_string());
        }
        return None;
    }

    // Try which crate first
    if let Ok(path) = which::which(command) {
        return Some(path.to_string_lossy().to_string());
    }

    // Fall back to login shell resolution (for macOS GUI apps)
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let script = if shell_init.is_empty() {
        format!("which {}", command)
    } else {
        format!("{}; which {}", shell_init, command)
    };
    let output = Command::new(&shell)
        .args(["-l", "-c", &script])
        .output()
        .ok()?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() && Path::new(&path).exists() {
            return Some(path);
        }
    }
    None
}

/// Convert a file system path to an LSP `Uri` (file:// scheme).
fn file_uri(path: &Path) -> Uri {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    // Canonicalize to resolve `.`, `..`, and duplicate separators.
    // Fall back to the absolute path if canonicalize fails (e.g. file doesn't exist yet).
    let canonical = abs.canonicalize().unwrap_or(abs);
    let url = url::Url::from_file_path(&canonical)
        .expect("Url::from_file_path should not fail for an absolute path");
    Uri::from_str(url.as_str()).expect("a valid file:// URL should parse as Uri")
}

/// Map a file extension to an LSP language identifier.
fn language_id_from_path(path: &Path) -> String {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "rs" => "rust",
        "ts" => "typescript",
        "tsx" => "typescriptreact",
        "js" => "javascript",
        "jsx" => "javascriptreact",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "c" => "c",
        "cpp" | "cc" | "cxx" => "cpp",
        "h" => "c",
        "hpp" | "hxx" => "cpp",
        "rb" => "ruby",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "cs" => "csharp",
        "lua" => "lua",
        "zig" => "zig",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "json" => "json",
        "html" => "html",
        "css" => "css",
        "scss" => "scss",
        "sh" | "bash" | "zsh" => "shellscript",
        _ => ext,
    }
    .to_string()
}
