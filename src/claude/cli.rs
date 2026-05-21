use std::path::Path;
use std::process::Command;

use super::types::ClaudeError;

/// Resolve environment variable **names** (one per line, # comments allowed)
/// from the current process environment and apply them to the command.
/// Lines containing `=` are treated as bare names (the `=…` suffix is stripped)
/// for backward compatibility with old KEY=VALUE config entries.
///
/// Missing names are surfaced through `on_log` so the GUI run log shows that
/// an expected auth token (e.g. `ANTHROPIC_API_KEY`) was not set, instead of
/// the run failing opaquely.
pub(crate) fn apply_env_vars(cmd: &mut Command, env_vars: &str, on_log: &mut dyn FnMut(&str)) {
    for (name, value) in resolve_env_pairs(env_vars, on_log) {
        cmd.env(name, value);
    }
}

/// Load all KEY=VALUE pairs from `.Dirigent/.env` (relative to `project_root`)
/// and set them on the command's environment. This allows users to maintain a
/// separate `.env` for AI CLI tools without touching the real `.env` used for
/// manual testing and production.
///
/// Lines that are empty, start with `#`, or lack an `=` sign are skipped.
/// Values may be optionally quoted with `"` or `'`.
///
/// On Unix, surfaces a one-time warning via `on_log` if the file is readable
/// by group or other — `.Dirigent/.env` routinely holds API keys, so loose
/// permissions are a real exfiltration risk on shared dev machines.
pub(crate) fn apply_dirigent_env(
    cmd: &mut Command,
    project_root: &Path,
    on_log: &mut dyn FnMut(&str),
) {
    for (key, value) in load_dirigent_env_pairs(project_root, on_log) {
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
///
/// Missing names are reported via `on_log` (and `log::warn!`) so the live run
/// log surfaces an unset auth token instead of the user only seeing an opaque
/// downstream failure.
pub(super) fn resolve_env_pairs(
    env_vars: &str,
    on_log: &mut dyn FnMut(&str),
) -> Vec<(String, String)> {
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
                on_log(&format!(
                    "\u{26A0} env var '{}' not found in environment, skipping\n",
                    name
                ));
            }
        }
    }
    pairs
}

/// Load KEY=VALUE pairs from `.Dirigent/.env` relative to `project_root`.
///
/// Also performs a one-time Unix permission check (see
/// [`warn_if_loose_dirigent_env_permissions`]) and surfaces a warning through
/// `on_log` if the file is group/other-readable.
pub(super) fn load_dirigent_env_pairs(
    project_root: &Path,
    on_log: &mut dyn FnMut(&str),
) -> Vec<(String, String)> {
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
    warn_if_loose_dirigent_env_permissions(&env_path, on_log);
    parse_dotenv_lines(&content)
}

/// Emit a one-time warning if `.Dirigent/.env` is readable by group or other
/// (i.e. mode bits `& 0o077 != 0`). The file routinely holds long-lived API
/// keys (ANTHROPIC_API_KEY, GH tokens, …), so loose permissions are a real
/// exfiltration risk on shared dev machines.
///
/// The warning is fired at most once per process via [`std::sync::Once`] so
/// repeated runs don't spam the live run log.
#[cfg(unix)]
fn warn_if_loose_dirigent_env_permissions(path: &Path, on_log: &mut dyn FnMut(&str)) {
    use std::sync::Once;
    static WARNED: Once = Once::new();

    let Some(msg) = loose_dirigent_env_permission_warning(path) else {
        return;
    };
    WARNED.call_once(|| {
        log::warn!("{}", msg.trim_end());
    });
    on_log(&msg);
}

#[cfg(not(unix))]
fn warn_if_loose_dirigent_env_permissions(_path: &Path, _on_log: &mut dyn FnMut(&str)) {}

/// Pure helper: return `Some(warning)` if `path` is group/other-readable on
/// Unix, `None` otherwise. Split out from the `Once`-gated wrapper so unit
/// tests can exercise the bit math without colliding on the process-wide flag.
#[cfg(unix)]
fn loose_dirigent_env_permission_warning(path: &Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path).ok()?.permissions().mode();
    if mode & 0o077 == 0 {
        return None;
    }
    Some(format!(
        "\u{26A0} {} has loose permissions (mode {:o}); secrets may leak to other users. Run: chmod 600 {}\n",
        path.display(),
        mode & 0o777,
        path.display(),
    ))
}

/// Parse dotenv-style KEY=VALUE lines into pairs.
///
/// Lines that are empty, start with `#`, or lack an `=` sign are skipped.
/// Keys and values are trimmed; values may be optionally quoted with `"` or `'`.
fn parse_dotenv_lines(content: &str) -> Vec<(String, String)> {
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

    #[test]
    fn parse_dotenv_skips_blanks_comments_and_keyless_lines() {
        let content = "\n# comment\nFOO=bar\n   \n=novalue\nBAZ=qux\n";
        let pairs = parse_dotenv_lines(content);
        assert_eq!(
            pairs,
            vec![
                ("FOO".to_string(), "bar".to_string()),
                ("BAZ".to_string(), "qux".to_string()),
            ]
        );
    }

    #[test]
    fn parse_dotenv_strips_surrounding_quotes_and_trims() {
        let content = "  FOO = \"bar\"  \nBAZ='qux'\nKEY=value with spaces\n";
        let pairs = parse_dotenv_lines(content);
        assert_eq!(
            pairs,
            vec![
                ("FOO".to_string(), "bar".to_string()),
                ("BAZ".to_string(), "qux".to_string()),
                ("KEY".to_string(), "value with spaces".to_string()),
            ]
        );
    }

    #[test]
    fn resolve_env_pairs_resolves_names_from_process_env() {
        // Use a name that is overwhelmingly likely to be set on dev machines.
        // We assert that the helper round-trips known-present names, drops
        // unknown ones, and forwards a warning about missing names to on_log.
        std::env::set_var("DIRIGENT_TEST_ENV_VAR", "test_value_123");
        let mut log_buf = String::new();
        let pairs = resolve_env_pairs(
            "DIRIGENT_TEST_ENV_VAR\n# comment\nDIRIGENT_MISSING_VAR\n",
            &mut |s| log_buf.push_str(s),
        );
        std::env::remove_var("DIRIGENT_TEST_ENV_VAR");
        assert_eq!(
            pairs,
            vec![(
                "DIRIGENT_TEST_ENV_VAR".to_string(),
                "test_value_123".to_string()
            )]
        );
        assert!(
            log_buf.contains("DIRIGENT_MISSING_VAR"),
            "missing var should be reported via on_log, got: {log_buf:?}"
        );
        assert!(
            !log_buf.contains("DIRIGENT_TEST_ENV_VAR"),
            "present var should not be reported, got: {log_buf:?}"
        );
    }

    #[test]
    fn resolve_env_pairs_strips_legacy_key_equals_value_suffix() {
        std::env::set_var("DIRIGENT_LEGACY_VAR", "real_value");
        let pairs = resolve_env_pairs("DIRIGENT_LEGACY_VAR=stale_inline_value\n", &mut |_| {});
        std::env::remove_var("DIRIGENT_LEGACY_VAR");
        assert_eq!(
            pairs,
            vec![("DIRIGENT_LEGACY_VAR".to_string(), "real_value".to_string())]
        );
    }

    #[cfg(unix)]
    #[test]
    fn loose_permission_warning_fires_for_group_or_other_readable() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".env");
        std::fs::write(&path, "ANTHROPIC_API_KEY=sk-secret\n").unwrap();

        // 0o644 is the typical default — group + other are readable.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let warning = loose_dirigent_env_permission_warning(&path)
            .expect("0o644 must trigger a warning — group/other can read the file");
        assert!(
            warning.contains("chmod 600"),
            "warning should tell the user how to fix it, got: {warning:?}",
        );
        assert!(
            warning.contains(&path.display().to_string()),
            "warning should name the offending file, got: {warning:?}",
        );

        // 0o600 is the secure permission — owner-only.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            loose_dirigent_env_permission_warning(&path).is_none(),
            "0o600 is the recommended mode and must not warn",
        );

        // 0o604 — group bits clear but other can read. Must still warn.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o604)).unwrap();
        assert!(
            loose_dirigent_env_permission_warning(&path).is_some(),
            "0o604 leaks to `other` and must warn",
        );
    }

    #[cfg(unix)]
    #[test]
    fn loose_permission_warning_silent_for_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(loose_dirigent_env_permission_warning(&tmp.path().join("nonexistent")).is_none());
    }
}
