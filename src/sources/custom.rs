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

/// Run a command with a timeout. Returns the output or an IO error on timeout.
///
/// Reads stdout and stderr on separate threads to avoid deadlocking when the
/// child produces more output than the OS pipe buffer can hold (~64 KB on macOS).
pub(crate) fn output_with_timeout(
    mut child: std::process::Child,
    timeout: std::time::Duration,
) -> std::io::Result<std::process::Output> {
    use std::io::Read;

    // Take ownership of the pipe handles so we can read them on background threads.
    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();

    let stdout_thread = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        if let Some(mut out) = stdout_handle {
            out.read_to_end(&mut buf)?;
        }
        Ok(buf)
    });
    let stderr_thread = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        if let Some(mut err) = stderr_handle {
            err.read_to_end(&mut buf)?;
        }
        Ok(buf)
    });

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

    let stdout = stdout_thread.join().unwrap_or_else(|_| Ok(Vec::new()))?;
    let stderr = stderr_thread.join().unwrap_or_else(|_| Ok(Vec::new()))?;

    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Parse JSON output from `gh api --paginate`.
/// When paginating, `gh` may concatenate multiple JSON arrays: `[...][...]`.
/// This function handles both a single valid array and concatenated arrays.
pub(crate) fn parse_paginated_json(raw: &str) -> Vec<serde_json::Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    // Fast path: valid single JSON array
    if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(trimmed) {
        return arr;
    }
    // Slow path: concatenated arrays — split on `][` and parse each chunk
    let mut items = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, ch) in trimmed.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    if let Ok(arr) =
                        serde_json::from_str::<Vec<serde_json::Value>>(&trimmed[start..=i])
                    {
                        items.extend(arr);
                    }
                    start = i + 1;
                }
            }
            _ => {}
        }
    }
    items
}

/// Fetch items from a custom command source.
/// The command should output JSON: either an array of objects or one object per line.
/// Each object should have "id" and "text" fields.
pub(crate) fn fetch_custom_command(
    project_root: &Path,
    command: &str,
    source_label: &str,
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
    Ok(parse_source_json(&json_str, source_label))
}

/// Parse JSON output from a source command.
/// Supports a single JSON array, concatenated arrays (`[...][...]`), or
/// newline-delimited JSON objects.  Each object must have "id" and "text" fields.
pub(super) fn parse_source_json(json_str: &str, source_label: &str) -> Vec<SourceItem> {
    // Try paginated (possibly concatenated) JSON arrays first
    let paginated = parse_paginated_json(json_str);
    if !paginated.is_empty() {
        return paginated
            .iter()
            .filter_map(|obj| parse_source_object(obj, source_label))
            .collect();
    }

    // Try newline-delimited JSON
    json_str
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let obj: serde_json::Value = serde_json::from_str(line).ok()?;
            parse_source_object(&obj, source_label)
        })
        .collect()
}

pub(super) fn parse_source_object(
    obj: &serde_json::Value,
    source_label: &str,
) -> Option<SourceItem> {
    let id = obj.get("id")?.as_str()?;
    let text = obj.get("text")?.as_str()?;
    Some(SourceItem {
        external_id: id.to_string(),
        text: text.to_string(),
        source_label: source_label.to_string(),
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
        let items = parse_source_json(json, "test");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].external_id, "1");
        assert_eq!(items[0].text, "first");
        assert_eq!(items[0].source_label, "test");
    }

    #[test]
    fn parse_ndjson() {
        let json = "{\"id\":\"a\",\"text\":\"alpha\"}\n{\"id\":\"b\",\"text\":\"beta\"}\n";
        let items = parse_source_json(json, "src");
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].external_id, "b");
    }

    #[test]
    fn parse_empty_json() {
        let items = parse_source_json("[]", "test");
        assert!(items.is_empty());
    }

    #[test]
    fn parse_missing_fields_skipped() {
        let json = r#"[{"id":"1"},{"id":"2","text":"ok"}]"#;
        let items = parse_source_json(json, "test");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].external_id, "2");
    }

    #[test]
    fn parse_concatenated_arrays() {
        let json = r#"[{"id":"1","text":"first"}][{"id":"2","text":"second"}]"#;
        let items = parse_source_json(json, "test");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].external_id, "1");
        assert_eq!(items[1].external_id, "2");
    }

    // -- parse_source_object --

    #[test]
    fn parse_source_object_valid() {
        let obj: serde_json::Value = serde_json::from_str(r#"{"id":"x","text":"hello"}"#).unwrap();
        let item = parse_source_object(&obj, "lbl").unwrap();
        assert_eq!(item.external_id, "x");
        assert_eq!(item.text, "hello");
        assert_eq!(item.source_label, "lbl");
    }

    #[test]
    fn parse_source_object_missing_id() {
        let obj: serde_json::Value = serde_json::from_str(r#"{"text":"hello"}"#).unwrap();
        assert!(parse_source_object(&obj, "lbl").is_none());
    }
}
