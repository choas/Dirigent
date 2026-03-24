/// Heuristic prompt refinement suggestions.
///
/// Analyzes user prompt text and returns actionable suggestions to improve
/// clarity before sending to Claude. All checks are client-side (no API call).
/// A single suggestion with a short label and explanation.
#[derive(Debug, Clone)]
pub(crate) struct PromptHint {
    pub label: &'static str,
    pub detail: &'static str,
}

/// Vague words/phrases that rarely produce good results.
const VAGUE_WORDS: &[&str] = &[
    "fix it",
    "make it work",
    "improve",
    "clean up",
    "make better",
    "do something",
    "handle",
    "deal with",
    "take care of",
    "look at",
    "check this",
    "update this",
    "change this",
    "modify this",
];

/// Action verbs that indicate a clear intent.
const ACTION_VERBS: &[&str] = &[
    "add",
    "remove",
    "delete",
    "rename",
    "refactor",
    "extract",
    "implement",
    "create",
    "replace",
    "move",
    "split",
    "merge",
    "convert",
    "migrate",
    "wrap",
    "unwrap",
    "inline",
    "optimize",
    "fix",
    "resolve",
    "rewrite",
    "test",
    "document",
    "validate",
    "parse",
    "serialize",
    "deserialize",
    "format",
    "lint",
    "build",
    "debug",
    "log",
    "trace",
    "profile",
    "benchmark",
    "sort",
    "filter",
    "map",
    "reduce",
    "group",
];

/// Analyze a prompt and return suggestions (empty = prompt looks fine).
pub(crate) fn analyze(text: &str) -> Vec<PromptHint> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    // Skip analysis for [command] prefixed cues — they expand to full prompts
    if trimmed.starts_with('[') && trimmed.contains(']') {
        return Vec::new();
    }

    let mut hints = Vec::new();
    let word_count = trimmed.split_whitespace().count();

    // Don't show any hints until the user has typed at least two words
    if word_count < 2 {
        return hints;
    }

    let lower = trimmed.to_lowercase();

    // Too short — likely missing context
    if word_count < 4 {
        hints.push(PromptHint {
            label: "Very short prompt",
            detail: "Add more detail about what you want changed and why.",
        });
    }

    // Contains vague phrases
    for vague in VAGUE_WORDS {
        if lower.contains(vague) {
            hints.push(PromptHint {
                label: "Vague language",
                detail: "Be specific: what exactly should change and how?",
            });
            break;
        }
    }

    // No action verb detected (only check if prompt is short enough to matter)
    if word_count < 20 {
        let has_action = ACTION_VERBS.iter().any(|v| {
            lower
                .split_whitespace()
                .any(|w| w.trim_matches(|c: char| !c.is_alphanumeric()) == *v)
        });
        if !has_action {
            hints.push(PromptHint {
                label: "No clear action",
                detail: "Start with a verb: add, remove, refactor, implement, fix...",
            });
        }
    }

    // All-caps (shouting)
    if trimmed.len() > 10
        && trimmed == trimmed.to_uppercase()
        && trimmed.contains(char::is_alphabetic)
    {
        hints.push(PromptHint {
            label: "All caps",
            detail: "Normal casing works better for AI prompts.",
        });
    }

    hints
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_prompt_no_hints() {
        assert!(analyze("").is_empty());
    }

    #[test]
    fn single_word_no_hints() {
        // Should not trigger any hints for a single word
        assert!(analyze("fix").is_empty());
    }

    #[test]
    fn short_prompt_warns() {
        let hints = analyze("fix bug");
        assert!(hints.iter().any(|h| h.label == "Very short prompt"));
    }

    #[test]
    fn vague_prompt_warns() {
        let hints = analyze("make it work please I need this to be done");
        assert!(hints.iter().any(|h| h.label == "Vague language"));
    }

    #[test]
    fn good_prompt_no_hints() {
        let hints = analyze("Add error handling to the parse_config function for missing fields");
        assert!(hints.is_empty());
    }

    #[test]
    fn no_action_verb_warns() {
        let hints = analyze("the button color");
        assert!(hints.iter().any(|h| h.label == "No clear action"));
    }

    #[test]
    fn all_caps_warns() {
        let hints = analyze("FIX THE BROKEN LOGIN PAGE NOW");
        assert!(hints.iter().any(|h| h.label == "All caps"));
    }

    #[test]
    fn command_prefix_skipped() {
        assert!(analyze("[plan] fix the login").is_empty());
        assert!(analyze("[test] add coverage").is_empty());
    }

    #[test]
    fn action_verb_present_no_warn() {
        let hints = analyze("refactor this");
        // Has action verb, so no "No clear action" hint (but may have "Very short prompt")
        assert!(!hints.iter().any(|h| h.label == "No clear action"));
    }
}
