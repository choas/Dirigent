/// Extract a JSON object from text that may contain markdown code fences or surrounding prose.
///
/// Tries, in order: ````json` fence, bare ``` fence whose content starts with `{`,
/// then the outermost `{`…`}` span.  Returns the trimmed input when nothing matches.
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
            if inner.starts_with('{') {
                return inner.to_string();
            }
        }
    }
    if let Some(start) = s.find('{') {
        if let Some(end) = s.rfind('}') {
            if end > start {
                return s[start..=end].to_string();
            }
        }
    }
    s.trim().to_string()
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
}
