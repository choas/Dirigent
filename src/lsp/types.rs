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

// ---------------------------------------------------------------------------
// Language presets for LSP server initialization
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LspLanguage {
    Rust,
    TypeScript,
    Python,
    Go,
    Java,
    CSharp,
    Cpp,
    Ruby,
    Swift,
    Kotlin,
    Elixir,
    Zig,
    Lua,
}

impl LspLanguage {
    pub fn label(&self) -> &'static str {
        match self {
            LspLanguage::Rust => "Rust",
            LspLanguage::TypeScript => "TypeScript",
            LspLanguage::Python => "Python",
            LspLanguage::Go => "Go",
            LspLanguage::Java => "Java",
            LspLanguage::CSharp => "C#",
            LspLanguage::Ruby => "Ruby",
            LspLanguage::Swift => "Swift",
            LspLanguage::Kotlin => "Kotlin",
            LspLanguage::Cpp => "C/C++",
            LspLanguage::Elixir => "Elixir",
            LspLanguage::Zig => "Zig",
            LspLanguage::Lua => "Lua",
        }
    }

    pub fn all() -> &'static [LspLanguage] {
        &[
            LspLanguage::Rust,
            LspLanguage::TypeScript,
            LspLanguage::Python,
            LspLanguage::Go,
            LspLanguage::Java,
            LspLanguage::CSharp,
            LspLanguage::Cpp,
            LspLanguage::Ruby,
            LspLanguage::Swift,
            LspLanguage::Kotlin,
            LspLanguage::Elixir,
            LspLanguage::Zig,
            LspLanguage::Lua,
        ]
    }
}

pub(crate) fn lsp_servers_for_language(lang: LspLanguage) -> Vec<LspServerConfig> {
    match lang {
        LspLanguage::Rust => vec![LspServerConfig {
            name: "rust-analyzer".into(),
            extensions: vec!["rs".into()],
            command: "rust-analyzer".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        }],
        LspLanguage::TypeScript => vec![LspServerConfig {
            name: "typescript-language-server".into(),
            extensions: vec!["ts".into(), "tsx".into(), "js".into(), "jsx".into()],
            command: "typescript-language-server".into(),
            args: vec!["--stdio".into()],
            env: vec![],
            enabled: true,
        }],
        LspLanguage::Python => vec![LspServerConfig {
            name: "pylsp".into(),
            extensions: vec!["py".into()],
            command: "pylsp".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        }],
        LspLanguage::Go => vec![LspServerConfig {
            name: "gopls".into(),
            extensions: vec!["go".into()],
            command: "gopls".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        }],
        LspLanguage::Java => vec![LspServerConfig {
            name: "jdtls".into(),
            extensions: vec!["java".into()],
            command: "jdtls".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        }],
        LspLanguage::CSharp => vec![LspServerConfig {
            name: "omnisharp".into(),
            extensions: vec!["cs".into()],
            command: "OmniSharp".into(),
            args: vec!["--languageserver".into()],
            env: vec![],
            enabled: true,
        }],
        LspLanguage::Cpp => vec![LspServerConfig {
            name: "clangd".into(),
            extensions: vec!["c".into(), "cpp".into(), "h".into(), "hpp".into()],
            command: "clangd".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        }],
        LspLanguage::Ruby => vec![LspServerConfig {
            name: "solargraph".into(),
            extensions: vec!["rb".into()],
            command: "solargraph".into(),
            args: vec!["stdio".into()],
            env: vec![],
            enabled: true,
        }],
        LspLanguage::Swift => vec![LspServerConfig {
            name: "sourcekit-lsp".into(),
            extensions: vec!["swift".into()],
            command: "sourcekit-lsp".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        }],
        LspLanguage::Kotlin => vec![LspServerConfig {
            name: "kotlin-language-server".into(),
            extensions: vec!["kt".into(), "kts".into()],
            command: "kotlin-language-server".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        }],
        LspLanguage::Elixir => vec![LspServerConfig {
            name: "elixir-ls".into(),
            extensions: vec!["ex".into(), "exs".into()],
            command: "elixir-ls".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        }],
        LspLanguage::Zig => vec![LspServerConfig {
            name: "zls".into(),
            extensions: vec!["zig".into()],
            command: "zls".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        }],
        LspLanguage::Lua => vec![LspServerConfig {
            name: "lua-language-server".into(),
            extensions: vec!["lua".into()],
            command: "lua-language-server".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        }],
    }
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
