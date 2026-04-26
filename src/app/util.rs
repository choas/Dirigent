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

#[cfg(test)]
mod tests {
    use super::*;

    // -- strip_ansi tests --

    #[test]
    fn strip_ansi_plain_text() {
        assert_eq!(strip_ansi("hello"), "hello");
    }

    #[test]
    fn strip_ansi_colored_text() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
    }

    #[test]
    fn strip_ansi_bold_and_color() {
        assert_eq!(strip_ansi("\x1b[1;33mwarning\x1b[0m"), "warning");
    }

    #[test]
    fn strip_ansi_empty_string() {
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn strip_ansi_no_escape_sequences() {
        assert_eq!(strip_ansi("just plain text"), "just plain text");
    }

    #[test]
    fn strip_ansi_non_ascii_preserved() {
        assert_eq!(strip_ansi("\x1b[32mcafé 日本語\x1b[0m"), "café 日本語");
    }

    // -- format_duration_ms tests --

    #[test]
    fn format_duration_ms_zero() {
        assert_eq!(format_duration_ms(0), "0ms");
    }

    #[test]
    fn format_duration_ms_under_one_second() {
        assert_eq!(format_duration_ms(500), "500ms");
        assert_eq!(format_duration_ms(999), "999ms");
    }

    #[test]
    fn format_duration_ms_exactly_one_second() {
        assert_eq!(format_duration_ms(1000), "1.0s");
    }

    #[test]
    fn format_duration_ms_seconds() {
        assert_eq!(format_duration_ms(1500), "1.5s");
        assert_eq!(format_duration_ms(60000), "60.0s");
    }

    #[test]
    fn format_duration_ms_boundary() {
        assert_eq!(format_duration_ms(999), "999ms");
        assert_eq!(format_duration_ms(1000), "1.0s");
    }

    // -- expand_tilde tests --

    #[test]
    fn expand_tilde_no_tilde() {
        assert_eq!(expand_tilde("/usr/local"), PathBuf::from("/usr/local"));
    }

    #[test]
    fn expand_tilde_with_subpath() {
        let result = expand_tilde("~/Documents");
        assert!(result.ends_with("Documents"));
        assert_ne!(result, PathBuf::from("~/Documents"));
    }

    #[test]
    fn expand_tilde_bare_tilde() {
        let result = expand_tilde("~");
        assert_ne!(result, PathBuf::from("~"));
    }

    #[test]
    fn expand_tilde_no_expansion_for_middle_tilde() {
        assert_eq!(expand_tilde("/home/~user"), PathBuf::from("/home/~user"));
    }
}
