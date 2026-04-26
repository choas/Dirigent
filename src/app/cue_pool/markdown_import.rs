use std::path::PathBuf;

pub(super) struct ImportedSection {
    pub(super) number: usize,
    pub(super) title: String,
    pub(super) body: String,
}

pub(super) fn parse_markdown_sections(content: &str) -> Vec<ImportedSection> {
    let mut sections = Vec::new();
    let mut current_title: Option<(usize, String)> = None;
    let mut body_lines: Vec<&str> = Vec::new();

    for line in content.lines() {
        if let Some(heading) = line.strip_prefix("### ") {
            // Flush previous section
            if let Some((num, title)) = current_title.take() {
                sections.push(ImportedSection {
                    number: num,
                    title,
                    body: clean_body(&body_lines),
                });
                body_lines.clear();
            }
            // Parse "N. Title" pattern
            let heading = heading.trim();
            if let Some(dot_pos) = heading.find(". ") {
                if let Ok(num) = heading[..dot_pos].parse::<usize>() {
                    current_title = Some((num, heading[dot_pos + 2..].to_string()));
                    continue;
                }
            }
            // Fallback: no number
            current_title = Some((sections.len() + 1, heading.to_string()));
        } else if current_title.is_some() {
            body_lines.push(line);
        }
    }
    // Flush last section
    if let Some((num, title)) = current_title {
        sections.push(ImportedSection {
            number: num,
            title,
            body: clean_body(&body_lines),
        });
    }

    sections
}

/// Clean up section body: strip `---` separators, code fences, and collapse
/// excessive blank lines while preserving the full content.
/// Collapse runs of consecutive blank lines down to a single blank line.
fn collapse_blank_lines(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut consecutive_blanks = 0u32;

    for line in text.lines() {
        if line.trim().is_empty() {
            consecutive_blanks += 1;
            if consecutive_blanks <= 1 {
                result.push('\n');
            }
        } else {
            consecutive_blanks = 0;
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(line);
        }
    }

    result
}

fn clean_body(lines: &[&str]) -> String {
    let mut out = Vec::new();
    let mut in_code_block = false;

    for &line in lines {
        let trimmed = line.trim();

        // Toggle code blocks — skip fence lines but keep code content
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        // Skip horizontal rules
        if !in_code_block && trimmed == "---" {
            continue;
        }

        out.push(line);
    }

    // Trim leading/trailing blank lines and collapse consecutive blanks
    let text = out.join("\n");
    let text = text.trim();

    collapse_blank_lines(text)
}

pub(super) fn pick_markdown_file(start_dir: &std::path::Path) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Import Markdown Document")
        .set_directory(start_dir)
        .add_filter("Text files", &["md", "txt", "markdown"])
        .pick_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_markdown_sections_basic() {
        let content =
            "### 1. First Section\nBody of first.\n### 2. Second Section\nBody of second.";
        let sections = parse_markdown_sections(content);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].number, 1);
        assert_eq!(sections[0].title, "First Section");
        assert!(sections[0].body.contains("Body of first"));
        assert_eq!(sections[1].number, 2);
        assert_eq!(sections[1].title, "Second Section");
    }

    #[test]
    fn parse_markdown_sections_no_numbered_headings() {
        let content = "### Introduction\nSome text.\n### Details\nMore text.";
        let sections = parse_markdown_sections(content);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].title, "Introduction");
        assert_eq!(sections[0].number, 1);
        assert_eq!(sections[1].title, "Details");
        assert_eq!(sections[1].number, 2);
    }

    #[test]
    fn parse_markdown_sections_empty_input() {
        assert!(parse_markdown_sections("").is_empty());
    }

    #[test]
    fn parse_markdown_sections_no_headings() {
        let content = "Just some text\nwithout any headings.";
        assert!(parse_markdown_sections(content).is_empty());
    }

    #[test]
    fn parse_markdown_sections_ignores_non_h3() {
        let content = "# Top level\n## Second level\n### 1. Actual Section\nBody.";
        let sections = parse_markdown_sections(content);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].title, "Actual Section");
    }

    #[test]
    fn parse_markdown_sections_non_ascii_content() {
        let content = "### 1. Ünïcödé Séction\nCafé résumé.\n日本語テキスト。";
        let sections = parse_markdown_sections(content);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].title, "Ünïcödé Séction");
        assert!(sections[0].body.contains("Café"));
        assert!(sections[0].body.contains("日本語"));
    }

    #[test]
    fn parse_markdown_sections_strips_separators_and_fences() {
        let content = "### 1. Section\n---\n```rust\nfn main() {}\n```\nAfter fence.";
        let sections = parse_markdown_sections(content);
        assert_eq!(sections.len(), 1);
        assert!(!sections[0].body.contains("---"));
        assert!(!sections[0].body.contains("```"));
        assert!(sections[0].body.contains("fn main() {}"));
        assert!(sections[0].body.contains("After fence."));
    }
}
