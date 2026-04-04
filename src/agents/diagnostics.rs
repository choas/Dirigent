use regex::Regex;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Diagnostic (parsed from cargo JSON output)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Diagnostic {
    pub file: String,
    pub line: usize,
    pub col: Option<usize>,
    pub message: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Severity {
    Error,
    Warning,
    Info,
}

// ---------------------------------------------------------------------------
// Cargo JSON diagnostic parser
// ---------------------------------------------------------------------------

/// Parse compiler/clippy diagnostics from `cargo --message-format=json` output.
pub(super) fn parse_cargo_diagnostics(output: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let value = match serde_json::from_str::<serde_json::Value>(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        extract_cargo_message_diagnostics(&value, &mut diagnostics);
    }
    diagnostics
}

/// Extract diagnostics from a single parsed cargo JSON message.
fn extract_cargo_message_diagnostics(value: &serde_json::Value, diagnostics: &mut Vec<Diagnostic>) {
    // Cargo wraps compiler messages in {"reason":"compiler-message","message":{...}}
    let msg = if value.get("reason").and_then(|r| r.as_str()) == Some("compiler-message") {
        match value.get("message") {
            Some(m) => m,
            None => return,
        }
    } else {
        // Direct rustc JSON diagnostic
        value
    };

    let message_text = match msg.get("message").and_then(|m| m.as_str()) {
        Some(t) => t,
        None => return,
    };

    let severity = match msg.get("level").and_then(|l| l.as_str()) {
        Some("error") => Severity::Error,
        Some("warning") => Severity::Warning,
        _ => Severity::Info,
    };

    let spans = match msg.get("spans").and_then(|s| s.as_array()) {
        Some(s) => s,
        None => return,
    };

    for span in spans {
        collect_span_diagnostic(span, spans.len(), message_text, severity, diagnostics);
    }
}

/// Process a single span entry and push a diagnostic if it qualifies.
fn collect_span_diagnostic(
    span: &serde_json::Value,
    span_count: usize,
    message_text: &str,
    severity: Severity,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let is_primary = span.get("is_primary").and_then(|p| p.as_bool()) == Some(true);
    if !is_primary && span_count > 1 {
        return;
    }
    let file = match span.get("file_name").and_then(|f| f.as_str()) {
        Some(f) => f,
        None => return,
    };
    let line = match span.get("line_start").and_then(|l| l.as_u64()) {
        Some(l) => l as usize,
        None => return,
    };
    let col = span
        .get("column_start")
        .and_then(|c| c.as_u64())
        .map(|c| c as usize);
    diagnostics.push(Diagnostic {
        file: file.to_string(),
        line,
        col,
        message: message_text.to_string(),
        severity,
    });
}

// ---------------------------------------------------------------------------
// Generic diagnostic parser (file:line:col: severity: message)
// ---------------------------------------------------------------------------

/// Parse diagnostics from generic compiler/linter output using common patterns:
/// - `file:line:col: error: message` (gcc, clang, rustc, tsc, swiftc)
/// - `file:line:col: warning: message`
/// - `file:line: error: message` (without column)
/// - `file(line,col): error message` (MSVC-style)
/// - `file:line: message` (generic, treated as error if process failed)
pub(super) fn parse_generic_diagnostics(output: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Pattern: file:line:col: severity: message
    // Matches: src/main.rs:10:5: error: something went wrong
    //          src/app.ts(15,3): error TS2345: something
    let re = Regex::new(
        r"(?m)^(.+?):(\d+)(?::(\d+))?:\s*(?:(error|warning|warn|info|note|hint))(?:\[.*?\])?:\s*(.+)$"
    ).expect("hardcoded diagnostic regex");

    // MSVC / TypeScript pattern: file(line,col): error CODE: message
    let re_paren =
        Regex::new(r"(?m)^(.+?)\((\d+),(\d+)\):\s*(?:(error|warning))(?:\s+\w+)?:\s*(.+)$")
            .expect("hardcoded diagnostic regex");

    for cap in re.captures_iter(output) {
        let file = cap[1].to_string();
        // Skip lines that look like URLs or stack traces
        if file.starts_with("http") || file.starts_with("    ") || file.starts_with("\t") {
            continue;
        }
        let line: usize = cap[2].parse().unwrap_or(0);
        if line == 0 {
            continue;
        }
        let col = cap.get(3).and_then(|m| m.as_str().parse().ok());
        let severity = match &cap[4] {
            "error" => Severity::Error,
            "warning" | "warn" => Severity::Warning,
            _ => Severity::Info,
        };
        let message = cap[5].trim().to_string();
        diagnostics.push(Diagnostic {
            file,
            line,
            col,
            message,
            severity,
        });
    }

    for cap in re_paren.captures_iter(output) {
        let file = cap[1].to_string();
        let line: usize = cap[2].parse().unwrap_or(0);
        if line == 0 {
            continue;
        }
        let col: Option<usize> = cap[3].parse().ok();
        let severity = match &cap[4] {
            "error" => Severity::Error,
            _ => Severity::Warning,
        };
        let message = cap[5].trim().to_string();
        diagnostics.push(Diagnostic {
            file,
            line,
            col,
            message,
            severity,
        });
    }

    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cargo_diagnostics_empty() {
        assert!(parse_cargo_diagnostics("").is_empty());
        assert!(parse_cargo_diagnostics("not json").is_empty());
    }

    #[test]
    fn parse_cargo_compiler_message() {
        let json = r#"{"reason":"compiler-message","message":{"message":"unused variable: `x`","level":"warning","spans":[{"file_name":"src/main.rs","line_start":10,"column_start":5,"is_primary":true}]}}"#;
        let diags = parse_cargo_diagnostics(json);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].file, "src/main.rs");
        assert_eq!(diags[0].line, 10);
        assert_eq!(diags[0].col, Some(5));
        assert_eq!(diags[0].severity, Severity::Warning);
    }

    #[test]
    fn parse_generic_gcc_style() {
        let output = "src/main.c:42:10: error: expected ';' after expression\n";
        let diags = parse_generic_diagnostics(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].file, "src/main.c");
        assert_eq!(diags[0].line, 42);
        assert_eq!(diags[0].col, Some(10));
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].message, "expected ';' after expression");
    }

    #[test]
    fn parse_generic_warning() {
        let output = "lib/utils.py:15:1: warning: unused import 'os'\n";
        let diags = parse_generic_diagnostics(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Severity::Warning);
    }

    #[test]
    fn parse_generic_no_column() {
        let output = "src/app.rs:100: error: something broke\n";
        let diags = parse_generic_diagnostics(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].col, None);
    }

    #[test]
    fn parse_generic_msvc_style() {
        let output =
            "src/app.ts(15,3): error TS2345: Argument of type 'string' is not assignable\n";
        let diags = parse_generic_diagnostics(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].file, "src/app.ts");
        assert_eq!(diags[0].line, 15);
        assert_eq!(diags[0].col, Some(3));
        assert_eq!(diags[0].severity, Severity::Error);
    }

    #[test]
    fn parse_generic_empty() {
        assert!(parse_generic_diagnostics("").is_empty());
        assert!(parse_generic_diagnostics("all good!").is_empty());
    }
}
