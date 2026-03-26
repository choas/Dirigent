use super::html::{is_skippable_markup, strip_html_tags};

/// Check whether a trimmed line is a severity/category label to skip.
pub(super) fn is_severity_label(trimmed: &str) -> bool {
    trimmed.starts_with("_\u{26a0}") || trimmed.starts_with("_\u{1f41b}")
}

/// Truncate a string to at most `max_len` bytes on a valid UTF-8 boundary,
/// appending "..." if truncated.
pub(super) fn truncate_with_ellipsis(s: &mut String, max_len: usize) {
    if s.len() <= max_len {
        return;
    }
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s.push_str("...");
}

/// Check whether a trimmed line opens a `<details>` or `<summary>` block.
pub(super) fn is_details_open(trimmed: &str) -> bool {
    trimmed.starts_with("<details") || trimmed.starts_with("<summary")
}

/// Check whether a trimmed line should be ignored (markup, labels, or blank).
fn is_ignorable_line(trimmed: &str) -> bool {
    is_skippable_markup(trimmed) || is_severity_label(trimmed) || trimmed.is_empty()
}

/// Strip HTML tags from a non-ignorable line, returning `None` if nothing remains.
fn clean_line(trimmed: &str) -> Option<String> {
    if is_ignorable_line(trimmed) {
        return None;
    }
    let cleaned = strip_html_tags(trimmed);
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return None;
    }
    Some(cleaned.to_string())
}

/// Extract a clean finding text from a review comment body.
/// Strips HTML tags, diff blocks, and suggestion blocks to get the core message.
pub(super) fn extract_finding_text(body: &str) -> String {
    let mut lines_out = Vec::new();
    let mut in_details = false;
    let mut in_code_block = false;

    for line in body.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }

        if is_details_open(trimmed) {
            in_details = true;
            continue;
        }
        if trimmed == "</details>" {
            in_details = false;
            continue;
        }
        if in_details {
            continue;
        }

        if let Some(cleaned) = clean_line(trimmed) {
            lines_out.push(cleaned);
        }
    }

    let mut result = lines_out.join("\n");
    truncate_with_ellipsis(&mut result, 2000);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_finding_text_strips_code_blocks() {
        let body = "**Bug:** Something is wrong.\n\n```rust\nlet x = 1;\n```\n\nPlease fix.";
        let text = extract_finding_text(body);
        assert!(text.contains("Bug:"));
        assert!(text.contains("Please fix"));
        assert!(!text.contains("let x"));
    }

    #[test]
    fn extract_finding_text_strips_details() {
        let body =
            "Finding.\n<details>\n<summary>Details</summary>\nHidden content\n</details>\nVisible.";
        let text = extract_finding_text(body);
        assert!(text.contains("Finding"));
        assert!(text.contains("Visible"));
        assert!(!text.contains("Hidden"));
    }

    #[test]
    fn extract_finding_text_strips_html_tags() {
        let body = r#"<img src="https://example.com/badge.png" height="20" alt="Action required">
1\. Pr cue location lost <code>🐞 Bug</code> <code>✓ Correctness</code>
<pre>
Refreshing an existing PR-sourced cue updates only <b><i>text</i></b> via <b><i>update_cue_text_by_source_ref</i></b>, but
PR inline comment location is no longer embedded in the cue text.
</pre>"#;
        let text = extract_finding_text(body);
        // HTML tags should be stripped, content preserved
        assert!(text.contains("Pr cue location lost"));
        assert!(text.contains("🐞 Bug"));
        assert!(text.contains("Correctness"));
        assert!(text.contains("update_cue_text_by_source_ref"));
        // No raw HTML tags
        assert!(!text.contains("<code>"));
        assert!(!text.contains("</code>"));
        assert!(!text.contains("<b>"));
        assert!(!text.contains("<pre>"));
        assert!(!text.contains("<img"));
    }

    #[test]
    fn extract_finding_text_strips_qodo_decorative_lines() {
        let body = r#"<img src="https://www.qodo.ai/logo.svg" width="80" alt="Qodo Logo">
<br/>
<a href="https://www.qodo.ai"><img src="https://www.qodo.ai/logo.svg" width="80" alt="Qodo Logo"></a>
Actual finding text here."#;
        let text = extract_finding_text(body);
        assert!(text.contains("Actual finding text here"));
        assert!(!text.contains("qodo.ai"));
        assert!(!text.contains("<br"));
    }
}
