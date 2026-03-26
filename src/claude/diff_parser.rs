/// Parse diff content from a Claude response.
pub(crate) fn parse_diff_from_response(response: &str) -> Option<String> {
    if let Some(diff) = extract_fenced_diff(response) {
        return Some(diff);
    }
    extract_unified_diff(response)
}

/// Extract fenced diff code blocks from a response string.
fn collect_fenced_blocks(response: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current_lines: Vec<&str> = Vec::new();
    let mut in_block = false;

    for line in response.lines() {
        let trimmed = line.trim_start();
        if !in_block && trimmed.starts_with("```diff") {
            in_block = true;
            current_lines.clear();
            continue;
        }
        if in_block && trimmed.starts_with("```") {
            if !current_lines.is_empty() {
                blocks.push(current_lines.join("\n"));
            }
            in_block = false;
            current_lines.clear();
            continue;
        }
        if in_block {
            current_lines.push(line);
        }
    }

    blocks
}

/// Ensure text ends with a newline.
fn ensure_trailing_newline(text: &mut String) {
    if !text.ends_with('\n') {
        text.push('\n');
    }
}

fn extract_fenced_diff(response: &str) -> Option<String> {
    let blocks = collect_fenced_blocks(response);
    if blocks.is_empty() {
        return None;
    }

    let diffs: Vec<String> = blocks
        .iter()
        .filter_map(|block| extract_unified_diff(block))
        .collect();

    if diffs.is_empty() {
        return None;
    }

    let mut result = diffs.join("\n");
    ensure_trailing_newline(&mut result);
    Some(result)
}

/// Check whether a line belongs to a unified diff hunk body.
fn is_diff_body_line(line: &str) -> bool {
    line.starts_with("@@ ")
        || line.starts_with('+')
        || line.starts_with('-')
        || line.starts_with(' ')
        || line.starts_with("\\ No newline at end of file")
}

/// Check whether the line at `i` starts a new file header (`--- ` / `+++ ` pair).
fn is_file_header_start(lines: &[&str], i: usize) -> bool {
    lines[i].starts_with("--- ") && i + 1 < lines.len() && lines[i + 1].starts_with("+++ ")
}

/// Collect contiguous diff hunk lines starting at `i`, returning the new index.
fn collect_hunk_lines<'a>(lines: &[&'a str], mut i: usize, result: &mut Vec<&'a str>) -> usize {
    while i < lines.len() {
        if is_diff_body_line(lines[i]) {
            result.push(lines[i]);
            i += 1;
            continue;
        }
        // Stop at next file header or non-diff line
        break;
    }
    i
}

fn extract_unified_diff(response: &str) -> Option<String> {
    let lines: Vec<&str> = response.lines().collect();
    let mut result: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if !is_file_header_start(&lines, i) {
            i += 1;
            continue;
        }
        // Push the --- and +++ header lines
        result.push(lines[i]);
        result.push(lines[i + 1]);
        i += 2;
        i = collect_hunk_lines(&lines, i, &mut result);
    }

    if result.is_empty() {
        return None;
    }

    let mut text = result.join("\n");
    ensure_trailing_newline(&mut text);
    Some(fix_hunk_headers(&text))
}

/// Count old and new lines in a hunk body starting at index `start`.
fn count_hunk_lines(lines: &[&str], start: usize) -> (usize, usize) {
    let mut old_count = 0usize;
    let mut new_count = 0usize;
    let mut j = start;
    while j < lines.len() {
        let line = lines[j];
        let is_next_hunk = line.starts_with("@@ ") || is_file_header_start(lines, j);
        if is_next_hunk {
            break;
        }
        if line.starts_with('+') {
            new_count += 1;
        } else if line.starts_with('-') {
            old_count += 1;
        } else if line.starts_with(' ') {
            old_count += 1;
            new_count += 1;
        } else if line.starts_with("\\ No newline at end of file") {
            // marker line; does not affect old/new counts
        } else {
            break;
        }
        j += 1;
    }
    (old_count, new_count)
}

fn fix_hunk_headers(diff_text: &str) -> String {
    let lines: Vec<&str> = diff_text.lines().collect();
    let mut result = Vec::new();

    for (i, &line) in lines.iter().enumerate() {
        if !line.starts_with("@@ ") {
            result.push(line.to_string());
            continue;
        }
        let (old_start, new_start, tail) = parse_hunk_header(line);
        let (old_count, new_count) = count_hunk_lines(&lines, i + 1);
        result.push(format!(
            "@@ -{},{} +{},{} @@{}",
            old_start, old_count, new_start, new_count, tail
        ));
    }

    let mut text = result.join("\n");
    ensure_trailing_newline(&mut text);
    text
}

fn parse_hunk_header(header: &str) -> (usize, usize, &str) {
    let inner = header.strip_prefix("@@ ").unwrap_or(header);

    let (range_part, tail) = if let Some(pos) = inner.find(" @@") {
        let after = &inner[pos + 3..];
        (&inner[..pos], after)
    } else {
        (inner, "")
    };

    let parts: Vec<&str> = range_part.split_whitespace().collect();
    let old_start = parts
        .first()
        .and_then(|p| p.strip_prefix('-'))
        .and_then(|p| p.split(',').next())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1);
    let new_start = parts
        .get(1)
        .and_then(|p| p.strip_prefix('+'))
        .and_then(|p| p.split(',').next())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1);

    (old_start, new_start, tail)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_diff_from_response --

    #[test]
    fn parse_fenced_diff() {
        let response = "\
Here's the fix:

```diff
--- a/foo.rs
+++ b/foo.rs
@@ -1,3 +1,3 @@
 line1
-old
+new
```

Done!";
        let diff = parse_diff_from_response(response).unwrap();
        assert!(diff.contains("--- a/foo.rs"));
        assert!(diff.contains("+++ b/foo.rs"));
        assert!(diff.contains("-old"));
        assert!(diff.contains("+new"));
    }

    #[test]
    fn parse_inline_unified_diff() {
        let response = "\
--- a/foo.rs
+++ b/foo.rs
@@ -1,2 +1,2 @@
 keep
-remove
+add
";
        let diff = parse_diff_from_response(response).unwrap();
        assert!(diff.contains("-remove"));
        assert!(diff.contains("+add"));
    }

    #[test]
    fn parse_no_diff_returns_none() {
        assert!(parse_diff_from_response("Just some text, no diff here.").is_none());
    }

    #[test]
    fn parse_multiple_fenced_diffs() {
        let response = "\
```diff
--- a/one.rs
+++ b/one.rs
@@ -1,1 +1,1 @@
-a
+b
```

```diff
--- a/two.rs
+++ b/two.rs
@@ -1,1 +1,1 @@
-c
+d
```";
        let diff = parse_diff_from_response(response).unwrap();
        assert!(diff.contains("--- a/one.rs"));
        assert!(diff.contains("--- a/two.rs"));
    }

    // -- fix_hunk_headers --

    #[test]
    fn fix_hunk_headers_corrects_counts() {
        let input = "\
--- a/f.rs
+++ b/f.rs
@@ -1,999 +1,999 @@
 context
-old1
-old2
+new1
";
        let result = fix_hunk_headers(input);
        // 1 context + 2 old = 3 old lines, 1 context + 1 new = 2 new lines
        assert!(result.contains("@@ -1,3 +1,2 @@"));
    }

    #[test]
    fn fix_hunk_headers_preserves_tail() {
        let input = "\
--- a/f.rs
+++ b/f.rs
@@ -10,0 +10,0 @@ fn main()
+new_line
";
        let result = fix_hunk_headers(input);
        assert!(result.contains(" fn main()"));
    }

    // -- parse_hunk_header --

    #[test]
    fn parse_hunk_header_basic() {
        let (old, new, tail) = parse_hunk_header("@@ -10,5 +20,3 @@");
        assert_eq!(old, 10);
        assert_eq!(new, 20);
        assert_eq!(tail, "");
    }

    #[test]
    fn parse_hunk_header_with_function_context() {
        let (old, new, tail) = parse_hunk_header("@@ -1,4 +1,4 @@ fn main()");
        assert_eq!(old, 1);
        assert_eq!(new, 1);
        assert_eq!(tail, " fn main()");
    }

    #[test]
    fn parse_hunk_header_no_comma() {
        let (old, new, _) = parse_hunk_header("@@ -1 +1 @@");
        assert_eq!(old, 1);
        assert_eq!(new, 1);
    }
}
