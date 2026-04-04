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
// Language presets — single source of truth for all supported languages
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

/// Static metadata for a supported language and its LSP server.
struct LanguagePreset {
    language: LspLanguage,
    label: &'static str,
    server_name: &'static str,
    extensions: &'static [&'static str],
    command: &'static str,
    args: &'static [&'static str],
    install_hint: &'static str,
    /// Included in the default settings preset list.
    default_preset: bool,
}

const PRESETS: &[LanguagePreset] = &[
    LanguagePreset {
        language: LspLanguage::Rust,
        label: "Rust",
        server_name: "rust-analyzer",
        extensions: &["rs"],
        command: "rust-analyzer",
        args: &[],
        install_hint: concat!(
            "Install rust-analyzer for Rust LSP support.\n\n",
            "Notes:\n",
            "- If using rustup: `rustup component add rust-analyzer`\n",
            "- Or install standalone: `brew install rust-analyzer` (macOS)\n",
            "- Ensure `rust-analyzer` is on PATH",
        ),
        default_preset: true,
    },
    LanguagePreset {
        language: LspLanguage::TypeScript,
        label: "TypeScript",
        server_name: "typescript-language-server",
        extensions: &["ts", "tsx", "js", "jsx", "mjs", "mts"],
        command: "typescript-language-server",
        args: &["--stdio"],
        install_hint: concat!(
            "Install typescript-language-server for TypeScript/JavaScript LSP support.\n\n",
            "Notes:\n",
            "- `npm install -g typescript-language-server typescript`\n",
            "- Or if using pnpm: `pnpm add -g typescript-language-server typescript`\n",
            "- Requires Node.js to be installed",
        ),
        default_preset: true,
    },
    LanguagePreset {
        language: LspLanguage::Python,
        label: "Python",
        server_name: "pylsp",
        extensions: &["py"],
        command: "pylsp",
        args: &[],
        install_hint: concat!(
            "Install pylsp (python-lsp-server) for Python LSP support.\n\n",
            "Notes:\n",
            "- Use uv if the project uses uv: `uv tool install python-lsp-server`\n",
            "- Or with pip: `pip install python-lsp-server`\n",
            "- Or with pipx: `pipx install python-lsp-server`\n",
            "- Ensure `pylsp` is on PATH after installation",
        ),
        default_preset: true,
    },
    LanguagePreset {
        language: LspLanguage::Go,
        label: "Go",
        server_name: "gopls",
        extensions: &["go"],
        command: "gopls",
        args: &[],
        install_hint: concat!(
            "Install gopls for Go LSP support.\n\n",
            "Notes:\n",
            "- `go install golang.org/x/tools/gopls@latest`\n",
            "- Requires Go to be installed\n",
            "- Ensure `$GOPATH/bin` is on PATH",
        ),
        default_preset: true,
    },
    LanguagePreset {
        language: LspLanguage::Java,
        label: "Java",
        server_name: "jdtls",
        extensions: &["java"],
        command: "jdtls",
        args: &[],
        install_hint: concat!(
            "Install Eclipse JDT Language Server for Java LSP support.\n\n",
            "Notes:\n",
            "- `brew install jdtls` (macOS)\n",
            "- Or download from https://download.eclipse.org/jdtls/\n",
            "- Requires Java JDK to be installed",
        ),
        default_preset: true,
    },
    LanguagePreset {
        language: LspLanguage::CSharp,
        label: "C#",
        server_name: "omnisharp",
        extensions: &["cs"],
        command: "OmniSharp",
        args: &["--languageserver"],
        install_hint: concat!(
            "Install OmniSharp for C# LSP support.\n\n",
            "Notes:\n",
            "- `brew install omnisharp/omnisharp-roslyn/omnisharp-mono` (macOS)\n",
            "- Or download from https://github.com/OmniSharp/omnisharp-roslyn\n",
            "- Requires .NET SDK to be installed",
        ),
        default_preset: false,
    },
    LanguagePreset {
        language: LspLanguage::Cpp,
        label: "C/C++",
        server_name: "clangd",
        extensions: &["c", "cpp", "cc", "cxx", "h", "hpp", "hxx"],
        command: "clangd",
        args: &[],
        install_hint: concat!(
            "Install clangd for C/C++ LSP support.\n\n",
            "Notes:\n",
            "- On macOS: `xcode-select --install` (includes clangd)\n",
            "- Or `brew install llvm` and add to PATH\n",
            "- On Linux: install via your package manager (e.g. `apt install clangd`)",
        ),
        default_preset: true,
    },
    LanguagePreset {
        language: LspLanguage::Ruby,
        label: "Ruby",
        server_name: "solargraph",
        extensions: &["rb"],
        command: "solargraph",
        args: &["stdio"],
        install_hint: concat!(
            "Install Solargraph for Ruby LSP support.\n\n",
            "Notes:\n",
            "- `gem install solargraph`\n",
            "- Or with bundler: add `gem 'solargraph'` to Gemfile\n",
            "- Requires Ruby to be installed",
        ),
        default_preset: false,
    },
    LanguagePreset {
        language: LspLanguage::Swift,
        label: "Swift",
        server_name: "sourcekit-lsp",
        extensions: &["swift"],
        command: "sourcekit-lsp",
        args: &[],
        install_hint: concat!(
            "Install SourceKit-LSP for Swift LSP support.\n\n",
            "Notes:\n",
            "- Included with Xcode: `xcode-select --install`\n",
            "- Or install Swift toolchain from swift.org\n",
            "- Usually at `/usr/bin/sourcekit-lsp` after Xcode install",
        ),
        default_preset: false,
    },
    LanguagePreset {
        language: LspLanguage::Kotlin,
        label: "Kotlin",
        server_name: "kotlin-language-server",
        extensions: &["kt", "kts"],
        command: "kotlin-language-server",
        args: &[],
        install_hint: concat!(
            "Install kotlin-language-server for Kotlin LSP support.\n\n",
            "Notes:\n",
            "- `brew install kotlin-language-server` (macOS)\n",
            "- Or build from source: https://github.com/fwcd/kotlin-language-server\n",
            "- Requires JDK to be installed",
        ),
        default_preset: false,
    },
    LanguagePreset {
        language: LspLanguage::Elixir,
        label: "Elixir",
        server_name: "elixir-ls",
        extensions: &["ex", "exs"],
        command: "elixir-ls",
        args: &[],
        install_hint: concat!(
            "Install ElixirLS for Elixir LSP support.\n\n",
            "Notes:\n",
            "- Download from https://github.com/elixir-lsp/elixir-ls/releases\n",
            "- Requires Elixir and Erlang/OTP to be installed\n",
            "- Ensure the `elixir-ls` script is on PATH",
        ),
        default_preset: false,
    },
    LanguagePreset {
        language: LspLanguage::Zig,
        label: "Zig",
        server_name: "zls",
        extensions: &["zig"],
        command: "zls",
        args: &[],
        install_hint: concat!(
            "Install ZLS for Zig LSP support.\n\n",
            "Notes:\n",
            "- `brew install zls` (macOS) or download from https://github.com/zigtools/zls\n",
            "- Requires Zig to be installed\n",
            "- Ensure version matches your Zig compiler version",
        ),
        default_preset: false,
    },
    LanguagePreset {
        language: LspLanguage::Lua,
        label: "Lua",
        server_name: "lua-language-server",
        extensions: &["lua"],
        command: "lua-language-server",
        args: &[],
        install_hint: concat!(
            "Install lua-language-server for Lua LSP support.\n\n",
            "Notes:\n",
            "- `brew install lua-language-server` (macOS)\n",
            "- Or download from https://github.com/LuaLS/lua-language-server\n",
            "- No Lua runtime required (self-contained)",
        ),
        default_preset: false,
    },
];

fn preset_for(lang: LspLanguage) -> &'static LanguagePreset {
    PRESETS
        .iter()
        .find(|p| p.language == lang)
        .expect("all LspLanguage variants covered in PRESETS")
}

impl LspLanguage {
    pub fn label(&self) -> &'static str {
        preset_for(*self).label
    }

    pub fn all() -> &'static [LspLanguage] {
        use LspLanguage::*;
        &[
            Rust, TypeScript, Python, Go, Java, CSharp, Cpp, Ruby, Swift, Kotlin, Elixir, Zig, Lua,
        ]
    }
}

// ---------------------------------------------------------------------------
// Config builders derived from PRESETS
// ---------------------------------------------------------------------------

fn preset_to_config(p: &LanguagePreset, enabled: bool) -> LspServerConfig {
    LspServerConfig {
        id: format!("preset-{}", p.server_name),
        name: p.server_name.into(),
        extensions: p.extensions.iter().map(|&s| s.into()).collect(),
        command: p.command.into(),
        args: p.args.iter().map(|&s| s.into()).collect(),
        env: vec![],
        enabled,
    }
}

pub(crate) fn lsp_servers_for_language(lang: LspLanguage) -> Vec<LspServerConfig> {
    vec![preset_to_config(preset_for(lang), true)]
}

/// Return an install hint for a known LSP server name.
/// Used to generate helpful cue prompts when the binary is not found.
pub(crate) fn lsp_install_hint(server_name: &str) -> String {
    PRESETS
        .iter()
        .find(|p| p.server_name == server_name || p.command == server_name)
        .map(|p| p.install_hint.to_string())
        .unwrap_or_else(|| {
            format!(
                "Install the '{}' language server.\n\n\
                 Check the server's documentation for installation instructions \
                 and ensure the binary is on PATH.",
                server_name
            )
        })
}

/// Well-known language server presets (all disabled by default for settings).
pub(crate) fn default_lsp_servers() -> Vec<LspServerConfig> {
    PRESETS
        .iter()
        .filter(|p| p.default_preset)
        .map(|p| preset_to_config(p, false))
        .collect()
}
