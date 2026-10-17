//! "Test Connection" in Settings → General: sends a one-word prompt through the
//! selected CLI provider, using the unsaved settings on screen, and checks that
//! a non-empty reply comes back.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::app::types::ConnectionTest;
use crate::app::DirigentApp;
use crate::settings::{CliProvider, Settings};

const TEST_PROMPT: &str = "hello";
/// Agents launched through `npx` may download a package on first use.
const TEST_TIMEOUT: Duration = Duration::from_secs(120);
const REPLY_PREVIEW_CHARS: usize = 60;

impl DirigentApp {
    pub(super) fn render_connection_test_row(&mut self, ui: &mut egui::Ui) {
        self.poll_connection_test();
        ui.label("Connection:");
        ui.horizontal(|ui| {
            let provider = self.settings.cli_provider.clone();
            match self
                .connection_test
                .as_ref()
                .filter(|test| test.provider == provider)
            {
                Some(test) if test.outcome.is_none() => {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new(format!(
                            "waiting for a reply\u{2026} {}s",
                            test.started.elapsed().as_secs()
                        ))
                        .weak(),
                    );
                    if ui.small_button("Cancel").clicked() {
                        self.connection_test = None;
                    }
                    ui.ctx().request_repaint_after(Duration::from_millis(500));
                }
                outcome => {
                    let label = outcome.and_then(|test| test.outcome.clone());
                    if ui
                        .button("Test Connection")
                        .on_hover_text(format!(
                            "Send \"{TEST_PROMPT}\" to {} with the settings above \
                             (unsaved changes included, pre/post-run scripts skipped) \
                             and check that it replies.",
                            provider.display_name()
                        ))
                        .clicked()
                    {
                        self.start_connection_test();
                    }
                    if let Some((result, elapsed)) = label {
                        self.render_connection_test_outcome(ui, &result, elapsed);
                    }
                }
            }
        });
        ui.end_row();
    }

    fn render_connection_test_outcome(
        &self,
        ui: &mut egui::Ui,
        result: &Result<String, String>,
        elapsed: Duration,
    ) {
        let secs = elapsed.as_secs_f32();
        match result {
            Ok(reply) => {
                ui.label(
                    egui::RichText::new(format!("\u{2713} {} ({secs:.1}s)", reply_preview(reply)))
                        .color(self.semantic.success),
                )
                .on_hover_text(reply);
            }
            Err(err) => {
                ui.label(
                    egui::RichText::new(format!("\u{2717} {}", reply_preview(err)))
                        .color(self.semantic.danger),
                )
                .on_hover_text(err);
            }
        }
    }

    fn start_connection_test(&mut self) {
        let provider = self.settings.cli_provider.clone();
        let settings = without_run_scripts(&self.settings, &provider);
        let project_root = self.project_root.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let ctx = self.egui_ctx.clone();
        {
            let provider = provider.clone();
            let cancel = Arc::clone(&cancel);
            std::thread::spawn(move || {
                let result = run_connection_test(&settings, &provider, &project_root, cancel);
                let _ = tx.send(result);
                if let Some(c) = ctx.get() {
                    c.request_repaint();
                }
            });
        }
        self.connection_test = Some(ConnectionTest {
            provider,
            started: Instant::now(),
            rx,
            cancel,
            outcome: None,
        });
    }

    fn poll_connection_test(&mut self) {
        let Some(test) = self.connection_test.as_mut() else {
            return;
        };
        if test.outcome.is_some() {
            return;
        }
        let elapsed = test.started.elapsed();
        match test.rx.try_recv() {
            Ok(result) => test.outcome = Some((result, elapsed)),
            Err(mpsc::TryRecvError::Disconnected) => {
                test.outcome = Some((Err("test worker stopped unexpectedly".into()), elapsed));
            }
            Err(mpsc::TryRecvError::Empty) if elapsed > TEST_TIMEOUT => {
                test.cancel.store(true, Ordering::Relaxed);
                test.outcome = Some((
                    Err(format!("no reply after {}s", TEST_TIMEOUT.as_secs())),
                    elapsed,
                ));
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
}

/// Send [`TEST_PROMPT`] through the same one-shot path the analysis helpers use
/// and return the trimmed reply. An empty reply counts as a failure: the CLI
/// started but produced nothing a cue run could use.
fn run_connection_test(
    settings: &Settings,
    provider: &CliProvider,
    project_root: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    let reply = crate::app::split_cue::run_cli_prompt(
        TEST_PROMPT,
        provider,
        project_root,
        settings,
        cancel,
    )?;
    let reply = reply.trim();
    if reply.is_empty() {
        return Err(format!(
            "{} started but returned an empty reply",
            provider.display_name()
        ));
    }
    Ok(reply.to_string())
}

/// Pre/post-run scripts (formatters, builds) are irrelevant to whether the CLI
/// answers, and shouldn't run just because someone pressed "Test".
fn without_run_scripts(settings: &Settings, provider: &CliProvider) -> Settings {
    let mut settings = settings.clone();
    let (pre, post) = match provider {
        CliProvider::Claude => (
            &mut settings.claude_pre_run_script,
            &mut settings.claude_post_run_script,
        ),
        CliProvider::OpenCode => (
            &mut settings.opencode_pre_run_script,
            &mut settings.opencode_post_run_script,
        ),
        CliProvider::Gemini => (
            &mut settings.gemini_pre_run_script,
            &mut settings.gemini_post_run_script,
        ),
        CliProvider::Codex => (
            &mut settings.codex_pre_run_script,
            &mut settings.codex_post_run_script,
        ),
        CliProvider::Acp => (
            &mut settings.acp_pre_run_script,
            &mut settings.acp_post_run_script,
        ),
    };
    pre.clear();
    post.clear();
    settings
}

/// First line of `text`, shortened to fit next to the button.
fn reply_preview(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or_default().trim();
    let mut preview: String = first_line.chars().take(REPLY_PREVIEW_CHARS).collect();
    if first_line.chars().count() > REPLY_PREVIEW_CHARS || text.lines().nth(1).is_some() {
        preview.push('\u{2026}');
    }
    preview
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal ACP agent: answers one prompt with `$1` as the reply text.
    const REPLYING_AGENT: &str = r#"
read -r init
echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"fake","version":"0.1"}}}'
read -r new
echo '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}'
read -r prompt
echo "$prompt" > prompt.json
echo "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"s1\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"$1\"}}}}"
echo '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}'
"#;

    fn acp_settings(root: &Path, reply: &str) -> Settings {
        let script = root.join("agent.sh");
        std::fs::write(&script, REPLYING_AGENT).unwrap();
        Settings {
            cli_provider: CliProvider::Acp,
            acp_cli_path: "sh".into(),
            acp_extra_args: format!("{} '{reply}'", script.display()),
            acp_pre_run_script: "touch pre-ran".into(),
            acp_post_run_script: "touch post-ran".into(),
            ..Settings::default()
        }
    }

    fn run(settings: &Settings, root: &Path) -> Result<String, String> {
        run_connection_test(
            &without_run_scripts(settings, &CliProvider::Acp),
            &CliProvider::Acp,
            root,
            Arc::new(AtomicBool::new(false)),
        )
    }

    #[test]
    fn reply_passes_and_scripts_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let settings = acp_settings(&root, "Hello! How can I help?");

        assert_eq!(run(&settings, &root).unwrap(), "Hello! How can I help?");
        let prompt = std::fs::read_to_string(root.join("prompt.json")).unwrap();
        assert!(prompt.contains(r#""text":"hello""#), "{prompt}");
        assert!(!root.join("pre-ran").exists());
        assert!(!root.join("post-ran").exists());
    }

    #[test]
    fn empty_reply_fails() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let err = run(&acp_settings(&root, "  "), &root).unwrap_err();
        assert!(err.contains("empty reply"), "{err}");
    }

    #[test]
    fn missing_binary_fails() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            cli_provider: CliProvider::Acp,
            acp_cli_path: "/nonexistent/acp-agent".into(),
            ..Settings::default()
        };
        assert!(run(&settings, dir.path()).is_err());
    }

    /// Runs the real test against a provider configured in this repo's saved
    /// settings; ACP uses `ACP_SMOKE_AGENT` when set. Costs a few tokens:
    /// `CONNECTION_SMOKE_PROVIDER=claude cargo test connection_smoke -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn connection_smoke_real_provider() {
        let name =
            std::env::var("CONNECTION_SMOKE_PROVIDER").expect("set CONNECTION_SMOKE_PROVIDER");
        let provider = CliProvider::all()
            .iter()
            .find(|p| {
                p.display_name()
                    .to_lowercase()
                    .starts_with(&name.to_lowercase())
            })
            .unwrap_or_else(|| panic!("unknown provider {name}"))
            .clone();
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut settings = crate::settings::load_settings(root);
        if let Ok(spec) = std::env::var("ACP_SMOKE_AGENT") {
            let (cmd, args) = spec.split_once(' ').unwrap_or((spec.as_str(), ""));
            settings.acp_cli_path = cmd.into();
            settings.acp_extra_args = args.into();
        }
        let reply = run_connection_test(
            &without_run_scripts(&settings, &provider),
            &provider,
            root,
            Arc::new(AtomicBool::new(false)),
        );
        println!("{}: {reply:?}", provider.display_name());
        reply.unwrap();
    }

    #[test]
    fn preview_keeps_the_first_line_short() {
        assert_eq!(reply_preview("hi"), "hi");
        assert_eq!(reply_preview("hi\nthere"), "hi\u{2026}");
        let long = "x".repeat(100);
        assert_eq!(
            reply_preview(&long).chars().count(),
            REPLY_PREVIEW_CHARS + 1
        );
    }
}
