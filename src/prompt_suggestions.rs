/// Heuristic prompt refinement suggestions.
///
/// Checks the user's cue text for common issues (too short, vague wording,
/// missing action verb, no file context) and returns non-blocking suggestions
/// to help the user write more effective prompts.

/// A single suggestion with a short label and description.
#[derive(Debug, Clone)]
pub(crate) struct PromptSuggestion {
    pub label: &'static str,
    pub detail: &'static str,
}

/// Analyse `text` and return zero or more improvement suggestions.
pub(crate) fn analyse_prompt(text: &str, has_file_context: bool) -> Vec<PromptSuggestion> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    // Skip analysis for [command] prefixed cues — they expand to full prompts
    if trimmed.starts_with('[') && trimmed.contains(']') {
        return Vec::new();
    }

    let mut suggestions = Vec::new();

    let words: Vec<&str> = trimmed.split_whitespace().collect();
    let word_count = words.len();

    // 1. Too short — fewer than 4 words is almost certainly too vague
    if word_count < 4 {
        suggestions.push(PromptSuggestion {
            label: "Too short",
            detail: "Prompts with fewer than 4 words often produce vague results. \
                     Try describing what you want changed and why.",
        });
    }

    // 2. Vague wording — common filler words that don't give Claude direction
    let vague_words = [
        "fix", "improve", "update", "change", "modify", "handle", "do", "make",
    ];
    let first_word = words.first().map(|w| w.to_lowercase()).unwrap_or_default();
    // Only flag if the vague word is the ONLY verb (short prompt)
    if word_count <= 5 && vague_words.contains(&first_word.as_str()) {
        suggestions.push(PromptSuggestion {
            label: "Vague action",
            detail: "Words like \"fix\" or \"improve\" are ambiguous alone. \
                     Try specifying what to fix and the expected behavior, \
                     e.g. \"Fix the null pointer in parse_config by adding a bounds check\".",
        });
    }

    // 3. No action verb at all — prompt reads like a description, not an instruction
    // Only directive verbs that tell the AI what to do — excludes programming
    // keywords (return, throw, map, filter, …) that appear in descriptions.
    let action_verbs = [
        "add",
        "remove",
        "delete",
        "create",
        "implement",
        "refactor",
        "rename",
        "move",
        "extract",
        "replace",
        "rewrite",
        "convert",
        "migrate",
        "split",
        "merge",
        "wrap",
        "inline",
        "optimize",
        "simplify",
        "fix",
        "repair",
        "resolve",
        "debug",
        "test",
        "document",
        "format",
        "lint",
        "configure",
        "enable",
        "disable",
        "generate",
        "scaffold",
        "upgrade",
        "downgrade",
        "analyze",
        "change",
        "update",
        "modify",
        "improve",
        "make",
        "write",
    ];
    let has_action = words.iter().any(|w| {
        let lower = w.to_lowercase();
        let clean = lower.trim_end_matches(|c: char| !c.is_alphabetic());
        action_verbs.contains(&clean)
    });
    if !has_action && word_count >= 4 {
        suggestions.push(PromptSuggestion {
            label: "No action verb",
            detail: "Your prompt doesn't contain a clear action verb. \
                     Start with what you want done: add, remove, refactor, implement, etc.",
        });
    }

    // 4. No file context and prompt seems file-specific
    if !has_file_context && word_count >= 4 {
        let mentions_file = words.iter().any(|w| {
            w.contains('.') && w.len() > 3 && !w.starts_with("e.g") && !w.starts_with("i.e")
        });
        let mentions_location = trimmed.contains("line ")
            || trimmed.contains("function ")
            || trimmed.contains("method ")
            || trimmed.contains("class ")
            || trimmed.contains("struct ");
        if mentions_file || mentions_location {
            suggestions.push(PromptSuggestion {
                label: "Consider file context",
                detail: "Your prompt mentions specific code but is a global cue. \
                         Creating the cue on the file (click a line number) gives Claude \
                         direct context and produces better results.",
            });
        }
    }

    suggestions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_prompt_no_suggestions() {
        assert!(analyse_prompt("", false).is_empty());
        assert!(analyse_prompt("   ", false).is_empty());
    }

    #[test]
    fn short_prompt_flagged() {
        let suggestions = analyse_prompt("fix bug", false);
        assert!(suggestions.iter().any(|s| s.label == "Too short"));
    }

    #[test]
    fn vague_action_flagged() {
        let suggestions = analyse_prompt("fix the thing", false);
        assert!(suggestions.iter().any(|s| s.label == "Vague action"));
    }

    #[test]
    fn good_prompt_no_suggestions() {
        let suggestions = analyse_prompt(
            "Refactor the parse_config function to return Result instead of panicking on invalid input",
            true,
        );
        assert!(suggestions.is_empty(), "Got: {:?}", suggestions);
    }

    #[test]
    fn no_action_verb_flagged() {
        let suggestions = analyse_prompt("the database connection pool settings", false);
        assert!(suggestions.iter().any(|s| s.label == "No action verb"));
    }

    #[test]
    fn command_prefix_skipped() {
        assert!(analyse_prompt("[plan] Add auth system", false).is_empty());
    }

    #[test]
    fn file_mention_in_global_cue_flagged() {
        let suggestions = analyse_prompt("Add error handling to main.rs line 42", false);
        assert!(suggestions
            .iter()
            .any(|s| s.label == "Consider file context"));
    }

    #[test]
    fn file_mention_with_context_not_flagged() {
        let suggestions = analyse_prompt("Add error handling to main.rs line 42", true);
        assert!(!suggestions
            .iter()
            .any(|s| s.label == "Consider file context"));
    }
}
