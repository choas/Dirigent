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
        }
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
            // (if, for, while, switch, when, catch) which would be spurious symbols:
            //   1) Kotlin `fun` (with optional modifiers)
            //   2) Java-style with at least one modifier keyword, optional return type
            //   3) Package-private Java: recognized return type (primitive or uppercase) before name
            (
                Regex::new(
                    r"^\s*(?:(?:(?:public|private|protected|static|final|abstract|override|open|suspend)\s+)*fun\s+|(?:(?:public|private|protected|static|final|abstract|override|open|suspend)\s+)+(?:[\w<>\[\],]+\s+)*|(?:void|int|long|float|double|boolean|char|byte|short|[A-Z][\w<>\[\],]*)\s+)(\w+)\s*\(",
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

// -- Generic parser --

fn parse_with_patterns(content: &[String], patterns: &[(Regex, SymbolKind)]) -> Vec<FileSymbol> {
    let mut symbols = Vec::new();
    let mut in_block_comment = false;

    for (idx, line) in content.iter().enumerate() {
        let trimmed = line.trim();

        // Track block comments
        if in_block_comment {
            if trimmed.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if trimmed.starts_with("/*") {
            in_block_comment = true;
            if trimmed.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }

        // Skip single-line comments and string-like lines
        if trimmed.starts_with("//")
            || trimmed.starts_with('#')
            || trimmed.starts_with("--")
            || trimmed.starts_with('*')
        {
            continue;
        }

        for (re, kind) in patterns {
            if let Some(caps) = re.captures(line) {
                let name = if *kind == SymbolKind::Impl {
                    // For impl blocks: "Trait for Type" or just "Type"
                    match (caps.get(1), caps.get(2)) {
                        (Some(trait_or_type), Some(target)) => {
                            format!("{} for {}", trait_or_type.as_str(), target.as_str())
                        }
                        (Some(m), None) => m.as_str().to_string(),
                        _ => continue,
                    }
                } else {
                    match caps.get(1) {
                        Some(m) => m.as_str().to_string(),
                        None => continue,
                    }
                };

                if !name.is_empty() {
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
}
