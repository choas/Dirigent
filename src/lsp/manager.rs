use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

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

/// Manages all running LSP clients for a project.
#[allow(dead_code)]
pub(crate) struct LspManager {
    /// The project root all servers share.
    project_root: PathBuf,
    /// Running clients keyed by server config name.
    clients: HashMap<String, LspClient>,
    /// Extension -> server name mapping (built from configs).
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
    /// Shell init snippet for resolving commands in macOS GUI context.
    shell_init: String,
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
            shell_init: shell_init.to_string(),
        }
    }

    /// Start language servers from the provided configs.
    /// Only starts servers that are enabled and not already running.
    pub fn start_servers(&mut self, configs: &[LspServerConfig]) {
        // Build extension map from all enabled configs
        self.extension_map.clear();
        for cfg in configs {
            if cfg.enabled {
                for ext in &cfg.extensions {
                    self.extension_map.insert(ext.clone(), cfg.name.clone());
                }
            }
        }

        for cfg in configs {
            if !cfg.enabled {
                continue;
            }
            if self.clients.contains_key(&cfg.name) {
                continue; // already running
            }
            self.start_server(cfg);
        }
    }

    /// Start a single language server.
    fn start_server(&mut self, cfg: &LspServerConfig) {
        let msg = format!("Starting LSP: {} ({})", cfg.name, cfg.command);
        eprintln!("[lsp] {}", msg);
        self.status_log.push(msg);

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
                self.pending_init.insert(cfg.name.clone(), init_id);
                self.clients.insert(cfg.name.clone(), client);
            }
            Err(e) => {
                let msg = format!("Failed to start {}: {}", cfg.name, e);
                eprintln!("[lsp] {}", msg);
                self.status_log.push(msg);
            }
        }
    }

    /// Stop a specific language server.
    pub fn stop_server(&mut self, name: &str) {
        if let Some(client) = self.clients.remove(name) {
            let _ = client.shutdown();
            // Give it a moment, then exit
            std::thread::sleep(std::time::Duration::from_millis(100));
            client.exit();
            client.kill();
            self.initialized.remove(name);
            self.pending_init.remove(name);
            let msg = format!("Stopped LSP: {}", name);
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

        // Remove dead clients
        let dead: Vec<String> = self
            .clients
            .iter()
            .filter(|(_, c)| !c.is_alive())
            .map(|(k, _)| k.clone())
            .collect();
        for name in dead {
            let msg = format!("LSP server exited: {}", name);
            eprintln!("[lsp] {}", msg);
            self.status_log.push(msg);
            self.clients.remove(&name);
            self.initialized.remove(&name);
            self.pending_init.remove(&name);
        }
    }

    /// Handle a message from a specific server.
    fn handle_message(&mut self, server_name: &str, msg: LspMessage) {
        match &msg {
            LspMessage::Response { id, result, error } => {
                // Check if this is an initialize response
                if let Some(init_id) = self.pending_init.get(server_name) {
                    if id == init_id {
                        self.pending_init.remove(&server_name.to_string());
                        if error.is_some() {
                            let msg = format!("LSP {} initialize failed: {:?}", server_name, error);
                            eprintln!("[lsp] {}", msg);
                            self.status_log.push(msg);
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

                // Forward other responses to event channel
                let _ = self.event_tx.send(LspEvent {
                    server_name: server_name.to_string(),
                    message: msg,
                });
            }
            LspMessage::Notification { method, params } => {
                // Log diagnostics
                if method == "textDocument/publishDiagnostics" {
                    if let Ok(diag_params) = serde_json::from_value::<
                        lsp_types::PublishDiagnosticsParams,
                    >(params.clone())
                    {
                        let count = diag_params.diagnostics.len();
                        if count > 0 {
                            let uri = diag_params.uri.to_string();
                            let short_uri = uri.rsplit('/').next().unwrap_or(&uri);
                            let msg = format!(
                                "LSP {}: {} diagnostics for {}",
                                server_name, count, short_uri
                            );
                            eprintln!("[lsp] {}", msg);
                        }
                    }
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
    pub fn hover(&self, file_path: &Path, line: u32, character: u32) -> Option<u64> {
        let server_name = self.server_for_file(file_path)?;
        if self.initialized.get(&server_name) != Some(&true) {
            return None;
        }
        let client = self.clients.get(&server_name)?;
        Some(client.hover(file_path, line, character))
    }

    /// Request go-to-definition for a position.
    pub fn definition(&self, file_path: &Path, line: u32, character: u32) -> Option<u64> {
        let server_name = self.server_for_file(file_path)?;
        if self.initialized.get(&server_name) != Some(&true) {
            return None;
        }
        let client = self.clients.get(&server_name)?;
        Some(client.definition(file_path, line, character))
    }

    /// Request find-references for a position.
    pub fn references(&self, file_path: &Path, line: u32, character: u32) -> Option<u64> {
        let server_name = self.server_for_file(file_path)?;
        if self.initialized.get(&server_name) != Some(&true) {
            return None;
        }
        let client = self.clients.get(&server_name)?;
        Some(client.references(file_path, line, character))
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

impl Drop for LspManager {
    fn drop(&mut self) {
        self.stop_all();
    }
}
