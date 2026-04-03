use std::path::Path;
use std::process::Command;

use crate::error::DirigentError;

use super::types::SourceItem;

/// Maximum length for a custom source command string.
pub(super) const MAX_COMMAND_LENGTH: usize = 4096;

/// Timeout for subprocess execution (seconds).
pub(crate) const SUBPROCESS_TIMEOUT_SECS: u64 = 60;

/// Shell metacharacters that could be used for injection.
pub(super) const SHELL_METACHARACTERS: &[char] =
    &['`', '$', '!', ';', '&', '|', '<', '>', '(', ')'];

/// Validate a custom command string for safety.
/// Rejects null bytes, line breaks, control characters (except tab),
/// shell metacharacters, and excessively long commands.
pub(super) fn validate_command(command: &str) -> Result<(), String> {
    if command.is_empty() {
        return Err("empty command".to_string());
    }
    if command.len() > MAX_COMMAND_LENGTH {
        return Err(format!(
            "command exceeds maximum length ({} > {})",
            command.len(),
            MAX_COMMAND_LENGTH
        ));
    }
    if command.contains('\0') {
        return Err("command contains null byte".to_string());
    }
    // Reject newlines/carriage-returns — they could chain commands via sh -c
    if command.contains('\n') || command.contains('\r') {
        return Err("command contains line break".to_string());
    }
    // Reject control characters other than tab
    if let Some(pos) = command.chars().position(|c| c.is_control() && c != '\t') {
        return Err(format!(
            "command contains control character at position {}",
            pos
        ));
    }
    // Reject shell metacharacters to prevent injection
    for &meta in SHELL_METACHARACTERS {
        if command.contains(meta) {
            return Err(format!("command contains shell metacharacter '{}'", meta));
        }
    }
    Ok(())
}

/// Drain a child process's stdout and stderr on background threads so the OS
/// pipe buffers don't fill up and block the child (classic pipe deadlock).
///
/// Returns join handles whose value is the captured bytes.  Pass the results
/// through [`collect_drained`] to get the final `Vec<u8>` values.
pub(crate) fn drain_child_pipes(
    child: &mut std::process::Child,
) -> (
    Option<std::thread::JoinHandle<Vec<u8>>>,
    Option<std::thread::JoinHandle<Vec<u8>>>,
) {
    use std::io::Read;

    let stdout_handle = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out.read_to_end(&mut buf);
            buf
        })
    });
    let stderr_handle = child.stderr.take().map(|mut err| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = err.read_to_end(&mut buf);
            buf
        })
    });
    (stdout_handle, stderr_handle)
}

/// Collect the output from drain handles returned by [`drain_child_pipes`].
pub(crate) fn collect_drained(handle: Option<std::thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    handle.and_then(|h| h.join().ok()).unwrap_or_default()
}

/// Run a command with a timeout. Returns the output or an IO error on timeout.
///
/// Reads stdout and stderr on separate threads to avoid deadlocking when the
/// child produces more output than the OS pipe buffer can hold (~64 KB on macOS).
pub(crate) fn output_with_timeout(
    mut child: std::process::Child,
    timeout: std::time::Duration,
) -> std::io::Result<std::process::Output> {
    let (stdout_handle, stderr_handle) = drain_child_pipes(&mut child);

    // Poll for process exit with a timeout.
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "subprocess timed out",
                ));
            }
            None => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    };

    let stdout = collect_drained(stdout_handle);
    let stderr = collect_drained(stderr_handle);

    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Parse JSON output from `gh api --paginate`.
/// When paginating, `gh` may concatenate multiple JSON arrays: `[...][...]`.
/// This function handles both a single valid array and concatenated arrays.
pub(crate) fn parse_paginated_json(raw: &str) -> crate::error::Result<Vec<serde_json::Value>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    // Fast path: valid single JSON array
    if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(trimmed) {
        return Ok(arr);
    }
    // Slow path: concatenated arrays — use streaming deserializer to handle `[...][...]`
    let mut items = Vec::new();
    let stream = serde_json::Deserializer::from_str(trimmed).into_iter::<Vec<serde_json::Value>>();
    for result in stream {
        let arr = result?;
        items.extend(arr);
    }
    Ok(items)
}

/// Fetch items from a custom command source.
/// The command should output JSON: either an array of objects or one object per line.
/// Each object should have "id" and "text" fields.
pub(crate) fn fetch_custom_command(
    project_root: &Path,
    command: &str,
    source_label: &str,
    source_id: &str,
) -> crate::error::Result<Vec<SourceItem>> {
    if let Err(e) = validate_command(command) {
        return Err(DirigentError::Source(format!(
            "refusing to run custom source command: {}",
            e
        )));
    }

    let child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(project_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let timeout = std::time::Duration::from_secs(SUBPROCESS_TIMEOUT_SECS);
    let output = output_with_timeout(child, timeout)?;

    if !output.status.success() {
        return Err(DirigentError::Source(format!(
            "custom source command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    parse_source_json(&json_str, source_label, source_id)
}

/// Parse newline-delimited JSON objects into source items.
fn parse_ndjson_items(json_str: &str, source_label: &str, source_id: &str) -> Vec<SourceItem> {
    json_str
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let obj: serde_json::Value = serde_json::from_str(line).ok()?;
            parse_source_object(&obj, source_label, source_id)
        })
        .collect()
}

/// Parse JSON output from a source command.
/// Supports a single JSON array, concatenated arrays (`[...][...]`), or
/// newline-delimited JSON objects.  Each object must have "id" and "text" fields.
pub(super) fn parse_source_json(
    json_str: &str,
    source_label: &str,
    source_id: &str,
) -> crate::error::Result<Vec<SourceItem>> {
    // Try paginated (possibly concatenated) JSON arrays first
    match parse_paginated_json(json_str) {
        Ok(paginated) if !paginated.is_empty() => {
            return Ok(paginated
                .iter()
                .filter_map(|obj| parse_source_object(obj, source_label, source_id))
                .collect());
        }
        Ok(_) => {} // empty result, fall through to NDJSON
        Err(paginated_err) => {
            // Paginated parsing failed; try NDJSON before propagating the error
            let ndjson_items = parse_ndjson_items(json_str, source_label, source_id);
            if !ndjson_items.is_empty() {
                return Ok(ndjson_items);
            }
            return Err(paginated_err);
        }
    }

    // Try newline-delimited JSON
    Ok(parse_ndjson_items(json_str, source_label, source_id))
}

pub(super) fn parse_source_object(
    obj: &serde_json::Value,
    source_label: &str,
    source_id: &str,
) -> Option<SourceItem> {
    let id = obj.get("id")?.as_str()?;
    let text = obj.get("text")?.as_str()?;
    Some(SourceItem {
        external_id: id.to_string(),
        text: text.to_string(),
        source_label: source_label.to_string(),
        source_id: source_id.to_string(),
        file_path: String::new(),
        line_number: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- validate_command --

    #[test]
    fn validate_command_accepts_simple() {
        assert!(validate_command("echo hello").is_ok());
    }

    #[test]
    fn validate_command_rejects_empty() {
        assert!(validate_command("").is_err());
    }

    #[test]
    fn validate_command_rejects_null_byte() {
        assert!(validate_command("echo\0hello").is_err());
    }

    #[test]
    fn validate_command_rejects_too_long() {
        let long = "a".repeat(MAX_COMMAND_LENGTH + 1);
        assert!(validate_command(&long).is_err());
    }

    #[test]
    fn validate_command_rejects_control_chars() {
        assert!(validate_command("echo \x01 hi").is_err());
    }

    #[test]
    fn validate_command_rejects_newlines() {
        assert!(validate_command("echo hello\nrm -rf /").is_err());
        assert!(validate_command("echo hello\r\nrm -rf /").is_err());
    }

    #[test]
    fn validate_command_rejects_shell_metacharacters() {
        for &meta in SHELL_METACHARACTERS {
            let cmd = format!("echo {}foo", meta);
            assert!(validate_command(&cmd).is_err(), "should reject '{}'", meta);
        }
    }

    #[test]
    fn validate_command_allows_safe_characters() {
        assert!(validate_command("python3 script.py --flag=value 'arg' \"arg2\"").is_ok());
        assert!(validate_command("curl https://example.com/api").is_ok());
    }

    // -- parse_source_json --

    #[test]
    fn parse_json_array() {
        let json = r#"[{"id":"1","text":"first"},{"id":"2","text":"second"}]"#;
        let items = parse_source_json(json, "test", "src-1").unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].external_id, "1");
        assert_eq!(items[0].text, "first");
        assert_eq!(items[0].source_label, "test");
        assert_eq!(items[0].source_id, "src-1");
    }

    #[test]
    fn parse_ndjson() {
        let json = "{\"id\":\"a\",\"text\":\"alpha\"}\n{\"id\":\"b\",\"text\":\"beta\"}\n";
        let items = parse_source_json(json, "src", "src-2").unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].external_id, "b");
    }

    #[test]
    fn parse_empty_json() {
        let items = parse_source_json("[]", "test", "src-3").unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn parse_missing_fields_skipped() {
        let json = r#"[{"id":"1"},{"id":"2","text":"ok"}]"#;
        let items = parse_source_json(json, "test", "src-4").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].external_id, "2");
    }

    #[test]
    fn parse_concatenated_arrays() {
        let json = r#"[{"id":"1","text":"first"}][{"id":"2","text":"second"}]"#;
        let items = parse_source_json(json, "test", "src-5").unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].external_id, "1");
        assert_eq!(items[1].external_id, "2");
    }

    #[test]
    fn parse_malformed_concatenated_arrays_returns_error() {
        let json = r#"[{"id":"1","text":"first"}][{"id":"2","text":}]"#;
        assert!(parse_source_json(json, "test", "src-6").is_err());
    }

    // -- parse_source_object --

    #[test]
    fn parse_source_object_valid() {
        let obj: serde_json::Value = serde_json::from_str(r#"{"id":"x","text":"hello"}"#).unwrap();
        let item = parse_source_object(&obj, "lbl", "sid").unwrap();
        assert_eq!(item.external_id, "x");
        assert_eq!(item.text, "hello");
        assert_eq!(item.source_label, "lbl");
        assert_eq!(item.source_id, "sid");
    }

    #[test]
    fn parse_source_object_missing_id() {
        let obj: serde_json::Value = serde_json::from_str(r#"{"text":"hello"}"#).unwrap();
        assert!(parse_source_object(&obj, "lbl", "sid").is_none());
    }

    #[test]
    fn parse_concatenated_arrays_with_brackets_in_strings() {
        // Brackets inside JSON strings must not confuse the parser
        let json = r#"[{"id":"1","text":"has ] bracket"}][{"id":"2","text":"has [ bracket"}]"#;
        let items = parse_source_json(json, "test", "src-7").unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text, "has ] bracket");
        assert_eq!(items[1].text, "has [ bracket");
    }
}
