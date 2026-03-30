use serde::{Deserialize, Serialize};

/// User-configurable language server entry (persisted in settings.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LspServerConfig {
    /// Stable, non-editable identifier used as the manager key.
    /// Generated once when the config is created; never changes afterward.
    #[serde(default = "generate_id")]
    pub id: String,
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

fn generate_id() -> String {
    uuid::Uuid::new_v4().to_string()
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

/// Canonical server definitions for all supported languages.
/// Each entry has `enabled: true` by default.
fn base_lsp_server_configs() -> Vec<LspServerConfig> {
    vec![
        LspServerConfig {
            id: generate_id(),
            name: "rust-analyzer".into(),
            extensions: vec!["rs".into()],
            command: "rust-analyzer".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        },
        LspServerConfig {
            id: generate_id(),
            name: "typescript-language-server".into(),
            extensions: vec![
                "ts".into(),
                "tsx".into(),
                "js".into(),
                "jsx".into(),
                "mjs".into(),
                "mts".into(),
            ],
            command: "typescript-language-server".into(),
            args: vec!["--stdio".into()],
            env: vec![],
            enabled: true,
        },
        LspServerConfig {
            id: generate_id(),
            name: "pylsp".into(),
            extensions: vec!["py".into()],
            command: "pylsp".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        },
        LspServerConfig {
            id: generate_id(),
            name: "gopls".into(),
            extensions: vec!["go".into()],
            command: "gopls".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        },
        LspServerConfig {
            id: generate_id(),
            name: "jdtls".into(),
            extensions: vec!["java".into()],
            command: "jdtls".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        },
        LspServerConfig {
            id: generate_id(),
            name: "omnisharp".into(),
            extensions: vec!["cs".into()],
            command: "OmniSharp".into(),
            args: vec!["--languageserver".into()],
            env: vec![],
            enabled: true,
        },
        LspServerConfig {
            id: generate_id(),
            name: "clangd".into(),
            extensions: vec![
                "c".into(),
                "cpp".into(),
                "cc".into(),
                "cxx".into(),
                "h".into(),
                "hpp".into(),
                "hxx".into(),
            ],
            command: "clangd".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        },
        LspServerConfig {
            id: generate_id(),
            name: "solargraph".into(),
            extensions: vec!["rb".into()],
            command: "solargraph".into(),
            args: vec!["stdio".into()],
            env: vec![],
            enabled: true,
        },
        LspServerConfig {
            id: generate_id(),
            name: "sourcekit-lsp".into(),
            extensions: vec!["swift".into()],
            command: "sourcekit-lsp".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        },
        LspServerConfig {
            id: generate_id(),
            name: "kotlin-language-server".into(),
            extensions: vec!["kt".into(), "kts".into()],
            command: "kotlin-language-server".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        },
        LspServerConfig {
            id: generate_id(),
            name: "elixir-ls".into(),
            extensions: vec!["ex".into(), "exs".into()],
            command: "elixir-ls".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        },
        LspServerConfig {
            id: generate_id(),
            name: "zls".into(),
            extensions: vec!["zig".into()],
            command: "zls".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        },
        LspServerConfig {
            id: generate_id(),
            name: "lua-language-server".into(),
            extensions: vec!["lua".into()],
            command: "lua-language-server".into(),
            args: vec![],
            env: vec![],
            enabled: true,
        },
    ]
}

pub(crate) fn lsp_servers_for_language(lang: LspLanguage) -> Vec<LspServerConfig> {
    let server_name = match lang {
        LspLanguage::Rust => "rust-analyzer",
        LspLanguage::TypeScript => "typescript-language-server",
        LspLanguage::Python => "pylsp",
        LspLanguage::Go => "gopls",
        LspLanguage::Java => "jdtls",
        LspLanguage::CSharp => "omnisharp",
        LspLanguage::Cpp => "clangd",
        LspLanguage::Ruby => "solargraph",
        LspLanguage::Swift => "sourcekit-lsp",
        LspLanguage::Kotlin => "kotlin-language-server",
        LspLanguage::Elixir => "elixir-ls",
        LspLanguage::Zig => "zls",
        LspLanguage::Lua => "lua-language-server",
    };
    base_lsp_server_configs()
        .into_iter()
        .filter(|s| s.name == server_name)
        .collect()
}

/// Return an install hint for a known LSP server name.
/// Used to generate helpful cue prompts when the binary is not found.
pub(crate) fn lsp_install_hint(server_name: &str) -> String {
    match server_name {
        "rust-analyzer" => concat!(
            "Install rust-analyzer for Rust LSP support.\n\n",
            "Notes:\n",
            "- If using rustup: `rustup component add rust-analyzer`\n",
            "- Or install standalone: `brew install rust-analyzer` (macOS)\n",
            "- Ensure `rust-analyzer` is on PATH",
        )
        .to_string(),
        "typescript-language-server" => concat!(
            "Install typescript-language-server for TypeScript/JavaScript LSP support.\n\n",
            "Notes:\n",
            "- `npm install -g typescript-language-server typescript`\n",
            "- Or if using pnpm: `pnpm add -g typescript-language-server typescript`\n",
            "- Requires Node.js to be installed",
        )
        .to_string(),
        "pylsp" => concat!(
            "Install pylsp (python-lsp-server) for Python LSP support.\n\n",
            "Notes:\n",
            "- Use uv if the project uses uv: `uv tool install python-lsp-server`\n",
            "- Or with pip: `pip install python-lsp-server`\n",
            "- Or with pipx: `pipx install python-lsp-server`\n",
            "- Ensure `pylsp` is on PATH after installation",
        )
        .to_string(),
        "gopls" => concat!(
            "Install gopls for Go LSP support.\n\n",
            "Notes:\n",
            "- `go install golang.org/x/tools/gopls@latest`\n",
            "- Requires Go to be installed\n",
            "- Ensure `$GOPATH/bin` is on PATH",
        )
        .to_string(),
        "jdtls" => concat!(
            "Install Eclipse JDT Language Server for Java LSP support.\n\n",
            "Notes:\n",
            "- `brew install jdtls` (macOS)\n",
            "- Or download from https://download.eclipse.org/jdtls/\n",
            "- Requires Java JDK to be installed",
        )
        .to_string(),
        "clangd" => concat!(
            "Install clangd for C/C++ LSP support.\n\n",
            "Notes:\n",
            "- On macOS: `xcode-select --install` (includes clangd)\n",
            "- Or `brew install llvm` and add to PATH\n",
            "- On Linux: install via your package manager (e.g. `apt install clangd`)",
        )
        .to_string(),
        "solargraph" => concat!(
            "Install Solargraph for Ruby LSP support.\n\n",
            "Notes:\n",
            "- `gem install solargraph`\n",
            "- Or with bundler: add `gem 'solargraph'` to Gemfile\n",
            "- Requires Ruby to be installed",
        )
        .to_string(),
        "sourcekit-lsp" => concat!(
            "Install SourceKit-LSP for Swift LSP support.\n\n",
            "Notes:\n",
            "- Included with Xcode: `xcode-select --install`\n",
            "- Or install Swift toolchain from swift.org\n",
            "- Usually at `/usr/bin/sourcekit-lsp` after Xcode install",
        )
        .to_string(),
        "kotlin-language-server" => concat!(
            "Install kotlin-language-server for Kotlin LSP support.\n\n",
            "Notes:\n",
            "- `brew install kotlin-language-server` (macOS)\n",
            "- Or build from source: https://github.com/fwcd/kotlin-language-server\n",
            "- Requires JDK to be installed",
        )
        .to_string(),
        "OmniSharp" | "omnisharp" => concat!(
            "Install OmniSharp for C# LSP support.\n\n",
            "Notes:\n",
            "- `brew install omnisharp/omnisharp-roslyn/omnisharp-mono` (macOS)\n",
            "- Or download from https://github.com/OmniSharp/omnisharp-roslyn\n",
            "- Requires .NET SDK to be installed",
        )
        .to_string(),
        "elixir-ls" => concat!(
            "Install ElixirLS for Elixir LSP support.\n\n",
            "Notes:\n",
            "- Download from https://github.com/elixir-lsp/elixir-ls/releases\n",
            "- Requires Elixir and Erlang/OTP to be installed\n",
            "- Ensure the `elixir-ls` script is on PATH",
        )
        .to_string(),
        "zls" => concat!(
            "Install ZLS for Zig LSP support.\n\n",
            "Notes:\n",
            "- `brew install zls` (macOS) or download from https://github.com/zigtools/zls\n",
            "- Requires Zig to be installed\n",
            "- Ensure version matches your Zig compiler version",
        )
        .to_string(),
        "lua-language-server" => concat!(
            "Install lua-language-server for Lua LSP support.\n\n",
            "Notes:\n",
            "- `brew install lua-language-server` (macOS)\n",
            "- Or download from https://github.com/LuaLS/lua-language-server\n",
            "- No Lua runtime required (self-contained)",
        )
        .to_string(),
        _ => format!(
            "Install the '{}' language server.\n\n\
             Check the server's documentation for installation instructions \
             and ensure the binary is on PATH.",
            server_name
        ),
    }
}

/// Well-known language server presets (all disabled by default for settings).
pub(crate) fn default_lsp_servers() -> Vec<LspServerConfig> {
    let defaults = [
        "rust-analyzer",
        "typescript-language-server",
        "pylsp",
        "gopls",
        "jdtls",
        "clangd",
    ];
    base_lsp_server_configs()
        .into_iter()
        .filter(|s| defaults.contains(&s.name.as_str()))
        .map(|mut s| {
            s.enabled = false;
            s
        })
        .collect()
}
