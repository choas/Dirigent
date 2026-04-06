use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Play {
    pub name: String,
    pub prompt: String,
}

impl Play {
    fn new(name: &str, prompt: &str) -> Self {
        Self {
            name: name.into(),
            prompt: prompt.into(),
        }
    }
}

/// A parsed template variable from a play prompt, e.g. `{LICENSE:MIT,Apache 2.0,ISC}`.
#[derive(Debug, Clone)]
pub(crate) struct PlayVariable {
    /// Variable name (e.g. "LICENSE").
    pub name: String,
    /// Predefined options (may be empty for free-text variables).
    pub options: Vec<String>,
    /// The full matched token including braces, for substitution.
    pub token: String,
}

/// Parse template variables from a play prompt.
/// Syntax: `{VAR_NAME:option1,option2,...}` or `{VAR_NAME}` for free-text.
pub(crate) fn parse_play_variables(prompt: &str) -> Vec<PlayVariable> {
    let mut vars = Vec::new();
    let mut seen_tokens = std::collections::HashSet::new();
    let mut rest = prompt;
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start..].find('}') else {
            break;
        };
        let token = &rest[start..start + end + 1];
        let inner = &rest[start + 1..start + end];
        rest = &rest[start + end + 1..];

        let is_variable = (!inner.is_empty() && !inner.contains(' ')) || inner.contains(':');
        if !is_variable {
            continue;
        }

        let (name, options) = if let Some(colon) = inner.find(':') {
            let name = inner[..colon].to_string();
            let opts: Vec<String> = inner[colon + 1..]
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            (name, opts)
        } else {
            (inner.to_string(), Vec::new())
        };

        if !name.is_empty() && seen_tokens.insert(token.to_string()) {
            vars.push(PlayVariable {
                name,
                options,
                token: token.to_string(),
            });
        }
    }
    vars
}

/// Substitute resolved variables back into the prompt template.
pub(crate) fn substitute_play_variables(
    prompt: &str,
    resolved: &[(String, String)], // (token, value)
) -> String {
    let mut result = prompt.to_string();
    for (token, value) in resolved {
        result = result.replace(token, value);
    }
    result
}

pub(crate) fn default_playbook() -> Vec<Play> {
    vec![
        Play::new("Documentation (Diataxis)", "Update or generate documentation following the Diataxis framework: tutorials (learning-oriented), how-to guides (task-oriented), reference (information-oriented), and explanation (understanding-oriented). Update README.md to accurately reflect the current state: features, setup instructions, and usage."),
        Play::new("Verify architecture", "Analyze the project architecture. Check for structural issues, circular dependencies, inconsistent patterns. Report findings without making changes."),
        Play::new("Verify last 5 commits", "Review the last 5 git commits. Check for bugs, incomplete changes, or inconsistencies. Report findings without making changes."),
        Play::new("Create release", "Prepare a release for version {VERSION}: update version numbers to {VERSION}, ensure CHANGELOG is current, verify tests pass, ensure LICENSE file ({LICENSE:MIT,Apache 2.0,BSD 2-Clause,BSD 3-Clause,ISC,MPL 2.0,Unlicense}) is present, create a release commit, create a git tag v{VERSION}, and run `git push && git push --tags`."),
        Play::new("Security audit", "Check for hardcoded secrets, insecure dependencies, injection vulnerabilities, unsafe code patterns. Report findings."),
        Play::new("Check dead code", "Find unused functions, unreachable branches, unused imports, stale modules. Report findings without removing anything."),
        Play::new("Add tests", "Identify untested code paths and write comprehensive tests for the most critical and least covered areas."),
        Play::new("Fix all warnings", "Detect the project type (e.g. Cargo.toml for Rust, package.json for JS/TS, go.mod for Go, etc.), run the appropriate check/lint command, collect all warnings, and fix every one of them."),
        Play::new("Commit changes", "Commit all current changes. Open the SQLite database (find the .db file in the repo) and query the cues table for rows with status 'done' or 'review'. Use their titles to write a meaningful commit message summarizing what was accomplished. Then UPDATE any cues with status='review' to status='done'. Finally, stage all changes with git and create the commit."),
        Play::new("Zero day test", "Somebody told me there is an RCE 0-day when this project opens a file. Find it."),
        Play::new("Pin dependency versions", "Detect the project type and its dependency file (e.g. Cargo.toml, package.json, go.mod, requirements.txt, pyproject.toml, pom.xml, build.gradle, Gemfile, etc.). For every dependency that uses a version range, caret (^), tilde (~), wildcard (*), or any specifier that allows automatic minor or patch updates, resolve it to the exact latest version available and pin it (e.g. change ^1.2.3 or ~1.2 to the exact latest version like 1.4.0). This protects against supply-chain attacks where a hijacked package publishes a malicious minor or patch release that gets pulled in automatically. Do not change dependencies that are already pinned to an exact version. After updating, verify the project still builds."),
    ]
}
