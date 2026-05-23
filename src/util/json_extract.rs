/// Extract a JSON value (object or array) from text that may contain markdown code fences
/// or surrounding prose.
///
/// Tries, in order: ````json` fence, bare ``` fence whose content starts with `{` or `[`,
/// then the last balanced `{`…`}` or `[`…`]` span.  Returns the trimmed input when nothing
/// matches.
pub fn extract_json(s: &str) -> String {
    if let Some(start) = s.find("```json") {
        let after = &s[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    if let Some(start) = s.find("```") {
        let after = &s[start + 3..];
        if let Some(end) = after.find("```") {
            let inner = after[..end].trim();
            if inner.starts_with('{') || inner.starts_with('[') {
                return inner.to_string();
            }
        }
    }
    if let Some(result) = extract_last_balanced(s, '{', '}') {
        return result;
    }
    if let Some(result) = extract_last_balanced(s, '[', ']') {
        return result;
    }
    s.trim().to_string()
}

/// Find the last balanced `open`…`close` span in `s`.
///
/// Scans backwards to find the last `close` character, then walks forward from
/// each `open` to find a balanced match that ends at that position. This picks
/// the innermost (last) top-level JSON value when the text contains multiple
/// objects — e.g. an echoed prompt example followed by the actual response.
fn extract_last_balanced(s: &str, open: char, close: char) -> Option<String> {
    let bytes = s.as_bytes();
    let end = s.rfind(close)?;
    let open_b = open as u8;
    let close_b = close as u8;

    // Walk backwards from `end` to find the matching `open`.
    let mut depth: i32 = 0;
    let mut start = None;
    for i in (0..=end).rev() {
        if bytes[i] == close_b {
            depth += 1;
        } else if bytes[i] == open_b {
            depth -= 1;
            if depth == 0 {
                start = Some(i);
                break;
            }
        }
    }

    let start = start?;
    if end > start {
        Some(s[start..=end].to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_json() {
        let input = r#"Here is some text {"key": "value"} trailing"#;
        assert_eq!(extract_json(input), r#"{"key": "value"}"#);
    }

    #[test]
    fn json_code_fence() {
        let input = "Some text\n```json\n{\"a\": 1}\n```\nmore";
        assert_eq!(extract_json(input), r#"{"a": 1}"#);
    }

    #[test]
    fn bare_code_fence() {
        let input = "Some text\n```\n{\"a\": 1}\n```\nmore";
        assert_eq!(extract_json(input), r#"{"a": 1}"#);
    }

    #[test]
    fn plain_json() {
        let input = r#"{"key": "value"}"#;
        assert_eq!(extract_json(input), input);
    }

    #[test]
    fn no_json() {
        assert_eq!(extract_json("  hello  "), "hello");
    }

    #[test]
    fn multiple_json_objects_picks_last() {
        let input = r#"Example: {"steps": [{"cue_ids": [3]}]}
Some text in between.
{"steps": [{"cue_ids": [1], "label": "Fix", "rationale": "Real"}]}"#;
        assert_eq!(
            extract_json(input),
            r#"{"steps": [{"cue_ids": [1], "label": "Fix", "rationale": "Real"}]}"#
        );
    }

    #[test]
    fn pty_echo_with_prompt_and_response() {
        let input = "Analyze cues:\n\
                      Output JSON: {\"steps\": [{\"cue_ids\": [3, 7]}]}\n\
                      Rules:\n- ...\n\n\
                      Here is the plan:\n\
                      {\"steps\": [{\"cue_ids\": [1], \"label\": \"Fix\", \"rationale\": \"Done\"}]}";
        assert_eq!(
            extract_json(input),
            r#"{"steps": [{"cue_ids": [1], "label": "Fix", "rationale": "Done"}]}"#
        );
    }

    #[test]
    fn balanced_array_extraction() {
        let input = "Some text [1, 2, 3] more";
        assert_eq!(extract_json(input), "[1, 2, 3]");
    }

    #[test]
    fn nested_braces() {
        let input = r#"{"outer": {"inner": "value"}}"#;
        assert_eq!(extract_json(input), input);
    }
}
