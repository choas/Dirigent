//! The shell command that starts Claude Code in the pane, and the tab label.

/// Build the command line typed into the pane's shell to start native
/// `claude` with the run's model, permission mode and args (which carry the
/// generated `--session-id`/`--resume`). Every argument is single-quoted
/// because the pane's shell interprets the line.
///
/// Neither the prompt (sent later over the socket, once the TUI is ready) nor
/// the forwarded environment (given to `tab.create`) is part of this line, so
/// neither lands in the pane's shell history or scrollback.
pub(crate) fn build_claude_command(
    claude: &str,
    model: &str,
    args: &[String],
    skip_permissions: bool,
) -> String {
    let mut command = shell_quote(claude);
    if skip_permissions {
        command.push_str(" --dangerously-skip-permissions");
    }
    if !model.is_empty() {
        command.push_str(" --model ");
        command.push_str(&shell_quote(model));
    }
    for arg in args.iter().filter(|a| !a.is_empty()) {
        command.push(' ');
        command.push_str(&shell_quote(arg));
    }
    command
}

/// Short tab label derived from the prompt.
pub(crate) fn tab_label(prompt: &str) -> String {
    const MAX_CHARS: usize = 24;
    let summary = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated: String = summary.chars().take(MAX_CHARS).collect();
    if summary.chars().count() > MAX_CHARS {
        format!("Dirigent: {truncated}\u{2026}")
    } else {
        format!("Dirigent: {truncated}")
    }
}

/// POSIX single-quote escape: nothing inside survives shell interpretation.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn command_quotes_every_argument_and_carries_session_args() {
        let cmd = build_claude_command(
            "/opt/claude code/claude",
            "claude-sonnet-4-6",
            &strings(&["--max-turns", "10", "--session-id", "abc-123"]),
            false,
        );
        assert_eq!(
            cmd,
            "'/opt/claude code/claude' --model 'claude-sonnet-4-6' \
             '--max-turns' '10' '--session-id' 'abc-123'"
        );
    }

    #[test]
    fn command_adds_skip_permissions_and_omits_empty_model_and_args() {
        let cmd = build_claude_command("claude", "", &strings(&["", "--resume", "id"]), true);
        assert_eq!(
            cmd,
            "'claude' --dangerously-skip-permissions '--resume' 'id'"
        );
    }

    #[test]
    fn command_neutralizes_shell_metacharacters() {
        let cmd = build_claude_command("claude", "", &strings(&["$(touch x)", "it's"]), false);
        assert_eq!(cmd, r#"'claude' '$(touch x)' 'it'\''s'"#);
    }

    #[test]
    fn tab_label_collapses_whitespace_and_truncates_on_chars() {
        assert_eq!(tab_label("fix\n  the   bug"), "Dirigent: fix the bug");
        let long = "ä".repeat(30);
        assert_eq!(
            tab_label(&long),
            format!("Dirigent: {}\u{2026}", "ä".repeat(24))
        );
        // Exactly at the limit: no ellipsis, even for multi-byte text.
        assert_eq!(
            tab_label(&"ü".repeat(24)),
            format!("Dirigent: {}", "ü".repeat(24))
        );
    }
}
