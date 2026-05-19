use std::path::Path;
use std::process::Command;

use super::types::ClaudeError;

/// Resolve environment variable **names** (one per line, # comments allowed)
/// from the current process environment and apply them to the command.
/// Lines containing `=` are treated as bare names (the `=…` suffix is stripped)
/// for backward compatibility with old KEY=VALUE config entries.
pub(crate) fn apply_env_vars(cmd: &mut Command, env_vars: &str) {
    for line in env_vars.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Accept bare names; strip any =value suffix left over from old config.
        let name = match line.split_once('=') {
            Some((key, _)) => key.trim(),
            None => line,
        };
        if name.is_empty() {
            continue;
        }
        match std::env::var(name) {
            Ok(value) => {
                cmd.env(name, value);
            }
            Err(_) => {
                log::warn!("env var '{}' not found in environment, skipping", name);
            }
        }
    }
}

/// Load all KEY=VALUE pairs from `.Dirigent/.env` (relative to `project_root`)
/// and set them on the command's environment. This allows users to maintain a
/// separate `.env` for AI CLI tools without touching the real `.env` used for
/// manual testing and production.
///
/// Lines that are empty, start with `#`, or lack an `=` sign are skipped.
/// Values may be optionally quoted with `"` or `'`.
pub(crate) fn apply_dirigent_env(cmd: &mut Command, project_root: &Path) {
    let env_path = project_root.join(".Dirigent").join(".env");
    let content = match std::fs::read_to_string(&env_path) {
        Ok(c) => c,
        Err(e) => {
            if env_path.exists() {
                log::warn!(".Dirigent/.env exists but is unreadable: {e}");
            }
            return;
        }
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };
        if key.is_empty() {
            continue;
        }
        let value = strip_surrounding_quotes(value);
        cmd.env(key, value);
    }
}

/// Load a single variable from `.Dirigent/.env`, falling back to `.env`.
///
/// This is the unified lookup used by source integrations (SonarQube, Slack, etc.)
/// so that AI-driven runs and manual runs can use different tokens.
pub(crate) fn load_env_var_with_dirigent_fallback(
    project_root: &Path,
    key: &str,
) -> Option<String> {
    // 1. Try .Dirigent/.env first
    if let Some(v) = load_env_file_var(&project_root.join(".Dirigent").join(".env"), key) {
        return Some(v);
    }
    // 2. Fall back to .env
    load_env_file_var(&project_root.join(".env"), key)
}

/// Strip matching surrounding single or double quotes from a string.
/// Returns the original string unchanged if no matching quotes are found.
fn strip_surrounding_quotes(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| s.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(s)
}

/// Parse a single key from a dotenv-style file.
fn load_env_file_var(path: &Path, key: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let prefix = format!("{}=", key);
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(value) = line.strip_prefix(&prefix) {
            let value = strip_surrounding_quotes(value.trim());
            return Some(value.to_string());
        }
    }
    None
}

/// Resolve environment variable **names** (one per line, `#` comments allowed)
/// from the current process environment and return them as `(key, value)` pairs.
pub(super) fn resolve_env_pairs(env_vars: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for line in env_vars.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let name = match line.split_once('=') {
            Some((key, _)) => key.trim(),
            None => line,
        };
        if name.is_empty() {
            continue;
        }
        match std::env::var(name) {
            Ok(value) => pairs.push((name.to_string(), value)),
            Err(_) => {
                log::warn!("env var '{}' not found in environment, skipping", name);
            }
        }
    }
    pairs
}

/// Load KEY=VALUE pairs from `.Dirigent/.env` relative to `project_root`.
pub(super) fn load_dirigent_env_pairs(project_root: &Path) -> Vec<(String, String)> {
    let env_path = project_root.join(".Dirigent").join(".env");
    let content = match std::fs::read_to_string(&env_path) {
        Ok(c) => c,
        Err(e) => {
            if env_path.exists() {
                log::warn!(".Dirigent/.env exists but is unreadable: {e}");
            }
            return Vec::new();
        }
    };
    let mut pairs = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };
        if key.is_empty() {
            continue;
        }
        let value = strip_surrounding_quotes(value);
        pairs.push((key.to_string(), value.to_string()));
    }
    pairs
}

/// Run a lifecycle script (pre-run or post-run).
///
/// Returns `Err` for pre-run failures (abort the run), logs but ignores
/// post-run failures when `fail_on_error` is false.
pub(super) fn run_lifecycle_script(
    script: &str,
    label: &str,
    project_root: &Path,
    on_log: &mut dyn FnMut(&str),
    fail_on_error: bool,
) -> Result<(), ClaudeError> {
    let trimmed = script.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    on_log(&format!("\u{25B6} {}: {}\n", label, trimmed));
    match Command::new("sh")
        .arg("-c")
        .arg(trimmed)
        .current_dir(project_root)
        .output()
    {
        Ok(output) => handle_script_output(&output, label, on_log, fail_on_error),
        Err(e) => handle_script_error(e, label, on_log, fail_on_error),
    }
}

/// Process successful script execution output.
fn handle_script_output(
    output: &std::process::Output,
    label: &str,
    on_log: &mut dyn FnMut(&str),
    fail_on_error: bool,
) -> Result<(), ClaudeError> {
    if !output.stdout.is_empty() {
        on_log(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        on_log(&String::from_utf8_lossy(&output.stderr));
    }
    if output.status.success() {
        return Ok(());
    }
    let msg = format!("{} script failed (exit {})", label, output.status);
    on_log(&format!("\u{2717} {}\n", msg));
    if fail_on_error {
        return Err(ClaudeError::SpawnFailed(std::io::Error::other(msg)));
    }
    Ok(())
}

/// Handle a script spawn error.
fn handle_script_error(
    e: std::io::Error,
    label: &str,
    on_log: &mut dyn FnMut(&str),
    fail_on_error: bool,
) -> Result<(), ClaudeError> {
    on_log(&format!("\u{2717} {} script error: {}\n", label, e));
    if fail_on_error {
        return Err(ClaudeError::SpawnFailed(e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_double_quotes() {
        assert_eq!(strip_surrounding_quotes(r#""value""#), "value");
    }

    #[test]
    fn strip_single_quotes() {
        assert_eq!(strip_surrounding_quotes("'value'"), "value");
    }

    #[test]
    fn strip_mismatched_quotes_unchanged() {
        assert_eq!(strip_surrounding_quotes(r#""value'"#), r#""value'"#);
        assert_eq!(strip_surrounding_quotes("'value\""), "'value\"");
    }

    #[test]
    fn strip_no_quotes_unchanged() {
        assert_eq!(strip_surrounding_quotes("value"), "value");
    }

    #[test]
    fn strip_empty_string() {
        assert_eq!(strip_surrounding_quotes(""), "");
    }

    #[test]
    fn strip_only_quotes() {
        assert_eq!(strip_surrounding_quotes(r#""""#), "");
        assert_eq!(strip_surrounding_quotes("''"), "");
    }

    #[test]
    fn strip_preserves_inner_quotes() {
        assert_eq!(
            strip_surrounding_quotes(r#""it's a \"test\"""#),
            r#"it's a \"test\""#
        );
    }
}
