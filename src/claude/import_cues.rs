const IMPORT_START: &str = "<!-- DIRIGENT_IMPORT_START -->";
const IMPORT_END: &str = "<!-- DIRIGENT_IMPORT_END -->";

const USER_TEXT_BEGIN: &str = "<!-- BEGIN_USER_TEXT -->";
const USER_TEXT_END: &str = "<!-- END_USER_TEXT -->";

/// A cue imported from Claude's structured output.
#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct ImportedCue {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub file_path: String,
    #[serde(default)]
    pub line_number: usize,
}

/// Check if the cue text looks like an import request (contains a GitHub PR URL).
pub(crate) fn is_import_request(text: &str) -> bool {
    text.contains("github.com/") && text.contains("/pull/")
}

/// Build a prompt for import requests. Instructs Claude to read the PR comments
/// and output structured JSON instead of editing files.
pub(crate) fn build_import_prompt(cue_text: &str, images: &[String]) -> String {
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
        "## Task\n\n\
         {USER_TEXT_BEGIN}{cue_text}{USER_TEXT_END}{images_section}\n\n\
         ## Instructions\n\n\
         Read the requested comments/reviews and output each actionable item as a cue.\n\
         Do NOT edit any files. Instead, output the items between the markers below\n\
         in JSON format:\n\n\
         <!-- DIRIGENT_IMPORT_START -->\n\
         [{{\"id\": \"unique-id\", \"text\": \"description\", \"file_path\": \"path/to/file\", \"line_number\": 42}}, ...]\n\
         <!-- DIRIGENT_IMPORT_END -->\n\n\
         Rules:\n\
         - `id` must be a unique identifier (e.g. the comment ID from the API)\n\
         - `text` should clearly describe the issue or action needed\n\
         - `file_path` and `line_number` are optional (use empty string and 0 if not file-specific)\n\
         - Skip auto-generated summaries, bot confirmations, and non-actionable comments\n\
         - Use `gh` CLI to access GitHub PRs (e.g. `gh pr view <number> --comments` or `gh api`)\n\
         - For PRs in other repos, use the full URL with `gh`: `gh pr view <url> --comments`",
    )
}

/// Parse import data from text (response or running log).
/// Looks for sentinel markers wrapping a JSON array of cues.
pub(crate) fn parse_import_cues(text: &str) -> Option<Vec<ImportedCue>> {
    let start = text.find(IMPORT_START)?;
    let after_start = start + IMPORT_START.len();
    let rest = &text[after_start..];
    let end = rest.find(IMPORT_END)?;
    let json_text = rest[..end].trim();
    serde_json::from_str(json_text).ok()
}

/// Extract a human-readable label from a GitHub PR URL in the cue text.
/// Returns e.g. "PR #2 choas/test-verifier".
pub(crate) fn extract_pr_label(cue_text: &str) -> String {
    if let Some(idx) = cue_text.find("github.com/") {
        let rest = &cue_text[idx + "github.com/".len()..];
        let parts: Vec<&str> = rest.splitn(5, '/').collect();
        if parts.len() >= 4 && parts[2] == "pull" {
            let number = parts[3]
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap_or(parts[3]);
            if !number.is_empty() {
                return format!("PR #{} {}/{}", number, parts[0], parts[1]);
            }
        }
    }
    "PR Import".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_import_request_with_pr_url() {
        assert!(is_import_request(
            "import comments from https://github.com/choas/test-verifier/pull/2"
        ));
    }

    #[test]
    fn is_import_request_without_pr_url() {
        assert!(!is_import_request("fix the bug in main.rs"));
        assert!(!is_import_request(
            "check https://github.com/choas/test-verifier/issues/5"
        ));
    }

    #[test]
    fn parse_import_cues_valid() {
        let text = format!(
            "Here are the findings:\n\n\
             {}\n\
             [{{\"id\":\"123\",\"text\":\"Fix the bug\",\"file_path\":\"src/main.rs\",\"line_number\":42}},\
              {{\"id\":\"456\",\"text\":\"Add tests\"}}]\n\
             {}\n\n\
             Done.",
            IMPORT_START, IMPORT_END,
        );
        let cues = parse_import_cues(&text).expect("should parse");
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].id, "123");
        assert_eq!(cues[0].text, "Fix the bug");
        assert_eq!(cues[0].file_path, "src/main.rs");
        assert_eq!(cues[0].line_number, 42);
        assert_eq!(cues[1].id, "456");
        assert_eq!(cues[1].text, "Add tests");
        assert_eq!(cues[1].file_path, "");
        assert_eq!(cues[1].line_number, 0);
    }

    #[test]
    fn parse_import_cues_no_markers() {
        assert!(parse_import_cues("just some text without markers").is_none());
    }

    #[test]
    fn parse_import_cues_invalid_json() {
        let text = format!("{}\nnot valid json\n{}", IMPORT_START, IMPORT_END,);
        assert!(parse_import_cues(&text).is_none());
    }

    #[test]
    fn extract_pr_label_full_url() {
        let text = "import from https://github.com/choas/test-verifier/pull/2 please";
        assert_eq!(extract_pr_label(text), "PR #2 choas/test-verifier");
    }

    #[test]
    fn extract_pr_label_no_url() {
        assert_eq!(extract_pr_label("fix the bug"), "PR Import");
    }

    #[test]
    fn build_import_prompt_contains_markers() {
        let prompt = build_import_prompt("import PR comments", &[]);
        assert!(prompt.contains("DIRIGENT_IMPORT_START"));
        assert!(prompt.contains("DIRIGENT_IMPORT_END"));
        assert!(prompt.contains("Do NOT edit any files"));
        assert!(!prompt.contains("Make the requested changes"));
    }
}
