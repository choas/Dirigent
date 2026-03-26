use std::path::Path;

/// Maximum bytes per auto-context section (file snippet or git diff).
/// Keeps the final prompt well under OS `ARG_MAX` limits (~1 MB on macOS).
const AUTO_CONTEXT_MAX_BYTES: usize = 100_000;

/// Parse a `[command]` prefix from cue text.
///
/// Returns `Some((command_name, remaining_text))` if the text starts with
/// `[word]`, otherwise `None`. The remaining text is trimmed.
pub(crate) fn parse_command_prefix(text: &str) -> Option<(&str, &str)> {
    let trimmed = text.trim_start();
    if !trimmed.starts_with('[') {
        return None;
    }
    let end = trimmed.find(']')?;
    let name = trimmed[1..end].trim();
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    let rest = trimmed[end + 1..].trim_start();
    Some((name, rest))
}

/// Build a structured prompt for Claude given a cue's context.
///
/// When `project_root` is provided and `file_path` is non-empty, the prompt
/// includes the surrounding file content (±50 lines) and any recent git diff
/// for the file, so Claude has immediate context without extra tool calls.
pub(crate) fn build_prompt(
    cue_text: &str,
    file_path: &str,
    line_number: usize,
    line_number_end: Option<usize>,
    images: &[String],
    _project_root: Option<&Path>,
) -> String {
    build_prompt_with_auto_context(
        cue_text,
        file_path,
        line_number,
        line_number_end,
        images,
        "",
    )
}

/// Build a structured prompt with optional auto-context (file snippet + git diff).
pub(crate) fn build_prompt_with_auto_context(
    cue_text: &str,
    file_path: &str,
    line_number: usize,
    line_number_end: Option<usize>,
    images: &[String],
    auto_context: &str,
) -> String {
    let images_section = if images.is_empty() {
        String::new()
    } else {
        let list: Vec<String> = images.iter().map(|p| format!("- {}", p)).collect();
        format!(
            "\n\n## Attached Images\n\n\
             The following images are attached. Use the Read tool to view them:\n{}",
            list.join("\n"),
        )
    };
    let auto_ctx_section = if auto_context.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", auto_context)
    };
    if file_path.is_empty() {
        format!(
            "## Task\n\n{}{}{}\n\n\
             ## Instructions\n\n\
             Make the requested changes directly by editing the files. \
             Do not output a diff — use your tools to edit files in place.",
            cue_text, images_section, auto_ctx_section,
        )
    } else {
        let line_ref = match line_number_end {
            Some(end) => format!("lines {}-{}", line_number, end),
            None => format!("line {}", line_number),
        };

        format!(
            "## Task\n\n{}{}\n\n\
             ## Context\n\n\
             Focus on {} in `{}`.\n{}\n\n\
             ## Instructions\n\n\
             Make the requested changes directly by editing the files. \
             Do not output a diff — use your tools to edit files in place.",
            cue_text, images_section, line_ref, file_path, auto_ctx_section,
        )
    }
}

/// Build the file-content snippet section for auto-context.
fn gather_file_snippet(
    project_root: &std::path::Path,
    file_path: &str,
    line_number: usize,
    line_number_end: Option<usize>,
) -> Option<String> {
    let full_path = project_root.join(file_path);
    let content = std::fs::read_to_string(&full_path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return None;
    }

    let center = line_number
        .saturating_sub(1)
        .min(lines.len().saturating_sub(1));
    let end_line = line_number_end
        .unwrap_or(line_number)
        .saturating_sub(1)
        .min(lines.len().saturating_sub(1));
    let span = end_line.saturating_sub(center) + 1;
    // Window: 50 lines total, centered on the target range
    let padding = 50usize.saturating_sub(span) / 2;
    let start = center.saturating_sub(padding).min(lines.len());
    let end = (end_line + padding + 1).min(lines.len());
    let start = start.min(end);

    let snippet: Vec<String> = lines[start..end]
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{:>4} | {}", start + i + 1, line))
        .collect();
    let snippet_text = snippet.join("\n");

    Some(format_file_snippet(file_path, start, end, &snippet_text))
}

/// Format the file snippet section, truncating if needed.
fn format_file_snippet(file_path: &str, start: usize, end: usize, snippet_text: &str) -> String {
    if snippet_text.len() <= AUTO_CONTEXT_MAX_BYTES {
        return format!(
            "## File Content\n\n\
             `{}` (lines {}-{}):\n```\n{}\n```",
            file_path,
            start + 1,
            end,
            snippet_text,
        );
    }
    // Truncate to fit within the byte ceiling
    let truncated: String = snippet_text
        .char_indices()
        .take_while(|&(i, _)| i < AUTO_CONTEXT_MAX_BYTES)
        .map(|(_, c)| c)
        .collect();
    format!(
        "## File Content\n\n\
         `{}` (lines {}-{}, truncated):\n```\n{}\n... (truncated)\n```",
        file_path,
        start + 1,
        end,
        truncated,
    )
}

/// Truncate text to fit within `AUTO_CONTEXT_MAX_BYTES`, appending a suffix if truncated.
fn truncate_to_byte_limit(text: &mut String) {
    if text.len() <= AUTO_CONTEXT_MAX_BYTES {
        return;
    }
    *text = text
        .char_indices()
        .take_while(|&(i, _)| i < AUTO_CONTEXT_MAX_BYTES)
        .map(|(_, c)| c)
        .collect();
    text.push_str("\n... (truncated)");
}

/// Build the git-diff section for auto-context.
fn gather_git_diff_section(project_root: &std::path::Path, file_path: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["diff", "--", file_path])
        .current_dir(project_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let diff = String::from_utf8_lossy(&output.stdout);
    let diff = diff.trim();
    if diff.is_empty() {
        return None;
    }

    // Limit diff to ~200 lines to avoid bloating the prompt
    let diff_lines: Vec<&str> = diff.lines().collect();
    let mut truncated = if diff_lines.len() > 200 {
        format!(
            "{}\n... ({} more lines)",
            diff_lines[..200].join("\n"),
            diff_lines.len() - 200
        )
    } else {
        diff.to_string()
    };
    // Enforce byte ceiling on top of line-count limit
    truncate_to_byte_limit(&mut truncated);

    Some(format!(
        "## Recent Changes (uncommitted)\n\n\
         ```diff\n{}\n```",
        truncated,
    ))
}

/// Generate auto-context for a file-specific cue: a snippet of the file around
/// the target line(s), and the git diff for the file (recent uncommitted changes).
///
/// Returns a formatted string to include in the prompt, or empty if no context
/// could be gathered (e.g. file doesn't exist or is a global cue).
pub(crate) fn gather_auto_context(
    project_root: &std::path::Path,
    file_path: &str,
    line_number: usize,
    line_number_end: Option<usize>,
    include_file: bool,
    include_git_diff: bool,
) -> String {
    if file_path.is_empty() {
        return String::new();
    }

    let mut sections = Vec::new();

    if include_file {
        if let Some(snippet) =
            gather_file_snippet(project_root, file_path, line_number, line_number_end)
        {
            sections.push(snippet);
        }
    }

    if include_git_diff {
        if let Some(diff_section) = gather_git_diff_section(project_root, file_path) {
            sections.push(diff_section);
        }
    }

    sections.join("\n\n")
}

/// Build a follow-up prompt for replying to a Review cue with feedback.
/// Includes the original task, the previous diff, and the user's reply.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_reply_prompt(
    original_cue: &str,
    file_path: &str,
    line_number: usize,
    line_number_end: Option<usize>,
    previous_diff: &str,
    reply: &str,
    images: &[String],
    _project_root: Option<&Path>,
) -> String {
    let context = if file_path.is_empty() {
        String::new()
    } else {
        let line_ref = match line_number_end {
            Some(end) => format!("lines {}-{}", line_number, end),
            None => format!("line {}", line_number),
        };
        format!(
            "## Context\n\n\
             Focus on {} in `{}`.\n\n",
            line_ref, file_path,
        )
    };
    let images_section = if images.is_empty() {
        String::new()
    } else {
        let list: Vec<String> = images.iter().map(|p| format!("- {}", p)).collect();
        format!(
            "\n\n## Attached Images\n\n\
             The following images are attached. Use the Read tool to view them:\n{}",
            list.join("\n"),
        )
    };
    format!(
        "## Original Task\n\n{}{}\n\n\
         {}\
         ## Previous Changes\n\n\
         You already made the following changes (currently applied in the working tree):\n\n\
         ```diff\n{}\n```\n\n\
         ## Feedback\n\n{}\n\n\
         ## Instructions\n\n\
         Adjust the code based on the feedback above. The previous changes are already applied — \
         build on them rather than starting over. \
         Make the requested changes directly by editing the files. \
         Do not output a diff — use your tools to edit files in place.",
        original_cue, images_section, context, previous_diff, reply,
    )
}

/// Extract the user-facing text from a structured prompt.
///
/// For an initial prompt, returns the text between "## Task" and the next section.
/// For a reply prompt, returns the text from "## Feedback".
/// Falls back to the full prompt if no structure is found.
pub(crate) fn extract_user_text_from_prompt(prompt: &str) -> String {
    // Reply prompt: extract feedback section
    if let Some(pos) = prompt.find("## Feedback\n\n") {
        let start = pos + "## Feedback\n\n".len();
        let rest = &prompt[start..];
        let end = rest.find("\n\n## ").unwrap_or(rest.len());
        return rest[..end].trim().to_string();
    }
    // Initial prompt: extract task section
    if let Some(pos) = prompt.find("## Task\n\n") {
        let start = pos + "## Task\n\n".len();
        let rest = &prompt[start..];
        let end = rest.find("\n\n## ").unwrap_or(rest.len());
        return rest[..end].trim().to_string();
    }
    prompt.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- build_prompt --

    #[test]
    fn build_prompt_global_cue() {
        let prompt = build_prompt("Add tests", "", 0, None, &[], None);
        assert!(prompt.contains("Add tests"));
        assert!(!prompt.contains("Focus on"));
    }

    #[test]
    fn build_prompt_with_file_single_line() {
        let prompt = build_prompt("Fix bug", "src/main.rs", 42, None, &[], None);
        assert!(prompt.contains("Fix bug"));
        assert!(prompt.contains("line 42"));
        assert!(prompt.contains("`src/main.rs`"));
    }

    #[test]
    fn build_prompt_with_file_line_range() {
        let prompt = build_prompt("Refactor", "lib.rs", 10, Some(20), &[], None);
        assert!(prompt.contains("lines 10-20"));
        assert!(prompt.contains("`lib.rs`"));
    }

    #[test]
    fn build_prompt_with_images() {
        let images = vec![
            "/tmp/screenshot.png".to_string(),
            "/tmp/design.jpg".to_string(),
        ];
        let prompt = build_prompt("Implement this design", "", 0, None, &images, None);
        assert!(prompt.contains("Attached Images"));
        assert!(prompt.contains("/tmp/screenshot.png"));
        assert!(prompt.contains("/tmp/design.jpg"));
    }

    // -- extract_user_text_from_prompt --

    #[test]
    fn extract_task_from_initial_prompt() {
        let prompt = build_prompt("Fix the bug", "src/main.rs", 42, None, &[], None);
        assert_eq!(extract_user_text_from_prompt(&prompt), "Fix the bug");
    }

    #[test]
    fn extract_task_from_global_prompt() {
        let prompt = build_prompt("Add tests", "", 0, None, &[], None);
        assert_eq!(extract_user_text_from_prompt(&prompt), "Add tests");
    }

    #[test]
    fn extract_feedback_from_reply_prompt() {
        let prompt = build_reply_prompt(
            "original task",
            "f.rs",
            1,
            None,
            "some diff",
            "please fix the typo",
            &[],
            None,
        );
        assert_eq!(
            extract_user_text_from_prompt(&prompt),
            "please fix the typo"
        );
    }

    #[test]
    fn extract_from_plain_text() {
        assert_eq!(
            extract_user_text_from_prompt("just plain text"),
            "just plain text"
        );
    }

    // -- parse_command_prefix --

    #[test]
    fn parse_command_prefix_basic() {
        let (name, rest) = parse_command_prefix("[plan] Add auth").unwrap();
        assert_eq!(name, "plan");
        assert_eq!(rest, "Add auth");
    }

    #[test]
    fn parse_command_prefix_no_bracket() {
        assert!(parse_command_prefix("just text").is_none());
    }

    #[test]
    fn parse_command_prefix_empty_name() {
        assert!(parse_command_prefix("[] some text").is_none());
    }

    #[test]
    fn parse_command_prefix_with_spaces_in_name() {
        assert!(parse_command_prefix("[two words] text").is_none());
    }

    #[test]
    fn parse_command_prefix_leading_whitespace() {
        let (name, rest) = parse_command_prefix("  [test] stuff").unwrap();
        assert_eq!(name, "test");
        assert_eq!(rest, "stuff");
    }

    #[test]
    fn parse_command_prefix_no_rest() {
        let (name, rest) = parse_command_prefix("[review]").unwrap();
        assert_eq!(name, "review");
        assert_eq!(rest, "");
    }
}
