/// Strip ANSI escape sequences (colors, bold, etc.) from a string.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            skip_escape_sequence(&mut chars);
        } else {
            out.push(c);
        }
    }
    out
}

/// Consume one escape sequence from the iterator.
fn skip_escape_sequence(chars: &mut std::str::Chars<'_>) {
    let Some(next) = chars.next() else {
        return;
    };
    if next == '[' {
        // CSI sequence: ESC [ ... final_byte
        // consume parameter bytes (0x30–0x3F), intermediate (0x20–0x2F),
        // then one final byte (0x40–0x7E)
        for c in chars.by_ref() {
            if ('\x40'..='\x7e').contains(&c) {
                break;
            }
        }
    }
    // OSC or other ESC-initiated sequences: the next char was already consumed
}

/// Format a duration in milliseconds to a human-readable string.
pub fn format_duration_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}
