use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;

use super::client::{LspClient, LspMessage};
use super::types::LspServerConfig;

/// Event forwarded from any language server to the main app.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct LspEvent {
    /// Which server produced this event.
    pub server_name: String,
    /// The message itself.
    pub message: LspMessage,
}

/// Kinds of pending LSP requests we track for routing responses.
#[derive(Debug, Clone, PartialEq)]
enum PendingRequestKind {
    Hover,
    Definition,
    References,
    DocumentSymbol(PathBuf),
}

/// An LSP diagnostic for a specific file.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct LspDiagnostic {
    pub line: usize, // 1-based
    pub character: usize,
    pub end_line: usize,
    pub end_character: usize,
    pub severity: LspDiagSeverity,
    pub message: String,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum LspDiagSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

/// An LSP document symbol mapped to our internal representation.
#[derive(Debug, Clone)]
pub(crate) struct LspDocumentSymbol {
    pub name: String,
    pub kind: lsp_types::SymbolKind,
    pub line: usize, // 1-based
    pub depth: usize,
}

/// Manages all running LSP clients for a project.
#[allow(dead_code)]
pub(crate) struct LspManager {
    /// The project root all servers share.
    project_root: PathBuf,
    /// Running clients keyed by server config id.
    clients: HashMap<String, LspClient>,
    /// Extension -> server id mapping (built from configs).
    extension_map: HashMap<String, String>,
    /// Aggregated event channel: all servers forward here.
    pub event_tx: mpsc::Sender<LspEvent>,
    pub event_rx: mpsc::Receiver<LspEvent>,
    /// Pending initialize request IDs (server_name -> request_id).
    pending_init: HashMap<String, u64>,
    /// Servers that have completed initialization.
    initialized: HashMap<String, bool>,
    /// Log of LSP status messages (server_name -> latest status).
    pub status_log: Vec<String>,
    /// Per-server error messages (server_name -> error string). Cleared on successful start.
    pub failed_servers: HashMap<String, String>,
    /// Servers pending graceful shutdown: name -> (shutdown_request_id, when_initiated).
    pending_shutdowns: HashMap<String, (u64, Instant)>,
    /// Shell init snippet for resolving commands in macOS GUI context.
    shell_init: String,

    // -- Shared state for UI consumption --
    /// Pending requests: request_id -> kind. Used to route responses.
    pending_requests: HashMap<u64, PendingRequestKind>,

    /// Latest hover result (markdown/plaintext). Cleared when a new hover is requested.
    pub hover_result: Option<String>,
    /// Whether a hover request is in flight.
    pub hover_pending: bool,

    /// Latest definition result: (file_path, line_1based).
    pub definition_result: Option<(PathBuf, usize)>,
    /// Whether a definition request is in flight.
    pub definition_pending: bool,

    /// Latest references result: list of (file_path, line_1based).
    pub references_result: Option<Vec<(PathBuf, usize)>>,

    /// LSP diagnostics per file (absolute path -> diagnostics).
    pub diagnostics: HashMap<PathBuf, Vec<LspDiagnostic>>,

    /// LSP document symbols per file (absolute path -> symbols).
    pub document_symbols: HashMap<PathBuf, Vec<LspDocumentSymbol>>,
    /// Pending document symbol request IDs (file_path -> request_id).
    pending_doc_symbols: HashMap<PathBuf, u64>,

    /// Debounce: last hover request position to avoid spamming.
    pub hover_file: Option<PathBuf>,
    pub hover_line: u32,
    pub hover_char: u32,
}

#[allow(dead_code)]
impl LspManager {
    pub fn new(project_root: PathBuf, shell_init: &str) -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        LspManager {
            project_root,
            clients: HashMap::new(),
            extension_map: HashMap::new(),
            event_tx,
            event_rx,
            pending_init: HashMap::new(),
            initialized: HashMap::new(),
            status_log: Vec::new(),
            failed_servers: HashMap::new(),
            pending_shutdowns: HashMap::new(),
            shell_init: shell_init.to_string(),
            pending_requests: HashMap::new(),
            hover_result: None,
            hover_pending: false,
            definition_result: None,
            definition_pending: false,
            references_result: None,
            diagnostics: HashMap::new(),
            document_symbols: HashMap::new(),
            pending_doc_symbols: HashMap::new(),
            hover_file: None,
            hover_line: 0,
            hover_char: 0,
        }
    }

    /// Start language servers from the provided configs.
    /// Only starts servers that are enabled and not already running.
    /// Rebuilds the full extension map from all configs.
    pub fn start_servers(&mut self, configs: &[LspServerConfig]) {
        // Build extension map from all enabled configs
        self.extension_map.clear();
        for cfg in configs {
            if cfg.enabled {
                for ext in &cfg.extensions {
                    self.extension_map.insert(ext.clone(), cfg.id.clone());
                }
            }
        }

        for cfg in configs {
            if !cfg.enabled {
                continue;
            }
            if self.clients.contains_key(&cfg.id) {
                continue; // already running
            }
            self.start_one(cfg);
        }
    }

    /// Start a single server without clearing the extension map.
    /// Use this when the user clicks "Start" on one server card.
    pub fn start_single(&mut self, cfg: &LspServerConfig) {
        if !cfg.enabled || self.clients.contains_key(&cfg.id) {
            return;
        }
        // Add this server's extensions to the map (don't clear others)
        for ext in &cfg.extensions {
            self.extension_map.insert(ext.clone(), cfg.id.clone());
        }
        self.start_one(cfg);
    }

    /// Start a single language server.
    fn start_one(&mut self, cfg: &LspServerConfig) {
        let msg = format!("Starting LSP: {} ({})", cfg.name, cfg.command);
        eprintln!("[lsp] {}", msg);
        self.status_log.push(msg);
        self.failed_servers.remove(&cfg.id);

        match LspClient::spawn(
            &cfg.name,
            &cfg.command,
            &cfg.args,
            &cfg.env,
            &self.project_root,
            &self.shell_init,
        ) {
            Ok(client) => {
                let init_id = client.initialize();
                self.pending_init.insert(cfg.id.clone(), init_id);
                self.clients.insert(cfg.id.clone(), client);
            }
            Err(e) => {
                let msg = format!("Failed to start {}: {}", cfg.name, e);
                eprintln!("[lsp] {}", msg);
                self.status_log.push(msg.clone());
                self.failed_servers.insert(cfg.id.clone(), msg);
            }
        }
    }

    /// Stop a specific language server (non-blocking).
    /// Sends `shutdown` and registers for deferred cleanup in `poll()`.
    pub fn stop_server(&mut self, name: &str) {
        if self.pending_shutdowns.contains_key(name) {
            return; // already shutting down
        }
        if let Some(client) = self.clients.get(name) {
            let shutdown_id = client.shutdown();
            self.pending_shutdowns
                .insert(name.to_string(), (shutdown_id, Instant::now()));
            let msg = format!("Stopping LSP: {}", name);
            eprintln!("[lsp] {}", msg);
            self.status_log.push(msg);
        }
    }

    /// Stop all language servers.
    pub fn stop_all(&mut self) {
        let names: Vec<String> = self.clients.keys().cloned().collect();
        for name in names {
            self.stop_server(&name);
        }
    }

    /// Force-stop a server immediately (synchronous teardown, no deferred shutdown).
    fn force_stop_server(&mut self, name: &str) {
        self.pending_shutdowns.remove(name);
        if let Some(client) = self.clients.remove(name) {
            let _ = client.shutdown();
            client.exit();
            client.kill();
        }
        self.initialized.remove(name);
        self.pending_init.remove(name);
        let msg = format!("Force-stopped LSP: {}", name);
        eprintln!("[lsp] {}", msg);
        self.status_log.push(msg);
    }

    /// Reconcile running servers with the desired configuration.
    /// Force-stops servers not present (or not enabled) in the new config,
    /// then starts servers that are missing from clients.
    pub fn reconcile(&mut self, configs: &[LspServerConfig]) {
        let desired: HashSet<String> = configs
            .iter()
            .filter(|c| c.enabled)
            .map(|c| c.id.clone())
            .collect();

        // Force-stop servers no longer in the desired set
        let to_stop: Vec<String> = self
            .clients
            .keys()
            .filter(|name| !desired.contains(*name))
            .cloned()
            .collect();
        for name in to_stop {
            self.force_stop_server(&name);
        }

        // Start new/missing servers (start_servers skips those already in clients)
        self.start_servers(configs);
    }

    /// Force-stop all servers and restart from configs.
    pub fn restart_all(&mut self, configs: &[LspServerConfig]) {
        let names: Vec<String> = self.clients.keys().cloned().collect();
        for name in names {
            self.force_stop_server(&name);
        }
        self.start_servers(configs);
    }

    /// Poll all running clients for new messages. Call this each frame.
    pub fn poll(&mut self) {
        let client_names: Vec<String> = self.clients.keys().cloned().collect();
        for name in &client_names {
            let msgs: Vec<LspMessage> = {
                if let Some(client) = self.clients.get(name) {
                    client.rx.try_iter().collect()
                } else {
                    continue;
                }
            };

            for msg in msgs {
                self.handle_message(name, msg);
            }
        }

        // Complete pending shutdowns that have timed out (graceful window: 500ms)
        let timed_out: Vec<String> = self
            .pending_shutdowns
            .iter()
            .filter(|(_, (_, initiated))| {
                initiated.elapsed() >= std::time::Duration::from_millis(500)
            })
            .map(|(name, _)| name.clone())
            .collect();
        for name in timed_out {
            self.finish_shutdown(&name);
        }

        // Remove dead clients
        let dead: Vec<String> = self
            .clients
            .iter()
            .filter(|(_, c)| !c.is_alive())
            .map(|(k, _)| k.clone())
            .collect();
        for name in dead {
            // If it was pending shutdown, just finish that
            if self.pending_shutdowns.contains_key(&name) {
                self.finish_shutdown(&name);
                continue;
            }
            let msg = format!("LSP server exited: {}", name);
            eprintln!("[lsp] {}", msg);
            self.status_log.push(msg.clone());
            self.failed_servers.insert(name.clone(), msg);
            self.clients.remove(&name);
            self.initialized.remove(&name);
            self.pending_init.remove(&name);
        }
    }

    /// Complete a pending shutdown: send exit, kill, and clean up state.
    fn finish_shutdown(&mut self, name: &str) {
        self.pending_shutdowns.remove(name);
        if let Some(client) = self.clients.remove(name) {
            client.exit();
            client.kill();
        }
        self.initialized.remove(name);
        self.pending_init.remove(name);
        let msg = format!("Stopped LSP: {}", name);
        eprintln!("[lsp] {}", msg);
        self.status_log.push(msg);
    }

    /// Handle a message from a specific server.
    fn handle_message(&mut self, server_name: &str, msg: LspMessage) {
        match &msg {
            LspMessage::Response { id, result, error } => {
                // Check if this is a shutdown response for a pending shutdown
                if let Some((shutdown_id, _)) = self.pending_shutdowns.get(server_name) {
                    if id == shutdown_id {
                        self.finish_shutdown(server_name);
                        return;
                    }
                }

                // Check if this is an initialize response
                if let Some(init_id) = self.pending_init.get(server_name) {
                    if id == init_id {
                        self.pending_init.remove(&server_name.to_string());
                        if error.is_some() {
                            let msg = format!("LSP {} initialize failed: {:?}", server_name, error);
                            eprintln!("[lsp] {}", msg);
                            self.status_log.push(msg.clone());
                            self.failed_servers.insert(server_name.to_string(), msg);
                            // Teardown the partially-started server so start_single() can retry
                            if let Some(client) = self.clients.remove(server_name) {
                                client.kill();
                            }
                            self.initialized.remove(server_name);
                        } else {
                            // Parse server capabilities
                            if let Some(result_val) = result {
                                if let Ok(init_result) =
                                    serde_json::from_value::<lsp_types::InitializeResult>(
                                        result_val.clone(),
                                    )
                                {
                                    if let Some(client) = self.clients.get_mut(server_name) {
                                        client.capabilities = Some(init_result.capabilities);
                                    }
                                }
                            }
                            // Send initialized notification
                            if let Some(client) = self.clients.get(server_name) {
                                client.initialized();
                            }
                            self.initialized.insert(server_name.to_string(), true);
                            let msg = format!("LSP {} initialized", server_name);
                            eprintln!("[lsp] {}", msg);
                            self.status_log.push(msg);
                        }
                        return;
                    }
                }

                // Route tracked responses to shared state
                if let Some(kind) = self.pending_requests.remove(id) {
                    self.handle_tracked_response(kind, result, error);
                    return;
                }

                // Forward other responses to event channel
                let _ = self.event_tx.send(LspEvent {
                    server_name: server_name.to_string(),
                    message: msg,
                });
            }
            LspMessage::Request { id, method, .. } => {
                // Server-initiated requests — forward to event channel so callers can respond
                eprintln!("[lsp] server request: {} (id={})", method, id);
                let _ = self.event_tx.send(LspEvent {
                    server_name: server_name.to_string(),
                    message: msg,
                });
            }
            LspMessage::Notification { method, params } => {
                // Parse and store diagnostics
                if method == "textDocument/publishDiagnostics" {
                    self.handle_diagnostics_notification(server_name, params);
                }

                // Forward all notifications
                let _ = self.event_tx.send(LspEvent {
                    server_name: server_name.to_string(),
                    message: LspMessage::Notification {
                        method: method.clone(),
                        params: params.clone(),
                    },
                });
            }
            LspMessage::ServerExited(reason) => {
                let msg = format!("LSP {}: {}", server_name, reason);
                eprintln!("[lsp] {}", msg);
                self.status_log.push(msg);
            }
        }
    }

    /// Handle a tracked response (hover, definition, references, documentSymbol).
    fn handle_tracked_response(
        &mut self,
        kind: PendingRequestKind,
        result: &Option<serde_json::Value>,
        error: &Option<serde_json::Value>,
    ) {
        if error.is_some() {
            eprintln!("[lsp] {:?} request error: {:?}", kind, error);
            match kind {
                PendingRequestKind::Hover => self.hover_pending = false,
                PendingRequestKind::Definition => self.definition_pending = false,
                PendingRequestKind::DocumentSymbol(ref path) => {
                    self.pending_doc_symbols.remove(path);
                    self.document_symbols.remove(path);
                }
                _ => {}
            }
            return;
        }

        let result_val = match result {
            Some(v) if !v.is_null() => v,
            _ => {
                match kind {
                    PendingRequestKind::Hover => {
                        self.hover_pending = false;
                        self.hover_result = None;
                    }
                    PendingRequestKind::Definition => {
                        self.definition_pending = false;
                        self.definition_result = None;
                    }
                    PendingRequestKind::DocumentSymbol(ref path) => {
                        self.pending_doc_symbols.remove(path);
                        self.document_symbols.remove(path);
                    }
                    _ => {}
                }
                return;
            }
        };

        match kind {
            PendingRequestKind::Hover => {
                self.hover_pending = false;
                self.hover_result = Self::parse_hover_result(result_val);
            }
            PendingRequestKind::Definition => {
                self.definition_pending = false;
                self.definition_result = Self::parse_definition_result(result_val);
            }
            PendingRequestKind::References => {
                self.references_result = Self::parse_references_result(result_val);
            }
            PendingRequestKind::DocumentSymbol(file_path) => {
                self.pending_doc_symbols.remove(&file_path);
                if let Some(syms) = Self::parse_document_symbols(result_val) {
                    self.document_symbols.insert(file_path, syms);
                }
            }
        }
    }

    /// Parse hover response into a display string.
    fn parse_hover_result(val: &serde_json::Value) -> Option<String> {
        let hover: lsp_types::Hover = serde_json::from_value(val.clone()).ok()?;
        match hover.contents {
            lsp_types::HoverContents::Scalar(content) => match content {
                lsp_types::MarkedString::String(s) => Some(s),
                lsp_types::MarkedString::LanguageString(ls) => Some(ls.value),
            },
            lsp_types::HoverContents::Array(items) => {
                let parts: Vec<String> = items
                    .into_iter()
                    .map(|item| match item {
                        lsp_types::MarkedString::String(s) => s,
                        lsp_types::MarkedString::LanguageString(ls) => ls.value,
                    })
                    .collect();
                Some(parts.join("\n\n"))
            }
            lsp_types::HoverContents::Markup(markup) => Some(markup.value),
        }
    }

    /// Parse definition response into (file_path, line).
    fn parse_definition_result(val: &serde_json::Value) -> Option<(PathBuf, usize)> {
        // Definition can be a single Location, Vec<Location>, or Vec<LocationLink>
        if let Ok(loc) = serde_json::from_value::<lsp_types::Location>(val.clone()) {
            return Self::location_to_path_line(&loc);
        }
        if let Ok(locs) = serde_json::from_value::<Vec<lsp_types::Location>>(val.clone()) {
            if let Some(loc) = locs.first() {
                return Self::location_to_path_line(loc);
            }
        }
        if let Ok(links) = serde_json::from_value::<Vec<lsp_types::LocationLink>>(val.clone()) {
            if let Some(link) = links.first() {
                let path = uri_to_path(&link.target_uri)?;
                let line = link.target_selection_range.start.line as usize + 1;
                return Some((path, line));
            }
        }
        None
    }

    /// Parse references response into list of (file_path, line).
    fn parse_references_result(val: &serde_json::Value) -> Option<Vec<(PathBuf, usize)>> {
        let locs: Vec<lsp_types::Location> = serde_json::from_value(val.clone()).ok()?;
        let results: Vec<(PathBuf, usize)> = locs
            .iter()
            .filter_map(|loc| {
                let path = uri_to_path(&loc.uri)?;
                let line = loc.range.start.line as usize + 1;
                Some((path, line))
            })
            .collect();
        if results.is_empty() {
            None
        } else {
            Some(results)
        }
    }

    /// Parse document symbols response.
    fn parse_document_symbols(val: &serde_json::Value) -> Option<Vec<LspDocumentSymbol>> {
        // Can be Vec<DocumentSymbol> (hierarchical) or Vec<SymbolInformation> (flat)
        if let Ok(syms) = serde_json::from_value::<Vec<lsp_types::DocumentSymbol>>(val.clone()) {
            let mut result = Vec::new();
            Self::flatten_document_symbols(&syms, 0, &mut result);
            return Some(result);
        }
        if let Ok(infos) = serde_json::from_value::<Vec<lsp_types::SymbolInformation>>(val.clone())
        {
            let result: Vec<LspDocumentSymbol> = infos
                .into_iter()
                .map(|info| LspDocumentSymbol {
                    name: info.name,
                    kind: info.kind,
                    line: info.location.range.start.line as usize + 1,
                    depth: 0,
                })
                .collect();
            return Some(result);
        }
        None
    }

    /// Recursively flatten hierarchical DocumentSymbol into a flat list with depth.
    fn flatten_document_symbols(
        syms: &[lsp_types::DocumentSymbol],
        depth: usize,
        out: &mut Vec<LspDocumentSymbol>,
    ) {
        for sym in syms {
            out.push(LspDocumentSymbol {
                name: sym.name.clone(),
                kind: sym.kind,
                line: sym.selection_range.start.line as usize + 1,
                depth,
            });
            if let Some(ref children) = sym.children {
                Self::flatten_document_symbols(children, depth + 1, out);
            }
        }
    }

    fn location_to_path_line(loc: &lsp_types::Location) -> Option<(PathBuf, usize)> {
        let path = uri_to_path(&loc.uri)?;
        let line = loc.range.start.line as usize + 1;
        Some((path, line))
    }

    /// Handle publishDiagnostics notification: parse and store.
    fn handle_diagnostics_notification(&mut self, server_name: &str, params: &serde_json::Value) {
        if let Ok(diag_params) =
            serde_json::from_value::<lsp_types::PublishDiagnosticsParams>(params.clone())
        {
            let path = match uri_to_path(&diag_params.uri) {
                Some(p) => p,
                None => return,
            };

            let diags: Vec<LspDiagnostic> = diag_params
                .diagnostics
                .iter()
                .map(|d| {
                    let severity = match d.severity {
                        Some(lsp_types::DiagnosticSeverity::ERROR) => LspDiagSeverity::Error,
                        Some(lsp_types::DiagnosticSeverity::WARNING) => LspDiagSeverity::Warning,
                        Some(lsp_types::DiagnosticSeverity::HINT) => LspDiagSeverity::Hint,
                        _ => LspDiagSeverity::Info,
                    };
                    LspDiagnostic {
                        line: d.range.start.line as usize + 1,
                        character: d.range.start.character as usize,
                        end_line: d.range.end.line as usize + 1,
                        end_character: d.range.end.character as usize,
                        severity,
                        message: d.message.clone(),
                        source: d.source.clone().unwrap_or_default(),
                    }
                })
                .collect();

            let count = diags.len();
            if count > 0 {
                let short = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                eprintln!("[lsp:{}] {} diagnostics for {}", server_name, count, short);
            }
            self.diagnostics.insert(path, diags);
        }
    }

    /// Request document symbols for a file and track pending request properly.
    pub fn request_document_symbols(&mut self, file_path: &Path) -> Option<u64> {
        self.document_symbols(file_path)
    }

    /// Notify the appropriate server that a file was opened.
    pub fn notify_file_opened(&mut self, file_path: &Path) {
        if let Some(server_name) = self.server_for_file(file_path) {
            if self.initialized.get(&server_name) == Some(&true) {
                if let Some(client) = self.clients.get_mut(&server_name) {
                    client.did_open(file_path);
                    let short = file_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    eprintln!("[lsp:{}] didOpen: {}", server_name, short);
                }
            }
        }
    }

    /// Notify the appropriate server that a file was closed.
    pub fn notify_file_closed(&mut self, file_path: &Path) {
        if let Some(server_name) = self.server_for_file(file_path) {
            if self.initialized.get(&server_name) == Some(&true) {
                if let Some(client) = self.clients.get_mut(&server_name) {
                    client.did_close(file_path);
                }
            }
        }
    }

    /// Notify the appropriate server that a file changed on disk.
    pub fn notify_file_changed(&mut self, file_path: &Path) {
        if let Some(server_name) = self.server_for_file(file_path) {
            if self.initialized.get(&server_name) == Some(&true) {
                if let Some(client) = self.clients.get_mut(&server_name) {
                    client.did_change(file_path);
                }
            }
        }
    }

    /// Request hover information for a position.
    /// Debounces: skips if same position was requested recently.
    pub fn hover(&mut self, file_path: &Path, line: u32, character: u32) -> Option<u64> {
        // Debounce: skip if same position
        if self.hover_file.as_deref() == Some(file_path)
            && self.hover_line == line
            && self.hover_char == character
            && self.hover_pending
        {
            return None;
        }
        let server_name = self.server_for_file(file_path)?;
        if self.initialized.get(&server_name) != Some(&true) {
            return None;
        }
        let client = self.clients.get(&server_name)?;
        let id = client.hover(file_path, line, character);
        self.pending_requests.insert(id, PendingRequestKind::Hover);
        self.hover_pending = true;
        self.hover_result = None;
        self.hover_file = Some(file_path.to_path_buf());
        self.hover_line = line;
        self.hover_char = character;
        Some(id)
    }

    /// Request go-to-definition for a position.
    pub fn definition(&mut self, file_path: &Path, line: u32, character: u32) -> Option<u64> {
        let server_name = self.server_for_file(file_path)?;
        if self.initialized.get(&server_name) != Some(&true) {
            return None;
        }
        let client = self.clients.get(&server_name)?;
        let id = client.definition(file_path, line, character);
        self.pending_requests
            .insert(id, PendingRequestKind::Definition);
        self.definition_pending = true;
        self.definition_result = None;
        Some(id)
    }

    /// Request find-references for a position.
    pub fn references(&mut self, file_path: &Path, line: u32, character: u32) -> Option<u64> {
        let server_name = self.server_for_file(file_path)?;
        if self.initialized.get(&server_name) != Some(&true) {
            return None;
        }
        let client = self.clients.get(&server_name)?;
        let id = client.references(file_path, line, character);
        self.pending_requests
            .insert(id, PendingRequestKind::References);
        Some(id)
    }

    /// Request document symbols for a file.
    pub fn document_symbols(&mut self, file_path: &Path) -> Option<u64> {
        // Don't re-request if one is already pending for this file
        if self.pending_doc_symbols.contains_key(file_path) {
            return None;
        }
        let server_name = self.server_for_file(file_path)?;
        if self.initialized.get(&server_name) != Some(&true) {
            return None;
        }
        let client = self.clients.get(&server_name)?;
        let id = client.document_symbols(file_path);
        self.pending_requests.insert(
            id,
            PendingRequestKind::DocumentSymbol(file_path.to_path_buf()),
        );
        self.pending_doc_symbols.insert(file_path.to_path_buf(), id);
        Some(id)
    }

    /// Check if an LSP server is initialized for this file's extension.
    pub fn has_initialized_server_for(&self, file_path: &Path) -> bool {
        if let Some(server_name) = self.server_for_file(file_path) {
            self.initialized.get(&server_name) == Some(&true)
        } else {
            false
        }
    }

    /// Returns the names of all running (initialized) servers.
    pub fn running_servers(&self) -> Vec<String> {
        self.initialized
            .iter()
            .filter(|(_, ready)| **ready)
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Returns the names of all running (but not yet initialized) servers.
    pub fn starting_servers(&self) -> Vec<String> {
        self.pending_init.keys().cloned().collect()
    }

    /// Check if any LSP server is available for a given file extension.
    pub fn has_server_for_extension(&self, ext: &str) -> bool {
        self.extension_map.contains_key(ext)
    }

    /// Look up which server handles a file (by extension).
    fn server_for_file(&self, file_path: &Path) -> Option<String> {
        let ext = file_path.extension().and_then(|e| e.to_str())?;
        self.extension_map.get(ext).cloned()
    }

    /// Update the shell init snippet (called when settings change).
    pub fn set_shell_init(&mut self, shell_init: &str) {
        self.shell_init = shell_init.to_string();
    }
}

/// Convert an LSP URI to a file system path.
fn uri_to_path(uri: &lsp_types::Uri) -> Option<PathBuf> {
    let s = uri.as_str();
    url::Url::parse(s).ok().and_then(|u| u.to_file_path().ok())
}

impl Drop for LspManager {
    fn drop(&mut self) {
        // Force-kill all servers immediately on drop (no poll loop available).
        for (_, client) in self.clients.drain() {
            let _ = client.shutdown();
            client.exit();
            client.kill();
        }
    }
}
