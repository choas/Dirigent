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
    let obj = extract_last_balanced_with_pos(s, '{', '}');
    let arr = extract_last_balanced_with_pos(s, '[', ']');
    match (obj, arr) {
        (Some((_, obj_end, obj_str)), Some((_, arr_end, arr_str))) => {
            if arr_end > obj_end {
                arr_str
            } else {
                obj_str
            }
        }
        (Some((_, _, obj_str)), None) => obj_str,
        (None, Some((_, _, arr_str))) => arr_str,
        (None, None) => s.trim().to_string(),
    }
}

/// Find the last balanced `open`…`close` span in `s`, skipping over JSON
/// string literals so that braces inside `"..."` don't affect depth counting.
fn extract_last_balanced_with_pos(s: &str, open: char, close: char) -> Option<(usize, usize, String)> {
    let bytes = s.as_bytes();
    let open_b = open as u8;
    let close_b = close as u8;

    let mut search_from = bytes.len();
    while let Some(rel) = bytes[..search_from].iter().rposition(|&b| b == close_b) {
        let end = rel;
        let mut depth: i32 = 0;
        let mut start = None;
        let mut i = end;
        loop {
            let b = bytes[i];
            if b == b'"' {
                if i == 0 {
                    break;
                }
                i -= 1;
                while i > 0 {
                    if bytes[i] == b'"' {
                        let backslashes =
                            bytes[..i].iter().rev().take_while(|&&c| c == b'\\').count();
                        if backslashes % 2 == 0 {
                            break;
                        }
                    }
                    i -= 1;
                }
            } else if b == close_b {
                depth += 1;
            } else if b == open_b {
                depth -= 1;
                if depth == 0 {
                    start = Some(i);
                    break;
                }
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }

        if let Some(s_pos) = start {
            if end > s_pos {
                return Some((s_pos, end, s[s_pos..=end].to_string()));
            }
        }
        search_from = end;
    }
    None
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

    #[test]
    fn braces_inside_strings_skipped() {
        let input = r#"{"label": "Fix {the bug}", "id": 1}"#;
        assert_eq!(extract_json(input), input);
    }

    #[test]
    fn array_after_object_picks_array() {
        let input = r#"{"ignored": true} and then [1, 2, 3]"#;
        assert_eq!(extract_json(input), "[1, 2, 3]");
    }

    #[test]
    fn close_bracket_in_trailing_prose() {
        let input = r#"{"key": "value"} see section [overview]"#;
        assert_eq!(extract_json(input), r#"{"key": "value"}"#);
    }

    #[test]
    fn close_brace_in_trailing_prose() {
        let input = r#"[1, 2, 3] refer to {docs}"#;
        assert_eq!(extract_json(input), "[1, 2, 3]");
    }

    #[test]
    fn braces_in_string_with_multiple_objects() {
        let input = r#"Example: {"a": "x{y}z"}
Real: {"steps": [{"cue_ids": [1], "label": "Fix {it}", "rationale": "ok"}]}"#;
        assert_eq!(
            extract_json(input),
            r#"{"steps": [{"cue_ids": [1], "label": "Fix {it}", "rationale": "ok"}]}"#
        );
    }
}
