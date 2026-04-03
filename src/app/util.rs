use std::path::PathBuf;

/// Expand a leading `~` to the user's home directory.
pub fn expand_tilde(raw: &str) -> PathBuf {
    if raw == "~" || raw.starts_with("~/") || raw.starts_with("~\\") {
        match dirs::home_dir() {
            Some(home) => {
                if raw == "~" {
                    home
                } else {
                    home.join(&raw[2..])
                }
            }
            None => PathBuf::from(raw),
        }
    } else {
        PathBuf::from(raw)
    }
}

/// Strip ANSI escape sequences (colors, bold, etc.) from a string.
pub fn strip_ansi(s: &str) -> String {
    let stripped = strip_ansi_escapes::strip(s);
    String::from_utf8_lossy(&stripped).into_owned()
}

/// Format a duration in milliseconds to a human-readable string.
pub fn format_duration_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}
