use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone)]
pub(crate) struct FileSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub line: usize,  // 1-based
    pub depth: usize, // indentation level (0 = top-level)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Class,
    Interface,
    Constant,
    Module,
    Type,
    Heading,
}

impl SymbolKind {
    pub fn icon(&self) -> &'static str {
        match self {
            SymbolKind::Function => "\u{0192}", // ƒ
            SymbolKind::Struct => "S",
            SymbolKind::Enum => "E",
            SymbolKind::Trait | SymbolKind::Interface => "T",
            SymbolKind::Impl => "I",
            SymbolKind::Class => "C",
            SymbolKind::Constant => "K",
            SymbolKind::Module => "M",
            SymbolKind::Type => "T",
            SymbolKind::Heading => "H",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SymbolKind::Function => "fn",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Interface => "interface",
            SymbolKind::Impl => "impl",
            SymbolKind::Class => "class",
            SymbolKind::Constant => "const",
            SymbolKind::Module => "mod",
            SymbolKind::Type => "type",
            SymbolKind::Heading => "",
        }
    }
}

/// Convert LSP document symbols to FileSymbol.
pub(super) fn from_lsp_symbols(lsp_syms: &[crate::lsp::LspDocumentSymbol]) -> Vec<FileSymbol> {
    lsp_syms
        .iter()
        .map(|s| FileSymbol {
            name: s.name.clone(),
            kind: lsp_symbol_kind_to_internal(s.kind),
            line: s.line,
            depth: s.depth,
        })
        .collect()
}

/// Map LSP SymbolKind to our internal SymbolKind.
fn lsp_symbol_kind_to_internal(kind: lsp_types::SymbolKind) -> SymbolKind {
    match kind {
        lsp_types::SymbolKind::FUNCTION | lsp_types::SymbolKind::METHOD => SymbolKind::Function,
        lsp_types::SymbolKind::STRUCT => SymbolKind::Struct,
        lsp_types::SymbolKind::ENUM => SymbolKind::Enum,
        lsp_types::SymbolKind::ENUM_MEMBER => SymbolKind::Constant,
        lsp_types::SymbolKind::INTERFACE => SymbolKind::Interface,
        lsp_types::SymbolKind::CLASS => SymbolKind::Class,
        lsp_types::SymbolKind::CONSTANT => SymbolKind::Constant,
        lsp_types::SymbolKind::MODULE | lsp_types::SymbolKind::NAMESPACE => SymbolKind::Module,
        lsp_types::SymbolKind::TYPE_PARAMETER => SymbolKind::Type,
        _ => SymbolKind::Function, // default fallback
    }
}

/// Parse symbols from file content based on file extension.
pub(super) fn parse_symbols(content: &[String], ext: &str) -> Vec<FileSymbol> {
    match ext {
        "rs" => parse_rust_symbols(content),
        "py" => parse_python_symbols(content),
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "mts" => parse_js_ts_symbols(content),
        "go" => parse_go_symbols(content),
        "java" | "kt" | "kts" => parse_java_kotlin_symbols(content),
        "c" | "cpp" | "cc" | "cxx" | "h" | "hpp" | "hxx" => parse_c_cpp_symbols(content),
        "rb" => parse_ruby_symbols(content),
        "swift" => parse_swift_symbols(content),
        "cs" => parse_csharp_symbols(content),
        "ex" | "exs" => parse_elixir_symbols(content),
        "zig" => parse_zig_symbols(content),
        "lua" => parse_lua_symbols(content),
        "md" | "mdx" => parse_markdown_headings(content),
        _ => Vec::new(),
    }
}

/// Find the innermost enclosing symbol at a given line number.
pub(super) fn enclosing_symbol(symbols: &[FileSymbol], line: usize) -> Option<&FileSymbol> {
    symbols.iter().rev().find(|s| s.line <= line)
}

/// Build definition-search regex patterns for a symbol name.
pub(super) fn definition_patterns(name: &str) -> Vec<Regex> {
    let escaped = regex::escape(name);
    [
        format!(r"\bfn\s+{}\b", escaped),
        format!(r"\bstruct\s+{}\b", escaped),
        format!(r"\benum\s+{}\b", escaped),
        format!(r"\btrait\s+{}\b", escaped),
        format!(r"\bclass\s+{}\b", escaped),
        format!(r"\binterface\s+{}\b", escaped),
        format!(r"\bdef\s+{}\b", escaped),
        format!(r"\bfunction\s+{}\b", escaped),
        format!(r"\bfunc\s+{}\b", escaped),
        format!(r"\btype\s+{}[\s<]", escaped),
        format!(r"\bconst\s+{}\b", escaped),
        format!(r"\bmod\s+{}\b", escaped),
        format!(r"\bmodule\s+{}\b", escaped),
        format!(r"\bimpl\b.*\b{}\b", escaped),
    ]
    .into_iter()
    .filter_map(|p| Regex::new(&p).ok())
    .collect()
}

// -- Language-specific parsers --

fn parse_rust_symbols(content: &[String]) -> Vec<FileSymbol> {
    static RE: LazyLock<Vec<(Regex, SymbolKind)>> = LazyLock::new(|| {
        vec![
            (
                Regex::new(r"^\s*(?:pub(?:\([\w:]+\))?\s+)?(?:async\s+)?fn\s+(\w+)").unwrap(),
                SymbolKind::Function,
            ),
            (
                Regex::new(r"^\s*(?:pub(?:\([\w:]+\))?\s+)?struct\s+(\w+)").unwrap(),
                SymbolKind::Struct,
            ),
            (
                Regex::new(r"^\s*(?:pub(?:\([\w:]+\))?\s+)?enum\s+(\w+)").unwrap(),
                SymbolKind::Enum,
            ),
            (
                Regex::new(r"^\s*(?:pub(?:\([\w:]+\))?\s+)?trait\s+(\w+)").unwrap(),
                SymbolKind::Trait,
            ),
            (
                Regex::new(r"^\s*impl(?:<[^>]*>)?\s+(\w+(?:::\w+)*)(?:\s+for\s+(\w+))?").unwrap(),
                SymbolKind::Impl,
            ),
            (
                Regex::new(r"^\s*(?:pub(?:\([\w:]+\))?\s+)?mod\s+(\w+)").unwrap(),
                SymbolKind::Module,
            ),
            (
                Regex::new(r"^\s*(?:pub(?:\([\w:]+\))?\s+)?const\s+([A-Z_]\w*)").unwrap(),
                SymbolKind::Constant,
            ),
            (
                Regex::new(r"^\s*(?:pub(?:\([\w:]+\))?\s+)?type\s+(\w+)").unwrap(),
                SymbolKind::Type,
            ),
        ]
    });
    parse_with_patterns(content, &RE)
}

fn parse_python_symbols(content: &[String]) -> Vec<FileSymbol> {
    static RE: LazyLock<Vec<(Regex, SymbolKind)>> = LazyLock::new(|| {
        vec![
            (
                Regex::new(r"^\s*(?:async\s+)?def\s+(\w+)").unwrap(),
                SymbolKind::Function,
            ),
            (Regex::new(r"^\s*class\s+(\w+)").unwrap(), SymbolKind::Class),
        ]
    });
    parse_with_patterns(content, &RE)
}

fn parse_js_ts_symbols(content: &[String]) -> Vec<FileSymbol> {
    static RE: LazyLock<Vec<(Regex, SymbolKind)>> = LazyLock::new(|| {
        vec![
            (
                Regex::new(r"^\s*(?:export\s+)?(?:async\s+)?function\s+(\w+)").unwrap(),
                SymbolKind::Function,
            ),
            (
                Regex::new(r"^\s*(?:export\s+)?(?:default\s+)?class\s+(\w+)").unwrap(),
                SymbolKind::Class,
            ),
            (
                Regex::new(r"^\s*(?:export\s+)?interface\s+(\w+)").unwrap(),
                SymbolKind::Interface,
            ),
            (
                Regex::new(r"^\s*(?:export\s+)?type\s+(\w+)\s*[=<]").unwrap(),
                SymbolKind::Type,
            ),
            (
                Regex::new(r"^\s*(?:export\s+)?enum\s+(\w+)").unwrap(),
                SymbolKind::Enum,
            ),
            // Arrow functions: const foo = (...) =>
            (
                Regex::new(
                    r"^\s*(?:export\s+)?(?:const|let|var)\s+(\w+)\s*=\s*(?:async\s+)?(?:\([^)]*\)|[a-zA-Z_]\w*)\s*=>",
                )
                .unwrap(),
                SymbolKind::Function,
            ),
        ]
    });
    parse_with_patterns(content, &RE)
}

fn parse_go_symbols(content: &[String]) -> Vec<FileSymbol> {
    static RE: LazyLock<Vec<(Regex, SymbolKind)>> = LazyLock::new(|| {
        vec![
            (
                Regex::new(r"^func\s+(?:\([^)]+\)\s+)?(\w+)").unwrap(),
                SymbolKind::Function,
            ),
            (
                Regex::new(r"^type\s+(\w+)\s+struct\b").unwrap(),
                SymbolKind::Struct,
            ),
            (
                Regex::new(r"^type\s+(\w+)\s+interface\b").unwrap(),
                SymbolKind::Interface,
            ),
            (Regex::new(r"^type\s+(\w+)\s+").unwrap(), SymbolKind::Type),
            (
                Regex::new(r"^(?:const|var)\s+(\w+)").unwrap(),
                SymbolKind::Constant,
            ),
        ]
    });
    parse_with_patterns(content, &RE)
}

fn parse_java_kotlin_symbols(content: &[String]) -> Vec<FileSymbol> {
    static RE: LazyLock<Vec<(Regex, SymbolKind)>> = LazyLock::new(|| {
        vec![
            // Heuristic: not a full AST parse. Three branches to require meaningful
            // context before the captured name and avoid matching control keywords
            // (if, for, while, switch, when, catch) or class/interface/enum declarations:
            //   1) Kotlin `fun` (with optional modifiers)
            //   2) Java-style with at least one modifier keyword and explicit return type
            //      (primitive or uppercase name — excludes class/interface/enum/object)
            //   3) Package-private Java: recognized return type (primitive or uppercase) before name
            (
                Regex::new(
                    r"^\s*(?:(?:(?:public|private|protected|static|final|abstract|override|open|suspend)\s+)*fun\s+|(?:(?:public|private|protected|static|final|abstract|override|open|suspend)\s+)+(?:(?:void|int|long|float|double|boolean|char|byte|short|[A-Z][\w<>\[\],]*)\s+)+|(?:void|int|long|float|double|boolean|char|byte|short|[A-Z][\w<>\[\],]*)\s+)(\w+)\s*\(",
                )
                .unwrap(),
                SymbolKind::Function,
            ),
            (
                Regex::new(
                    r"^\s*(?:(?:public|private|protected|abstract|final|open|data|sealed|inner)\s+)*class\s+(\w+)",
                )
                .unwrap(),
                SymbolKind::Class,
            ),
            (
                Regex::new(
                    r"^\s*(?:(?:public|private|protected)\s+)*interface\s+(\w+)",
                )
                .unwrap(),
                SymbolKind::Interface,
            ),
            (
                Regex::new(
                    r"^\s*(?:(?:public|private|protected)\s+)*enum\s+(?:class\s+)?(\w+)",
                )
                .unwrap(),
                SymbolKind::Enum,
            ),
        ]
    });
    parse_with_patterns(content, &RE)
}

fn parse_c_cpp_symbols(content: &[String]) -> Vec<FileSymbol> {
    static RE: LazyLock<Vec<(Regex, SymbolKind)>> = LazyLock::new(|| {
        vec![
            (
                Regex::new(r"^\s*(?:typedef\s+)?struct\s+(\w+)").unwrap(),
                SymbolKind::Struct,
            ),
            (Regex::new(r"^\s*class\s+(\w+)").unwrap(), SymbolKind::Class),
            (
                Regex::new(r"^\s*enum\s+(?:class\s+)?(\w+)").unwrap(),
                SymbolKind::Enum,
            ),
            (
                Regex::new(r"^\s*namespace\s+(\w+)").unwrap(),
                SymbolKind::Module,
            ),
            (
                Regex::new(r"^\s*#define\s+(\w+)").unwrap(),
                SymbolKind::Constant,
            ),
        ]
    });
    parse_with_patterns(content, &RE)
}

fn parse_ruby_symbols(content: &[String]) -> Vec<FileSymbol> {
    static RE: LazyLock<Vec<(Regex, SymbolKind)>> = LazyLock::new(|| {
        vec![
            (
                Regex::new(r"^\s*def\s+(?:self\.)?(\w+[?!=]?)").unwrap(),
                SymbolKind::Function,
            ),
            (Regex::new(r"^\s*class\s+(\w+)").unwrap(), SymbolKind::Class),
            (
                Regex::new(r"^\s*module\s+(\w+)").unwrap(),
                SymbolKind::Module,
            ),
        ]
    });
    parse_with_patterns(content, &RE)
}

fn parse_swift_symbols(content: &[String]) -> Vec<FileSymbol> {
    static RE: LazyLock<Vec<(Regex, SymbolKind)>> = LazyLock::new(|| {
        vec![
            (
                Regex::new(
                    r"^\s*(?:(?:public|private|internal|open|fileprivate)\s+)?(?:(?:override|static|class|mutating)\s+)*func\s+(\w+)",
                )
                .unwrap(),
                SymbolKind::Function,
            ),
            (
                Regex::new(
                    r"^\s*(?:(?:public|private|internal|open)\s+)?(?:final\s+)?class\s+(\w+)",
                )
                .unwrap(),
                SymbolKind::Class,
            ),
            (
                Regex::new(
                    r"^\s*(?:(?:public|private|internal)\s+)?struct\s+(\w+)",
                )
                .unwrap(),
                SymbolKind::Struct,
            ),
            (
                Regex::new(
                    r"^\s*(?:(?:public|private|internal)\s+)?enum\s+(\w+)",
                )
                .unwrap(),
                SymbolKind::Enum,
            ),
            (
                Regex::new(
                    r"^\s*(?:(?:public|private|internal)\s+)?protocol\s+(\w+)",
                )
                .unwrap(),
                SymbolKind::Interface,
            ),
        ]
    });
    parse_with_patterns(content, &RE)
}

fn parse_csharp_symbols(content: &[String]) -> Vec<FileSymbol> {
    static RE: LazyLock<Vec<(Regex, SymbolKind)>> = LazyLock::new(|| {
        vec![
            (
                Regex::new(
                    r"^\s*(?:(?:public|private|protected|internal|abstract|sealed|static|partial)\s+)*class\s+(\w+)",
                )
                .unwrap(),
                SymbolKind::Class,
            ),
            (
                Regex::new(
                    r"^\s*(?:(?:public|private|protected|internal)\s+)*interface\s+(\w+)",
                )
                .unwrap(),
                SymbolKind::Interface,
            ),
            (
                Regex::new(
                    r"^\s*(?:(?:public|private|protected|internal)\s+)*struct\s+(\w+)",
                )
                .unwrap(),
                SymbolKind::Struct,
            ),
            (
                Regex::new(
                    r"^\s*(?:(?:public|private|protected|internal)\s+)*enum\s+(\w+)",
                )
                .unwrap(),
                SymbolKind::Enum,
            ),
            (
                Regex::new(r"^\s*namespace\s+(\w+(?:\.\w+)*)").unwrap(),
                SymbolKind::Module,
            ),
        ]
    });
    parse_with_patterns(content, &RE)
}

fn parse_elixir_symbols(content: &[String]) -> Vec<FileSymbol> {
    static RE: LazyLock<Vec<(Regex, SymbolKind)>> = LazyLock::new(|| {
        vec![
            (
                Regex::new(r"^\s*(?:def|defp)\s+(\w+[?!]?)").unwrap(),
                SymbolKind::Function,
            ),
            (
                Regex::new(r"^\s*defmodule\s+([\w.]+)").unwrap(),
                SymbolKind::Module,
            ),
            (
                Regex::new(r"^\s*defmacro\s+(\w+[?!]?)").unwrap(),
                SymbolKind::Function,
            ),
        ]
    });
    parse_with_patterns(content, &RE)
}

fn parse_zig_symbols(content: &[String]) -> Vec<FileSymbol> {
    static RE: LazyLock<Vec<(Regex, SymbolKind)>> = LazyLock::new(|| {
        vec![
            (
                Regex::new(r"^\s*(?:pub\s+)?fn\s+(\w+)").unwrap(),
                SymbolKind::Function,
            ),
            (
                Regex::new(r"^\s*(?:pub\s+)?const\s+(\w+)\s*=\s*(?:struct|packed struct)").unwrap(),
                SymbolKind::Struct,
            ),
            (
                Regex::new(r"^\s*(?:pub\s+)?const\s+(\w+)\s*=\s*enum").unwrap(),
                SymbolKind::Enum,
            ),
            (
                Regex::new(r"^\s*(?:pub\s+)?const\s+([A-Z_]\w*)").unwrap(),
                SymbolKind::Constant,
            ),
        ]
    });
    parse_with_patterns(content, &RE)
}

fn parse_lua_symbols(content: &[String]) -> Vec<FileSymbol> {
    static RE: LazyLock<Vec<(Regex, SymbolKind)>> = LazyLock::new(|| {
        vec![(
            Regex::new(r"^\s*(?:local\s+)?function\s+([\w.:]+)").unwrap(),
            SymbolKind::Function,
        )]
    });
    parse_with_patterns(content, &RE)
}

fn parse_markdown_headings(content: &[String]) -> Vec<FileSymbol> {
    let mut symbols = Vec::new();
    let mut in_code_block = false;

    for (idx, line) in content.iter().enumerate() {
        let trimmed = line.trim_start();

        // Track fenced code blocks to avoid matching headings inside them
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }

        // Match # through #### headings
        let level = trimmed.bytes().take_while(|&b| b == b'#').count();
        if (1..=4).contains(&level) && trimmed.as_bytes().get(level) == Some(&b' ') {
            let name = trimmed[level..].trim().to_string();
            if !name.is_empty() {
                symbols.push(FileSymbol {
                    name,
                    kind: SymbolKind::Heading,
                    line: idx + 1,
                    depth: level - 1, // # = 0, ## = 1, ### = 2, #### = 3
                });
            }
        }
    }
    symbols
}

// -- Comment detection --

/// Returns true if the trimmed line is inside or is a comment, updating
/// `in_block_comment` state for `/* ... */` block comments across lines.
pub(crate) fn is_comment_line(trimmed: &str, in_block_comment: &mut bool) -> bool {
    // Handle block comment state
    if *in_block_comment {
        if let Some(pos) = trimmed.find("*/") {
            // Block comment ends on this line; still treat this line as comment
            let rest = trimmed[pos + 2..].trim();
            *in_block_comment = false;
            // If there's meaningful code after the closing */, don't skip
            return rest.is_empty();
        }
        return true;
    }

    // Check for block comment start
    if trimmed.starts_with("/*") {
        if trimmed.contains("*/") {
            // Single-line block comment like /* foo */
            return true;
        }
        *in_block_comment = true;
        return true;
    }

    // Single-line comment styles
    trimmed.starts_with("//")       // C, C++, Rust, Java, JS, Go, Swift, …
        || trimmed.starts_with('#') // Python, Ruby, Shell, YAML, …
        || trimmed.starts_with("--") // SQL, Haskell, Lua, …
        || trimmed.starts_with('*') // Block comment continuation (e.g. * @param)
        || trimmed.starts_with("\"\"\"") // Python docstrings
        || trimmed.starts_with("'''") // Python docstrings
}

// -- Generic parser --

/// Extract the symbol name from regex captures, handling `impl` blocks specially.
/// Returns `None` when required capture groups are missing.
fn extract_symbol_name(caps: &regex::Captures<'_>, kind: SymbolKind) -> Option<String> {
    if kind == SymbolKind::Impl {
        // For impl blocks: "Trait for Type" or just "Type"
        match (caps.get(1), caps.get(2)) {
            (Some(trait_or_type), Some(target)) => Some(format!(
                "{} for {}",
                trait_or_type.as_str(),
                target.as_str()
            )),
            (Some(m), None) => Some(m.as_str().to_string()),
            _ => None,
        }
    } else {
        caps.get(1).map(|m| m.as_str().to_string())
    }
}

fn parse_with_patterns(content: &[String], patterns: &[(Regex, SymbolKind)]) -> Vec<FileSymbol> {
    let mut symbols = Vec::new();
    let mut in_block_comment = false;

    for (idx, line) in content.iter().enumerate() {
        let trimmed = line.trim();

        if is_comment_line(trimmed, &mut in_block_comment) {
            continue;
        }

        for (re, kind) in patterns {
            let Some(caps) = re.captures(line) else {
                continue;
            };
            let Some(name) = extract_symbol_name(&caps, *kind) else {
                continue;
            };
            if name.is_empty() {
                continue;
            }

            let indent = line.len() - line.trim_start().len();
            let depth = indent / 4;

            symbols.push(FileSymbol {
                name,
                kind: *kind,
                line: idx + 1,
                depth,
            });
            break; // Only match first pattern per line
        }
    }
    symbols
}

/// Extract the word at a given byte offset within a line of text.
pub(super) fn word_at_offset(line: &str, byte_offset: usize) -> Option<&str> {
    if byte_offset >= line.len() || !line.is_char_boundary(byte_offset) {
        return None;
    }

    let bytes = line.as_bytes();
    if !is_word_char(bytes[byte_offset]) {
        return None;
    }

    let start = (0..byte_offset)
        .rev()
        .take_while(|&i| line.is_char_boundary(i) && is_word_char(bytes[i]))
        .last()
        .unwrap_or(byte_offset);

    let end = (byte_offset..bytes.len())
        .take_while(|&i| line.is_char_boundary(i) && is_word_char(bytes[i]))
        .last()
        .map(|i| i + 1)
        .unwrap_or(byte_offset + 1);

    let word = &line[start..end];
    if word.is_empty() {
        None
    } else {
        Some(word)
    }
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- word_at_offset tests --

    #[test]
    fn word_at_offset_simple_ascii() {
        let line = "fn hello_world() {";
        assert_eq!(word_at_offset(line, 0), Some("fn"));
        assert_eq!(word_at_offset(line, 1), Some("fn"));
        assert_eq!(word_at_offset(line, 3), Some("hello_world"));
        assert_eq!(word_at_offset(line, 8), Some("hello_world"));
        assert_eq!(word_at_offset(line, 13), Some("hello_world"));
    }

    #[test]
    fn word_at_offset_on_non_word_char() {
        let line = "fn foo(bar)";
        // '(' is not a word char
        assert_eq!(word_at_offset(line, 6), None);
        // space
        assert_eq!(word_at_offset(line, 2), None);
    }

    #[test]
    fn word_at_offset_past_end() {
        let line = "hello";
        assert_eq!(word_at_offset(line, 5), None);
        assert_eq!(word_at_offset(line, 100), None);
    }

    #[test]
    fn word_at_offset_empty_line() {
        assert_eq!(word_at_offset("", 0), None);
    }

    #[test]
    fn word_at_offset_multibyte_utf8() {
        // "let über = 1;" — 'ü' is 2 bytes (0xC3 0xBC)
        let line = "let über = 1;";
        // byte 4 = start of 'ü' (0xC3) — not ASCII alphanumeric
        assert_eq!(word_at_offset(line, 4), None);
        // byte 5 = continuation byte of 'ü', not a char boundary
        assert_eq!(word_at_offset(line, 5), None);
        // byte 6 = 'b'
        assert_eq!(word_at_offset(line, 6), Some("ber"));
    }

    #[test]
    fn word_at_offset_cjk_characters() {
        // CJK chars are 3 bytes each; they are not ASCII word chars
        let line = "foo 日本語 bar";
        assert_eq!(word_at_offset(line, 0), Some("foo"));
        // byte 4 = start of '日' (3-byte char), not a word char
        assert_eq!(word_at_offset(line, 4), None);
        // byte 5 = continuation byte, not a char boundary
        assert_eq!(word_at_offset(line, 5), None);
        // "bar" starts after "foo " (4) + 3*3 (9) + " " (1) = byte 14
        let bar_start = line.find("bar").unwrap();
        assert_eq!(word_at_offset(line, bar_start), Some("bar"));
    }

    #[test]
    fn word_at_offset_at_boundaries() {
        let line = "a";
        assert_eq!(word_at_offset(line, 0), Some("a"));

        let line = "_";
        assert_eq!(word_at_offset(line, 0), Some("_"));
    }

    #[test]
    fn word_at_offset_underscores() {
        let line = "__init__";
        assert_eq!(word_at_offset(line, 0), Some("__init__"));
        assert_eq!(word_at_offset(line, 4), Some("__init__"));
        assert_eq!(word_at_offset(line, 7), Some("__init__"));
    }

    // -- definition_patterns tests --

    #[test]
    fn definition_patterns_rust_fn() {
        let pats = definition_patterns("my_func");
        let matches_any = |text: &str| pats.iter().any(|re| re.is_match(text));
        assert!(matches_any("fn my_func()"));
        assert!(matches_any("pub fn my_func(x: i32)"));
        assert!(matches_any("  async fn my_func() {"));
    }

    #[test]
    fn definition_patterns_struct_enum_trait() {
        let pats = definition_patterns("Foo");
        let matches_any = |text: &str| pats.iter().any(|re| re.is_match(text));
        assert!(matches_any("struct Foo {"));
        assert!(matches_any("enum Foo {"));
        assert!(matches_any("trait Foo {"));
        assert!(matches_any("impl Foo {"));
    }

    #[test]
    fn definition_patterns_no_false_positive() {
        let pats = definition_patterns("Foo");
        let matches_any = |text: &str| pats.iter().any(|re| re.is_match(text));
        assert!(!matches_any("let foo = 1;"));
        assert!(!matches_any("FooBar::new()"));
    }

    #[test]
    fn definition_patterns_multi_language() {
        let pats = definition_patterns("greet");
        let matches_any = |text: &str| pats.iter().any(|re| re.is_match(text));
        assert!(matches_any("def greet(name):"));
        assert!(matches_any("function greet() {"));
        assert!(matches_any("func greet() {"));
        assert!(matches_any("class greet {"));
        assert!(matches_any("interface greet {"));
    }

    #[test]
    fn definition_patterns_special_regex_chars() {
        let pats = definition_patterns("foo+bar");
        let matches_any = |text: &str| pats.iter().any(|re| re.is_match(text));
        assert!(matches_any("fn foo+bar()"));
        assert!(!matches_any("fn fooXbar()"));
    }

    #[test]
    fn definition_patterns_type_pattern() {
        let pats = definition_patterns("MyType");
        let matches_any = |text: &str| pats.iter().any(|re| re.is_match(text));
        assert!(matches_any("type MyType = i32;"));
        assert!(matches_any("type MyType<T> = Vec<T>;"));
    }

    // -- parse_symbols tests --

    #[test]
    fn parse_symbols_rust_basic() {
        let content: Vec<String> = vec![
            "pub fn hello() {".into(),
            "    let x = 1;".into(),
            "}".into(),
            "struct Point {".into(),
            "    x: f64,".into(),
            "}".into(),
        ];
        let syms = parse_symbols(&content, "rs");
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].name, "hello");
        assert_eq!(syms[0].kind, SymbolKind::Function);
        assert_eq!(syms[0].line, 1);
        assert_eq!(syms[1].name, "Point");
        assert_eq!(syms[1].kind, SymbolKind::Struct);
    }

    #[test]
    fn parse_symbols_python() {
        let content: Vec<String> = vec![
            "class Greeter:".into(),
            "    def greet(self):".into(),
            "        pass".into(),
        ];
        let syms = parse_symbols(&content, "py");
        assert!(syms
            .iter()
            .any(|s| s.name == "Greeter" && s.kind == SymbolKind::Class));
        assert!(syms
            .iter()
            .any(|s| s.name == "greet" && s.kind == SymbolKind::Function));
    }

    #[test]
    fn parse_symbols_unknown_ext_returns_empty() {
        let content: Vec<String> = vec!["some content".into()];
        assert!(parse_symbols(&content, "xyz").is_empty());
    }

    #[test]
    fn parse_symbols_empty_content() {
        assert!(parse_symbols(&[], "rs").is_empty());
    }

    #[test]
    fn parse_symbols_skips_comments() {
        let content: Vec<String> = vec![
            "// fn commented_out() {}".into(),
            "fn real_function() {}".into(),
        ];
        let syms = parse_symbols(&content, "rs");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "real_function");
    }

    #[test]
    fn parse_symbols_javascript() {
        let content: Vec<String> = vec![
            "function render() {".into(),
            "  return null;".into(),
            "}".into(),
            "class App {".into(),
            "}".into(),
        ];
        let syms = parse_symbols(&content, "js");
        assert!(syms.iter().any(|s| s.name == "render"));
        assert!(syms.iter().any(|s| s.name == "App"));
    }

    #[test]
    fn parse_symbols_markdown_headings() {
        let content: Vec<String> = vec![
            "# Title".into(),
            "Some text".into(),
            "## Section".into(),
            "### Subsection".into(),
        ];
        let syms = parse_symbols(&content, "md");
        assert_eq!(syms.len(), 3);
        assert_eq!(syms[0].name, "Title");
        assert_eq!(syms[0].kind, SymbolKind::Heading);
        assert_eq!(syms[1].name, "Section");
        assert_eq!(syms[2].name, "Subsection");
    }

    #[test]
    fn parse_symbols_non_ascii_identifiers() {
        let content: Vec<String> = vec!["def café():".into(), "    pass".into()];
        let syms = parse_symbols(&content, "py");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "café");
    }

    // -- enclosing_symbol tests --

    #[test]
    fn enclosing_symbol_finds_nearest() {
        let symbols = vec![
            FileSymbol {
                name: "foo".into(),
                kind: SymbolKind::Function,
                line: 1,
                depth: 0,
            },
            FileSymbol {
                name: "bar".into(),
                kind: SymbolKind::Function,
                line: 10,
                depth: 0,
            },
            FileSymbol {
                name: "baz".into(),
                kind: SymbolKind::Function,
                line: 20,
                depth: 0,
            },
        ];
        assert_eq!(enclosing_symbol(&symbols, 15).unwrap().name, "bar");
        assert_eq!(enclosing_symbol(&symbols, 10).unwrap().name, "bar");
        assert_eq!(enclosing_symbol(&symbols, 1).unwrap().name, "foo");
        assert_eq!(enclosing_symbol(&symbols, 25).unwrap().name, "baz");
    }

    #[test]
    fn enclosing_symbol_before_first_returns_none() {
        let symbols = vec![FileSymbol {
            name: "foo".into(),
            kind: SymbolKind::Function,
            line: 5,
            depth: 0,
        }];
        assert!(enclosing_symbol(&symbols, 3).is_none());
    }

    #[test]
    fn enclosing_symbol_empty_symbols() {
        assert!(enclosing_symbol(&[], 10).is_none());
    }

    // -- is_comment_line tests --

    #[test]
    fn is_comment_line_single_line_styles() {
        let mut in_block = false;
        assert!(is_comment_line("// rust comment", &mut in_block));
        assert!(is_comment_line("# python comment", &mut in_block));
        assert!(is_comment_line("-- sql comment", &mut in_block));
        assert!(!is_comment_line("let x = 1;", &mut in_block));
    }

    #[test]
    fn is_comment_line_block_comment() {
        let mut in_block = false;
        assert!(is_comment_line("/* start", &mut in_block));
        assert!(in_block);
        assert!(is_comment_line("still in block", &mut in_block));
        assert!(is_comment_line("end */", &mut in_block));
        assert!(!in_block);
        assert!(!is_comment_line("code after block", &mut in_block));
    }

    #[test]
    fn is_comment_line_single_line_block() {
        let mut in_block = false;
        assert!(is_comment_line("/* single line block */", &mut in_block));
        assert!(!in_block);
    }

    #[test]
    fn is_comment_line_block_with_trailing_code() {
        let mut in_block = true;
        assert!(!is_comment_line("*/ int x = 1;", &mut in_block));
        assert!(!in_block);
    }

    #[test]
    fn is_comment_line_python_docstrings() {
        let mut in_block = false;
        assert!(is_comment_line(r#""""docstring""""#, &mut in_block));
        assert!(is_comment_line("'''docstring'''", &mut in_block));
    }

    #[test]
    fn is_comment_line_empty_string() {
        let mut in_block = false;
        assert!(!is_comment_line("", &mut in_block));
    }
}
