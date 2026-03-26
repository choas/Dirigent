/// Check if a comment is a confirmation/addressed reply rather than a new finding.
/// CodeRabbit appends "Confirmed as addressed" to the *original* comment body,
/// so we must search the entire text, not just the beginning.
pub(super) fn is_confirmation_comment(body: &str) -> bool {
    let trimmed = body.trim();
    // Strip HTML comments to get visible text
    let without_html = trimmed
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.starts_with("<!--") && !t.ends_with("-->") && !t.is_empty()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let check = without_html.trim();
    // Check anywhere in the text — CodeRabbit edits the original comment to
    // append the confirmation marker at the bottom.
    check.contains("\u{2705} Confirmed as addressed")
        || check.contains("Automated reply from [Dirigent]")
        || check.contains("<review_comment_addressed>")
        // Pure confirmation comments (standalone)
        || check.starts_with("Fixed in commit")
}

/// Check if a comment is an auto-generated summary (e.g. CodeRabbit walkthrough,
/// Qodo review header) rather than an actionable finding.
pub(super) fn is_auto_summary_comment(body: &str) -> bool {
    body.contains("<!-- walkthrough_start -->")
        || body.contains("auto-generated comment: summarize")
        || body.contains("auto-generated comment: release notes")
        // Qodo review summary header (no actionable content)
        || body.contains("Code Review by Qodo")
}

/// Extract the first "Prompt for AI Agents" block from a CodeRabbit comment.
pub(super) fn extract_agent_prompt(body: &str) -> Option<String> {
    extract_all_agent_prompts(body).into_iter().next()
}

/// Check if a marker occurrence is part of the combined "all review comments" block.
fn is_combined_prompt_block(body: &str, abs_pos: usize) -> bool {
    let mut context_start = abs_pos.saturating_sub(60);
    // Ensure we land on a valid UTF-8 char boundary (emojis are multi-byte)
    while context_start > 0 && !body.is_char_boundary(context_start) {
        context_start -= 1;
    }
    body[context_start..abs_pos].contains("all review comments")
}

/// Extract the code-fenced prompt text that follows a marker position.
fn extract_code_block_after(text: &str) -> Option<String> {
    let code_start = text.find("```")?;
    let code_content = &text[code_start + 3..];
    // Skip the language identifier line if present
    let code_content = code_content
        .find('\n')
        .map_or(code_content, |nl| &code_content[nl + 1..]);
    let code_end = code_content.find("```")?;
    let prompt = code_content[..code_end].trim().to_string();
    if prompt.is_empty() {
        None
    } else {
        Some(prompt)
    }
}

/// Extract ALL individual "Prompt for AI Agents" blocks from a body.
/// Skips the combined "Prompt for all review comments" block.
pub(super) fn extract_all_agent_prompts(body: &str) -> Vec<String> {
    let mut prompts = Vec::new();
    let marker = "Prompt for AI Agents";

    let mut search_from = 0;
    while let Some(rel_pos) = body[search_from..].find(marker) {
        let abs_pos = search_from + rel_pos;
        search_from = abs_pos + marker.len();

        if is_combined_prompt_block(body, abs_pos) {
            continue;
        }

        let after_marker = &body[abs_pos + marker.len()..];
        if let Some(prompt) = extract_code_block_after(after_marker) {
            prompts.push(prompt);
        }
    }

    prompts
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- is_confirmation_comment --

    #[test]
    fn confirmation_comment_with_checkmark() {
        let body = "Some finding text\n\n\u{2705} Confirmed as addressed by @user";
        assert!(is_confirmation_comment(body));
    }

    #[test]
    fn confirmation_comment_in_html_stripped() {
        // Confirmation marker as visible text (not in HTML comment) should be detected
        let body = "Finding text\n<!-- comment -->\n\u{2705} Confirmed as addressed\n<!-- end -->";
        assert!(is_confirmation_comment(body));
    }

    #[test]
    fn non_confirmation_comment() {
        let body = "**Bug found:** This function panics on empty input.";
        assert!(!is_confirmation_comment(body));
    }

    // -- is_auto_summary_comment --

    #[test]
    fn auto_summary_walkthrough() {
        let body = "<!-- walkthrough_start -->\n## Walkthrough\nSome changes...";
        assert!(is_auto_summary_comment(body));
    }

    #[test]
    fn auto_summary_not_review() {
        // Review status comment is NOT an auto-summary (it contains actual findings)
        let body = "<!-- This is an auto-generated comment by CodeRabbit for review status -->";
        assert!(!is_auto_summary_comment(body));
    }

    #[test]
    fn qodo_summary_header_is_skipped() {
        let body = r#"<h3>Code Review by Qodo</h3>
<code>🐞 Bugs (2)</code>  <code>📘 Rule violations (0)</code>
<img src="https://www.qodo.ai/logo.svg" height="10%" alt="Grey Divider">
<a href="https://www.qodo.ai"><img src="https://www.qodo.ai/logo.svg" width="80" alt="Qodo Logo"></a>"#;
        assert!(is_auto_summary_comment(body));
    }

    // -- extract_all_agent_prompts --

    #[test]
    fn extract_single_agent_prompt() {
        let body = r#"Some finding text

<details>
<summary>🤖 Prompt for AI Agents</summary>

```
Fix the bug in src/main.rs at line 42.
```

</details>"#;
        let prompts = extract_all_agent_prompts(body);
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains("Fix the bug"));
    }

    #[test]
    fn extract_multiple_agent_prompts_skips_combined() {
        let body = r#"<details>
<summary>🤖 Prompt for AI Agents</summary>

```
First finding.
```

</details>

<details>
<summary>🤖 Prompt for AI Agents</summary>

```
Second finding.
```

</details>

<details>
<summary>🤖 Prompt for all review comments with AI agents</summary>

```
Combined prompt (should be skipped).
```

</details>"#;
        let prompts = extract_all_agent_prompts(body);
        assert_eq!(prompts.len(), 2);
        assert!(prompts[0].contains("First finding"));
        assert!(prompts[1].contains("Second finding"));
    }

    #[test]
    fn extract_agent_prompt_with_emoji_context() {
        // Emojis near the marker shouldn't cause panics
        let body = "🧹🔧🐛 Some context\n\n<summary>🤖 Prompt for AI Agents</summary>\n\n```\nFix it.\n```";
        let prompts = extract_all_agent_prompts(body);
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains("Fix it"));
    }
}
