//! One Dirigent run as native Claude Code in a herdr pane.
//!
//! herdr owns the `claude` process, so instead of reading its output this
//! path installs the session-gated Stop hook, starts Claude in a new tab,
//! submits the prompt once Claude is ready, and then waits for the hook's
//! sentinel while watching the pane. The run fails — rather than waiting
//! forever or landing in Review early — when the hook cannot be installed,
//! the pane closes, or Claude exits without the hook firing. Cancel stops
//! Claude in the pane, not just Dirigent's wait.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::client::{HerdrApi, HerdrError, SocketClient, discover_socket, resolve_herdr_binary};
use super::command::{build_claude_command, tab_label};
use super::pane::{
    BootStep, BootWatch, ExitWatch, Timing, create_tab, interrupt, is_pane_gone, launch, observe,
    read_text, submit_prompt,
};
use super::transcript;
use crate::ai_backend::{BackendError, BackendRequest, BackendResponse};
use crate::claude::{self, ClaudeRunMetadata, DoneHook};

/// The hook writes its payload and then touches the sentinel; allow a slow
/// filesystem to order them the other way round.
const PAYLOAD_GRACE: Duration = Duration::from_millis(150);
/// Consecutive failed herdr requests before the run gives up on the server.
const MAX_TRANSPORT_ERRORS: u32 = 10;

/// Run `req` in a herdr pane. Analysis prompts (`read_only`) need captured
/// output and no UI, so they run headless Claude instead.
pub(crate) fn run_herdr_cli(
    req: &BackendRequest<'_>,
    on_log: &mut dyn FnMut(&str),
    cancel: &Arc<AtomicBool>,
) -> Result<BackendResponse, BackendError> {
    if req.read_only {
        let headless = BackendRequest {
            use_pty: false,
            ..*req
        };
        return claude::run_claude_cli(&headless, on_log, Arc::clone(cancel));
    }

    let herdr = resolve_herdr_binary(req.herdr_cli_path)?;
    let claude_bin = claude::resolve_claude_binary(req.cli_path)?;
    let mut args = claude::split_extra_args(req.extra_args);
    args.extend_from_slice(req.extra_args_vec);
    let session_id = claude::extract_session_id(&args);

    let hook = install_stop_hook(req.project_root, session_id.as_deref())?;
    on_log("\u{23CE} Stop hook installed\n");

    let api = SocketClient::new(discover_socket(&herdr)?);

    // A resumed session's transcript already holds earlier runs; remember
    // where this run starts.
    let projects_dir = transcript::claude_projects_dir();
    let prior_transcript = session_id
        .as_deref()
        .zip(projects_dir.as_deref())
        .and_then(|(id, dir)| transcript::find_session_transcript(dir, id))
        .and_then(|path| {
            let len = std::fs::metadata(&path).ok()?.len();
            Some((path, len))
        });

    // Settings env vars and `.Dirigent/.env` reach the pane through the
    // `tab.create` request, never through a command line.
    let env = claude::compose_run_envs(req.env_vars, req.project_root, on_log);
    if !env.is_empty() {
        on_log(&format!(
            "Forwarding {} environment variable(s) to the herdr pane\n",
            env.len()
        ));
    }
    let pane_launch = PaneLaunch {
        cwd: req.project_root.to_string_lossy().into_owned(),
        label: tab_label(req.prompt),
        env,
        command: build_claude_command(&claude_bin, req.model, &args, req.skip_permissions),
        prompt: req.prompt.to_string(),
    };

    let sentinel = hook.sentinel_path().to_path_buf();
    let outcome = drive_pane(
        &api,
        &pane_launch,
        &|| sentinel.exists(),
        cancel,
        on_log,
        &Timing::DEFAULT,
        &std::thread::sleep,
    );
    let pane_id = match outcome {
        PaneRun::Finished { pane_id } => pane_id,
        PaneRun::Cancelled => return Err(BackendError::Cancelled),
        PaneRun::Failed(message) => {
            return Err(BackendError::Failed {
                message: format!("herdr run failed: {message}"),
            });
        }
    };

    std::thread::sleep(PAYLOAD_GRACE);
    let payload = std::fs::read_to_string(hook.payload_path()).ok();
    Ok(collect_response(
        &api,
        &pane_id,
        payload.as_deref(),
        hook.read_summary(),
        RunTranscript {
            session_id: session_id.as_deref(),
            projects_dir: projects_dir.as_deref(),
            prior: prior_transcript,
        },
    ))
}

/// Without the Stop hook nothing tells Dirigent the turn ended, and the cue
/// would land in Review while Claude is still editing.
fn install_stop_hook(
    project_root: &Path,
    session_id: Option<&str>,
) -> Result<DoneHook, BackendError> {
    DoneHook::install(project_root, session_id).map_err(|e| BackendError::Failed {
        message: format!(
            "cannot install the Claude Code Stop hook, so Dirigent could not tell \
             when the herdr run ends: {e:#}"
        ),
    })
}

/// What to start in the pane.
pub(crate) struct PaneLaunch {
    pub cwd: String,
    pub label: String,
    pub env: Vec<(String, String)>,
    pub command: String,
    pub prompt: String,
}

#[derive(Debug, PartialEq)]
pub(crate) enum PaneRun {
    Finished { pane_id: String },
    Cancelled,
    Failed(String),
}

/// Create the tab, start Claude, submit the prompt and wait for the Stop hook
/// (`stop_fired`), the pane or Claude going away, or a cancel.
pub(crate) fn drive_pane(
    api: &dyn HerdrApi,
    pane_launch: &PaneLaunch,
    stop_fired: &dyn Fn() -> bool,
    cancel: &AtomicBool,
    on_log: &mut dyn FnMut(&str),
    timing: &Timing,
    sleep: &dyn Fn(Duration),
) -> PaneRun {
    if cancel.load(Ordering::Relaxed) {
        return PaneRun::Cancelled;
    }
    let pane_id = match create_tab(api, &pane_launch.cwd, &pane_launch.label, &pane_launch.env) {
        Ok((_tab, pane)) => pane,
        Err(e) => return PaneRun::Failed(format!("could not create a herdr tab: {e}")),
    };
    on_log(&format!("Created herdr pane {pane_id}\n"));
    let mut run = PaneSession {
        api,
        pane_id: &pane_id,
        cancel,
        timing,
        sleep,
        transport_errors: 0,
    };

    if let Err(e) = launch(api, &pane_id, &pane_launch.command) {
        return PaneRun::Failed(format!(
            "could not start Claude Code in pane {pane_id}: {e}"
        ));
    }
    on_log("Waiting for Claude Code to become ready\u{2026}\n");
    if let Err(end) = run.submit_when_ready(&pane_launch.prompt, on_log) {
        return end;
    }
    on_log(&format!("Prompt submitted to herdr pane {pane_id}\n"));
    on_log("Waiting for the run to finish (Stop hook)\u{2026}\n");
    match run.wait_for_stop(stop_fired, on_log) {
        Ok(()) => PaneRun::Finished {
            pane_id: pane_id.clone(),
        },
        Err(end) => end,
    }
}

/// The pane-bound part of [`drive_pane`]. Each phase returns `Err(PaneRun)`
/// when the run ends early.
struct PaneSession<'a> {
    api: &'a dyn HerdrApi,
    pane_id: &'a str,
    cancel: &'a AtomicBool,
    timing: &'a Timing,
    sleep: &'a dyn Fn(Duration),
    transport_errors: u32,
}

impl PaneSession<'_> {
    fn submit_when_ready(
        &mut self,
        prompt: &str,
        on_log: &mut dyn FnMut(&str),
    ) -> Result<(), PaneRun> {
        let mut boot = BootWatch::new(self.timing);
        let mut told_user = false;
        loop {
            let obs = self.next_observation(on_log)?;
            let detected = match boot.step(&obs) {
                BootStep::Wait => continue,
                BootStep::WaitingForUser => {
                    if !told_user {
                        on_log(&format!(
                            "Claude Code is waiting for input in herdr pane {} \
                             (e.g. folder trust) \u{2014} answer it there\n",
                            self.pane_id
                        ));
                        told_user = true;
                    }
                    continue;
                }
                BootStep::Ready => obs.agent_is_claude(),
                BootStep::ReadyUndetected => {
                    on_log("herdr did not detect Claude Code; sending the prompt as raw input\n");
                    obs.agent_is_claude()
                }
                BootStep::Failed(reason) => return Err(PaneRun::Failed(reason)),
            };

            (self.sleep)(self.timing.ready_grace);
            self.check_cancel(on_log)?;
            match submit_prompt(
                self.api,
                self.pane_id,
                prompt,
                detected,
                self.timing,
                self.sleep,
            ) {
                Ok(()) => return Ok(()),
                // A dialog opened between the last poll and the prompt; wait
                // for the user to answer it, then try again.
                Err(e) if e.code() == Some("agent_blocked") => {
                    boot = BootWatch::new(self.timing);
                }
                Err(e) if is_pane_gone(&e) => {
                    return Err(PaneRun::Failed(
                        "the herdr pane was closed before the prompt was sent".to_string(),
                    ));
                }
                Err(e) => {
                    return Err(PaneRun::Failed(format!(
                        "could not send the prompt to pane {}: {e}",
                        self.pane_id
                    )));
                }
            }
        }
    }

    fn wait_for_stop(
        &mut self,
        stop_fired: &dyn Fn() -> bool,
        on_log: &mut dyn FnMut(&str),
    ) -> Result<(), PaneRun> {
        let mut exit = ExitWatch::new(true, true, self.timing.gone_polls);
        loop {
            if stop_fired() {
                on_log("\u{23CE} Stop hook fired\n");
                return Ok(());
            }
            let obs = self.next_observation(on_log)?;
            if let Some(reason) = exit.observe(&obs) {
                // The hook may have fired right before Claude exited.
                if stop_fired() {
                    on_log("\u{23CE} Stop hook fired\n");
                    return Ok(());
                }
                return Err(PaneRun::Failed(format!(
                    "{reason} before the Stop hook fired"
                )));
            }
        }
    }

    /// Sleep one poll, honour cancel, and observe the pane. Transport errors
    /// are retried until [`MAX_TRANSPORT_ERRORS`] in a row.
    fn next_observation(
        &mut self,
        on_log: &mut dyn FnMut(&str),
    ) -> Result<super::pane::PaneObservation, PaneRun> {
        loop {
            self.check_cancel(on_log)?;
            (self.sleep)(self.timing.poll);
            self.check_cancel(on_log)?;
            match observe(self.api, self.pane_id) {
                Ok(obs) => {
                    self.transport_errors = 0;
                    return Ok(obs);
                }
                Err(e) => self.note_transport_error(e)?,
            }
        }
    }

    fn note_transport_error(&mut self, e: HerdrError) -> Result<(), PaneRun> {
        self.transport_errors += 1;
        if self.transport_errors >= MAX_TRANSPORT_ERRORS {
            return Err(PaneRun::Failed(format!(
                "lost contact with the herdr server: {e}"
            )));
        }
        Ok(())
    }

    fn check_cancel(&self, on_log: &mut dyn FnMut(&str)) -> Result<(), PaneRun> {
        if !self.cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let outcome = interrupt(self.api, self.pane_id, self.timing, self.sleep);
        on_log(&format!("Cancelled: {}\n", outcome.describe(self.pane_id)));
        Err(PaneRun::Cancelled)
    }
}

/// Where this run's transcript lives and where it starts.
struct RunTranscript<'a> {
    session_id: Option<&'a str>,
    projects_dir: Option<&'a Path>,
    /// Transcript path and length before the run, for a resumed session.
    prior: Option<(PathBuf, u64)>,
}

/// Build the response from the transcript, pane text and Stop-hook payload.
fn collect_response(
    api: &dyn HerdrApi,
    pane_id: &str,
    payload: Option<&str>,
    summary: Option<claude_pty::StopHookSummary>,
    run: RunTranscript<'_>,
) -> BackendResponse {
    let path = payload
        .and_then(transcript::transcript_path_from_payload)
        .or_else(|| {
            run.session_id
                .zip(run.projects_dir)
                .and_then(|(id, dir)| transcript::find_session_transcript(dir, id))
        });
    let offset = match (&path, &run.prior) {
        (Some(path), Some((prior, len))) if path == prior => *len,
        _ => 0,
    };
    let parsed = path
        .and_then(|p| transcript::read_from(&p, offset))
        .map(|raw| transcript::summarize(&raw))
        .unwrap_or_default();
    let stdout = transcript::conversation_log(
        parsed.text,
        || read_text(api, pane_id),
        summary
            .as_ref()
            .and_then(|s| s.last_assistant_message.as_deref()),
    );
    BackendResponse {
        stdout,
        num_turns: parsed.num_turns,
        input_tokens: parsed.input_tokens,
        output_tokens: parsed.output_tokens,
        metadata: Some(ClaudeRunMetadata {
            completion_reason: Some("herdr Stop hook".to_string()),
            stop_hook: summary,
            ..ClaudeRunMetadata::default()
        }),
        ..BackendResponse::default()
    }
}

#[cfg(test)]
mod tests {
    use super::super::pane::tests::{
        ScriptedHerdr, TEST_TIMING, api_error, pane_info, process_info,
    };
    use super::*;
    use serde_json::{Value, json};
    use std::cell::Cell;
    use std::rc::Rc;

    fn pane_launch() -> PaneLaunch {
        PaneLaunch {
            cwd: "/repo".into(),
            label: "Dirigent: fix".into(),
            env: vec![("TOKEN".into(), "s3cret".into())],
            command: "'claude' '--session-id' 'abc'".into(),
            prompt: "fix the bug".into(),
        }
    }

    /// Phases of a fake Claude in a fake herdr pane.
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Claude {
        Booting,
        Idle,
        Blocked,
        Working,
        Exited,
        PaneClosed,
    }

    /// A herdr whose pane hosts a Claude in `state`; `on_call` lets the test
    /// advance the state as calls arrive.
    fn herdr(
        state: Rc<Cell<Claude>>,
        mut on_call: impl FnMut(&str, &Value, &Cell<Claude>) + 'static,
    ) -> ScriptedHerdr {
        ScriptedHerdr::new(move |method, params| {
            on_call(method, params, &state);
            let ok = || Ok(json!({ "type": "ok" }));
            match (method, state.get()) {
                ("tab.create", _) => Ok(json!({
                    "tab": { "tab_id": "w1:t1" }, "root_pane": { "pane_id": "w1:p1" } })),
                (_, Claude::PaneClosed) => Err(api_error("pane_not_found")),
                ("pane.get", Claude::Booting | Claude::Exited) => Ok(pane_info(None, "unknown")),
                ("pane.get", Claude::Idle) => Ok(pane_info(Some("claude"), "idle")),
                ("pane.get", Claude::Blocked) => Ok(pane_info(Some("claude"), "blocked")),
                ("pane.get", Claude::Working) => Ok(pane_info(Some("claude"), "working")),
                ("pane.process_info", s) => {
                    Ok(process_info(matches!(s, Claude::Booting | Claude::Exited)))
                }
                ("agent.prompt", Claude::Blocked) => Err(api_error("agent_blocked")),
                _ => ok(),
            }
        })
    }

    fn drive(
        api: &ScriptedHerdr,
        stop_fired: &dyn Fn() -> bool,
        cancel: &AtomicBool,
    ) -> (PaneRun, String) {
        let mut log = String::new();
        let outcome = drive_pane(
            api,
            &pane_launch(),
            stop_fired,
            cancel,
            &mut |s| log.push_str(s),
            &TEST_TIMING,
            &|_| {},
        );
        (outcome, log)
    }

    #[test]
    fn happy_path_launches_prompts_and_finishes_on_the_stop_hook() {
        let state = Rc::new(Cell::new(Claude::Booting));
        let polls = Rc::new(Cell::new(0));
        let polls_seen = Rc::clone(&polls);
        let api = herdr(Rc::clone(&state), move |method, _, s| match method {
            "pane.get" => {
                polls_seen.set(polls_seen.get() + 1);
                if polls_seen.get() == 2 {
                    s.set(Claude::Idle);
                }
            }
            "agent.prompt" => s.set(Claude::Working),
            _ => {}
        });
        let fired = Rc::clone(&polls);
        let (outcome, log) = drive(&api, &move || fired.get() >= 6, &AtomicBool::new(false));

        assert_eq!(
            outcome,
            PaneRun::Finished {
                pane_id: "w1:p1".into()
            }
        );
        assert!(log.contains("Stop hook fired"), "{log}");
        let input = &api.params_of("pane.send_input")[0];
        assert_eq!(input["text"], "'claude' '--session-id' 'abc'");
        assert_eq!(input["keys"], json!(["enter"]));
        assert_eq!(api.params_of("agent.prompt")[0]["text"], "fix the bug");
        assert_eq!(api.params_of("tab.create")[0]["env"]["TOKEN"], "s3cret");
        // The prompt never travels as raw pane input when the agent is known.
        assert!(api.params_of("pane.send_text").is_empty());
    }

    #[test]
    fn claude_exiting_without_the_stop_hook_fails_the_run() {
        let state = Rc::new(Cell::new(Claude::Idle));
        let api = herdr(Rc::clone(&state), |method, _, s| {
            if method == "agent.prompt" {
                // Claude crashes right after taking the prompt.
                s.set(Claude::Exited);
            }
        });
        let (outcome, _) = drive(&api, &|| false, &AtomicBool::new(false));
        assert!(
            matches!(&outcome, PaneRun::Failed(m) if m.contains("exited") && m.contains("before the Stop hook")),
            "{outcome:?}"
        );
    }

    #[test]
    fn closing_the_pane_fails_the_run() {
        // Closed while Claude works on the prompt.
        let state = Rc::new(Cell::new(Claude::Idle));
        let api = herdr(Rc::clone(&state), |method, _, s| match method {
            "agent.prompt" => s.set(Claude::Working),
            "pane.get" if s.get() == Claude::Working => s.set(Claude::PaneClosed),
            _ => {}
        });
        let (outcome, _) = drive(&api, &|| false, &AtomicBool::new(false));
        assert!(
            matches!(&outcome, PaneRun::Failed(m) if m.contains("pane was closed before the Stop hook")),
            "{outcome:?}"
        );

        // Closed just as the prompt is sent.
        let state = Rc::new(Cell::new(Claude::Idle));
        let api = herdr(Rc::clone(&state), |method, _, s| {
            if method == "agent.prompt" {
                s.set(Claude::PaneClosed);
            }
        });
        let (outcome, _) = drive(&api, &|| false, &AtomicBool::new(false));
        assert!(
            matches!(&outcome, PaneRun::Failed(m) if m.contains("closed before the prompt was sent")),
            "{outcome:?}"
        );
    }

    #[test]
    fn hook_firing_just_before_exit_still_counts_as_finished() {
        let state = Rc::new(Cell::new(Claude::Idle));
        let api = herdr(Rc::clone(&state), |method, _, s| {
            if method == "agent.prompt" {
                s.set(Claude::Exited);
            }
        });
        // Every check before a poll misses; the hook has fired by the time
        // the exit verdict re-checks it.
        let checks = Cell::new(0);
        let stop_fired = || {
            checks.set(checks.get() + 1);
            checks.get() > TEST_TIMING.gone_polls
        };
        let (outcome, _) = drive(&api, &stop_fired, &AtomicBool::new(false));
        assert!(matches!(outcome, PaneRun::Finished { .. }), "{outcome:?}");
    }

    #[test]
    fn cancel_while_waiting_interrupts_claude_in_the_pane() {
        let state = Rc::new(Cell::new(Claude::Idle));
        let cancel = Rc::new(AtomicBool::new(false));
        let cancel_c = Rc::clone(&cancel);
        let api = herdr(Rc::clone(&state), move |method, params, s| match method {
            "agent.prompt" => {
                s.set(Claude::Working);
                cancel_c.store(true, Ordering::Relaxed);
            }
            "pane.send_keys" if params["keys"][0] == "ctrl+c" => s.set(Claude::Exited),
            _ => {}
        });
        let (outcome, log) = drive(&api, &|| false, &cancel);

        assert_eq!(outcome, PaneRun::Cancelled);
        let keys: Vec<Value> = api
            .params_of("pane.send_keys")
            .iter()
            .map(|p| p["keys"][0].clone())
            .collect();
        assert_eq!(keys, vec!["escape", "ctrl+c", "ctrl+c"]);
        assert!(log.contains("Cancelled: interrupted Claude Code"), "{log}");
    }

    #[test]
    fn cancel_before_start_creates_nothing() {
        let api = herdr(Rc::new(Cell::new(Claude::Booting)), |_, _, _| {});
        let (outcome, _) = drive(&api, &|| false, &AtomicBool::new(true));
        assert_eq!(outcome, PaneRun::Cancelled);
        assert!(api.methods().is_empty());
    }

    #[test]
    fn cancel_during_boot_also_stops_the_pane() {
        let state = Rc::new(Cell::new(Claude::Booting));
        let cancel = Rc::new(AtomicBool::new(false));
        let cancel_c = Rc::clone(&cancel);
        let api = herdr(Rc::clone(&state), move |method, _, _| {
            if method == "pane.get" {
                cancel_c.store(true, Ordering::Relaxed);
            }
        });
        let (outcome, log) = drive(&api, &|| false, &cancel);
        assert_eq!(outcome, PaneRun::Cancelled);
        assert!(api.methods().contains(&"pane.send_keys".to_string()));
        assert!(log.contains("Cancelled:"), "{log}");
        assert!(api.params_of("agent.prompt").is_empty());
    }

    #[test]
    fn a_dialog_at_prompt_time_waits_for_the_user_then_retries() {
        let state = Rc::new(Cell::new(Claude::Idle));
        let prompts = Rc::new(Cell::new(0));
        let prompts_c = Rc::clone(&prompts);
        let blocked_polls = Rc::new(Cell::new(0));
        let api = herdr(Rc::clone(&state), move |method, _, s| match method {
            "agent.prompt" => {
                prompts_c.set(prompts_c.get() + 1);
                // A dialog opens just as the first prompt arrives.
                s.set(if prompts_c.get() == 1 {
                    Claude::Blocked
                } else {
                    Claude::Working
                });
            }
            "pane.get" if s.get() == Claude::Blocked => {
                blocked_polls.set(blocked_polls.get() + 1);
                if blocked_polls.get() == 2 {
                    // The user answered it.
                    s.set(Claude::Idle);
                }
            }
            _ => {}
        });
        let checks = Cell::new(0);
        let stop_fired = || {
            checks.set(checks.get() + 1);
            checks.get() > 2
        };
        let (outcome, log) = drive(&api, &stop_fired, &AtomicBool::new(false));
        assert!(matches!(outcome, PaneRun::Finished { .. }), "{outcome:?}");
        assert!(log.contains("answer it there"), "{log}");
        assert_eq!(prompts.get(), 2);
    }

    #[test]
    fn a_shell_that_never_runs_claude_is_never_sent_the_prompt() {
        let state = Rc::new(Cell::new(Claude::Booting));
        let api = herdr(Rc::clone(&state), |_, _, _| {});
        let (outcome, _) = drive(&api, &|| false, &AtomicBool::new(false));
        assert!(
            matches!(&outcome, PaneRun::Failed(m) if m.contains("did not start")),
            "{outcome:?}"
        );
        assert!(api.params_of("agent.prompt").is_empty());
        assert!(api.params_of("pane.send_text").is_empty());
    }

    #[test]
    fn unreachable_server_fails_after_repeated_errors() {
        let api = ScriptedHerdr::new(|method, _| match method {
            "tab.create" => Ok(json!({
                "tab": { "tab_id": "w1:t1" }, "root_pane": { "pane_id": "w1:p1" } })),
            "pane.send_input" => Ok(json!({ "type": "ok" })),
            _ => Err(HerdrError::Transport("connection refused".into())),
        });
        let (outcome, _) = drive(&api, &|| false, &AtomicBool::new(false));
        assert!(
            matches!(&outcome, PaneRun::Failed(m) if m.contains("lost contact")),
            "{outcome:?}"
        );
    }

    #[test]
    fn stop_hook_install_failure_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::fs::write(dir.path().join(".claude/settings.local.json"), "{broken").unwrap();
        let err = match install_stop_hook(dir.path(), Some("abc")) {
            Ok(_) => panic!("must fail"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("Stop hook"), "{err}");
        assert!(err.contains("settings.local.json"), "{err}");
    }

    /// Analysis prompts run headless Claude: a bogus herdr path must not
    /// matter, and no Stop hook or pane is set up.
    #[cfg(unix)]
    #[test]
    fn read_only_requests_run_headless_claude_not_herdr() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let fake_claude = dir.path().join("fake-claude");
        std::fs::write(&fake_claude, "#!/bin/sh\necho headless-ok\n").unwrap();
        std::fs::set_permissions(&fake_claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        let req = BackendRequest {
            prompt: "p",
            project_root: dir.path(),
            model: "",
            cli_path: &fake_claude.to_string_lossy(),
            herdr_cli_path: "/nonexistent/herdr-for-test",
            extra_args: "",
            extra_args_vec: &[],
            env_vars: "",
            pre_run_script: "",
            post_run_script: "",
            hook_trust: crate::ai_backend::HookScriptTrust::Trusted,
            skip_permissions: false,
            use_pty: true,
            read_only: true,
        };
        let response = run_herdr_cli(&req, &mut |_| {}, &Arc::new(AtomicBool::new(false)))
            .expect("headless run");
        assert_eq!(response.stdout.trim(), "headless-ok");
        assert!(!dir.path().join(".claude").exists());
    }

    fn transcript_dir(lines: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("-repo");
        std::fs::create_dir_all(&project).unwrap();
        let path = project.join("sess.jsonl");
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        (dir, path)
    }

    const OLD: &str = r#"{"type":"assistant","message":{"id":"m0","content":[{"type":"text","text":"earlier run"}],"usage":{"output_tokens":1}}}"#;
    const NEW: &str = r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"done <commit-message>x</commit-message>"}],"usage":{"input_tokens":10,"output_tokens":5}}}"#;

    #[test]
    fn response_comes_from_the_payload_transcript_after_the_prior_offset() {
        let (dir, path) = transcript_dir(&[OLD, NEW]);
        let prior_len = (OLD.len() + 1) as u64;
        let payload = json!({ "transcript_path": path, "last_assistant_message": "m" }).to_string();
        let api = ScriptedHerdr::new(|_, _| panic!("pane text not needed"));
        let response = collect_response(
            &api,
            "w1:p1",
            Some(&payload),
            None,
            RunTranscript {
                session_id: Some("sess"),
                projects_dir: Some(dir.path()),
                prior: Some((path.clone(), prior_len)),
            },
        );
        assert_eq!(
            response.stdout,
            "Claude: done <commit-message>x</commit-message>\n"
        );
        assert_eq!(response.num_turns, Some(1));
        assert_eq!(response.input_tokens, Some(10));
        assert_eq!(response.output_tokens, Some(5));
        assert_eq!(
            response.metadata.unwrap().completion_reason.as_deref(),
            Some("herdr Stop hook")
        );
    }

    #[test]
    fn response_falls_back_to_session_scan_then_pane_text() {
        let (dir, _) = transcript_dir(&[NEW]);
        let api = ScriptedHerdr::new(|_, _| panic!("pane text not needed"));
        let scanned = collect_response(
            &api,
            "w1:p1",
            None,
            None,
            RunTranscript {
                session_id: Some("sess"),
                projects_dir: Some(dir.path()),
                prior: None,
            },
        );
        assert!(scanned.stdout.contains("done"));

        let api = ScriptedHerdr::new(|method, _| {
            assert_eq!(method, "pane.read");
            Ok(json!({ "type": "pane_read", "read": { "text": "pane says hi" } }))
        });
        let from_pane = collect_response(
            &api,
            "w1:p1",
            None,
            None,
            RunTranscript {
                session_id: Some("other"),
                projects_dir: Some(dir.path()),
                prior: None,
            },
        );
        assert_eq!(from_pane.stdout, "pane says hi");
        assert_eq!(from_pane.num_turns, None, "unknown, not zero");
    }

    /// End-to-end against a running herdr and a real Claude Code (costs a few
    /// tokens, opens a herdr tab):
    /// `cargo test herdr_smoke -- --ignored --nocapture`
    /// `HERDR_SMOKE_MODEL` overrides the model (default `haiku`). It runs in a
    /// fresh temp dir, so Claude asks whether to trust the folder: answer
    /// "Yes" in the herdr tab and the run continues.
    #[test]
    #[ignore]
    fn herdr_smoke_real_run() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let model = std::env::var("HERDR_SMOKE_MODEL").unwrap_or_else(|_| "haiku".into());
        let session = vec!["--session-id".to_string(), uuid::Uuid::new_v4().to_string()];
        let req = BackendRequest {
            prompt: "Create a file named hello.txt containing exactly: hi\n\
                     Then reply with the single word: done",
            project_root: &root,
            model: &model,
            cli_path: "",
            herdr_cli_path: "",
            extra_args: "",
            extra_args_vec: &session,
            env_vars: "",
            pre_run_script: "",
            post_run_script: "",
            hook_trust: crate::ai_backend::HookScriptTrust::Trusted,
            skip_permissions: true,
            use_pty: true,
            read_only: false,
        };
        let response = run_herdr_cli(
            &req,
            &mut |s| eprint!("{s}"),
            &Arc::new(AtomicBool::new(false)),
        )
        .expect("herdr run");
        eprintln!(
            "--- response ---\n{}\n--- {:?} turns",
            response.stdout, response.num_turns
        );
        assert!(response.stdout.contains("done"), "{}", response.stdout);
        assert_eq!(
            std::fs::read_to_string(root.join("hello.txt"))
                .unwrap()
                .trim(),
            "hi"
        );
        assert!(response.num_turns.unwrap_or(0) >= 1);
    }
}
