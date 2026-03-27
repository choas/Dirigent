use serde::{Deserialize, Serialize};

/// User-configurable language server entry (persisted in settings.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LspServerConfig {
    /// Human-readable label shown in the UI (e.g. "rust-analyzer").
    pub name: String,
    /// File extensions this server handles (e.g. ["rs"]).
    pub extensions: Vec<String>,
    /// Command to launch the server (e.g. "rust-analyzer").
    pub command: String,
    /// Arguments passed to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables set before spawning (NAME=VALUE pairs).
    #[serde(default)]
    pub env: Vec<String>,
    /// Whether this server is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Well-known language server presets.
pub(crate) fn default_lsp_servers() -> Vec<LspServerConfig> {
    vec![
        LspServerConfig {
            name: "rust-analyzer".into(),
            extensions: vec!["rs".into()],
            command: "rust-analyzer".into(),
            args: vec![],
            env: vec![],
            enabled: false,
        },
        LspServerConfig {
            name: "typescript-language-server".into(),
            extensions: vec!["ts".into(), "tsx".into(), "js".into(), "jsx".into()],
            command: "typescript-language-server".into(),
            args: vec!["--stdio".into()],
            env: vec![],
            enabled: false,
        },
        LspServerConfig {
            name: "pylsp".into(),
            extensions: vec!["py".into()],
            command: "pylsp".into(),
            args: vec![],
            env: vec![],
            enabled: false,
        },
        LspServerConfig {
            name: "gopls".into(),
            extensions: vec!["go".into()],
            command: "gopls".into(),
            args: vec![],
            env: vec![],
            enabled: false,
        },
        LspServerConfig {
            name: "jdtls".into(),
            extensions: vec!["java".into()],
            command: "jdtls".into(),
            args: vec![],
            env: vec![],
            enabled: false,
        },
        LspServerConfig {
            name: "clangd".into(),
            extensions: vec!["c".into(), "cpp".into(), "h".into(), "hpp".into()],
            command: "clangd".into(),
            args: vec![],
            env: vec![],
            enabled: false,
        },
    ]
}
