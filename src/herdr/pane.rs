//! Pane operations over [`HerdrApi`] and the decisions the run loop makes from
//! what it observes. Everything here is synchronous and free of real sleeps
//! and clocks (both are injected), so the cancel and pane-exit logic is unit
//! tested against a scripted herdr.

use std::time::Duration;

use serde_json::{Value, json};

use super::client::{HerdrApi, HerdrError};

/// herdr's agent label for Claude Code.
const CLAUDE_AGENT: &str = "claude";

/// Poll cadence and patience of a pane run.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Timing {
    pub poll: Duration,
    /// Polls to wait for herdr to detect Claude Code before giving up on
    /// detection (a login shell plus Claude's startup takes ~5–10 s).
    pub boot_polls: u32,
    /// Extra wait after herdr reports Claude as idle: detection can fire just
    /// before the TUI input box accepts text.
    pub ready_grace: Duration,
    /// Gap between pasted text and Enter when falling back to raw input.
    pub submit_delay: Duration,
    /// Consecutive "Claude is gone" observations before the run fails. One
    /// poll can see a transient state (detection lag, a subprocess in the
    /// foreground), so a single miss is never enough.
    pub gone_polls: u32,
    /// Polls to wait for Claude to exit after an interrupt before the pane is
    /// closed instead.
    pub interrupt_polls: u32,
}

impl Timing {
    pub(crate) const DEFAULT: Timing = Timing {
        poll: Duration::from_millis(500),
        boot_polls: 60,
        ready_grace: Duration::from_secs(2),
        submit_delay: Duration::from_millis(300),
        gone_polls: 4,
        interrupt_polls: 10,
    };
}

/// One look at the pane.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PaneObservation {
    Closed,
    Open {
        /// herdr's detected agent (`"claude"`), if any.
        agent: Option<String>,
        /// herdr's agent status: `idle`, `working`, `blocked`, `done`, `unknown`.
        status: Option<String>,
        /// Whether the pane's shell is the foreground process group, i.e. no
        /// command (Claude) is running in it. `None` when herdr could not say.
        shell_in_foreground: Option<bool>,
    },
}

impl PaneObservation {
    pub(crate) fn agent_is_claude(&self) -> bool {
        matches!(self, PaneObservation::Open { agent: Some(a), .. } if a == CLAUDE_AGENT)
    }

    fn status(&self) -> Option<&str> {
        match self {
            PaneObservation::Open { status, .. } => status.as_deref(),
            PaneObservation::Closed => None,
        }
    }

    fn shell_in_foreground(&self) -> Option<bool> {
        match self {
            PaneObservation::Open {
                shell_in_foreground,
                ..
            } => *shell_in_foreground,
            PaneObservation::Closed => None,
        }
    }
}

pub(crate) fn is_pane_gone(err: &HerdrError) -> bool {
    matches!(err.code(), Some("pane_not_found" | "tab_not_found"))
}

/// Observe a pane via `pane.get` (agent, status) and `pane.process_info`
/// (foreground process group).
pub(crate) fn observe(api: &dyn HerdrApi, pane_id: &str) -> Result<PaneObservation, HerdrError> {
    let info = match api.call("pane.get", json!({ "pane_id": pane_id })) {
        Ok(info) => info,
        Err(e) if is_pane_gone(&e) => return Ok(PaneObservation::Closed),
        Err(e) => return Err(e),
    };
    let text = |ptr: &str| info.pointer(ptr).and_then(Value::as_str).map(String::from);
    let shell_in_foreground = match api.call("pane.process_info", json!({ "pane_id": pane_id })) {
        Ok(p) => {
            let shell = p.pointer("/process_info/shell_pid").and_then(Value::as_u64);
            let fg = p
                .pointer("/process_info/foreground_process_group_id")
                .and_then(Value::as_u64);
            shell.zip(fg).map(|(shell, fg)| shell == fg)
        }
        Err(e) if is_pane_gone(&e) => return Ok(PaneObservation::Closed),
        Err(_) => None,
    };
    Ok(PaneObservation::Open {
        agent: text("/pane/agent"),
        status: text("/pane/agent_status"),
        shell_in_foreground,
    })
}

/// Where and how to open the run's tab.
pub(crate) struct TabSpec<'a> {
    pub cwd: &'a str,
    /// The project's herdr workspace label (the project folder's name).
    pub workspace: &'a str,
    pub label: &'a str,
    pub env: &'a [(String, String)],
}

/// Open the run's tab in the project's herdr workspace — the one labelled
/// like the project folder — creating that workspace when there is none.
/// Returns `(tab_id, pane_id)`. The environment travels in the socket
/// request only.
pub(crate) fn open_run_tab(
    api: &dyn HerdrApi,
    spec: &TabSpec<'_>,
) -> Result<(String, String), HerdrError> {
    let env: serde_json::Map<String, Value> = spec
        .env
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    let created = match find_workspace(api, spec.workspace)? {
        Some(workspace_id) => api.call(
            "tab.create",
            json!({ "workspace_id": workspace_id, "cwd": spec.cwd, "label": spec.label,
                    "focus": true, "env": env }),
        )?,
        // A new workspace comes with its first tab; name that one for the run.
        None => {
            let created = api.call(
                "workspace.create",
                json!({ "cwd": spec.cwd, "label": spec.workspace, "focus": true, "env": env }),
            )?;
            if let Some(tab_id) = created.pointer("/tab/tab_id").and_then(Value::as_str) {
                // Cosmetic; the run works under the default tab name too.
                let _ = api.call(
                    "tab.rename",
                    json!({ "tab_id": tab_id, "label": spec.label }),
                );
            }
            created
        }
    };
    let id = |ptr: &str| {
        created
            .pointer(ptr)
            .and_then(Value::as_str)
            .map(String::from)
    };
    match (id("/tab/tab_id"), id("/root_pane/pane_id")) {
        (Some(tab), Some(pane)) => Ok((tab, pane)),
        _ => Err(HerdrError::Transport(format!(
            "herdr returned no tab/pane id: {created}"
        ))),
    }
}

/// The id of the workspace labelled `label`, preferring the focused one when
/// several share the label.
fn find_workspace(api: &dyn HerdrApi, label: &str) -> Result<Option<String>, HerdrError> {
    let listed = api.call("workspace.list", json!({}))?;
    let matching: Vec<&Value> = listed
        .get("workspaces")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|w| w.get("label").and_then(Value::as_str) == Some(label))
        .collect();
    let chosen = matching
        .iter()
        .find(|w| w.get("focused").and_then(Value::as_bool) == Some(true))
        .or(matching.first());
    Ok(chosen
        .and_then(|w| w.get("workspace_id"))
        .and_then(Value::as_str)
        .map(String::from))
}

/// Type `command` into the pane's shell and press Enter.
pub(crate) fn launch(api: &dyn HerdrApi, pane_id: &str, command: &str) -> Result<(), HerdrError> {
    api.call(
        "pane.send_input",
        json!({ "pane_id": pane_id, "text": command, "keys": ["enter"] }),
    )
    .map(|_| ())
}

/// Submit the prompt to Claude. `agent.prompt` honours the TUI's bracketed
/// paste mode and delays Enter itself; when herdr has not detected the agent,
/// paste the text and press Enter separately.
pub(crate) fn submit_prompt(
    api: &dyn HerdrApi,
    pane_id: &str,
    prompt: &str,
    agent_detected: bool,
    timing: &Timing,
    sleep: &dyn Fn(Duration),
) -> Result<(), HerdrError> {
    if agent_detected {
        return api
            .call("agent.prompt", json!({ "target": pane_id, "text": prompt }))
            .map(|_| ());
    }
    api.call(
        "pane.send_text",
        json!({ "pane_id": pane_id, "text": prompt }),
    )?;
    sleep(timing.submit_delay);
    api.call(
        "pane.send_keys",
        json!({ "pane_id": pane_id, "keys": ["enter"] }),
    )
    .map(|_| ())
}

/// The pane's recent text, as a last-resort conversation log.
pub(crate) fn read_text(api: &dyn HerdrApi, pane_id: &str) -> Option<String> {
    let read = api
        .call(
            "pane.read",
            json!({ "pane_id": pane_id, "source": "recent_unwrapped", "lines": 400 }),
        )
        .ok()?;
    read.pointer("/read/text")
        .and_then(Value::as_str)
        .map(String::from)
}

/// What the boot phase should do after an observation.
#[derive(Debug, PartialEq)]
pub(crate) enum BootStep {
    Wait,
    /// Claude is showing a dialog (folder trust, login, …) only the user can
    /// answer in the pane; wait without a time limit.
    WaitingForUser,
    /// herdr sees Claude idle at its prompt.
    Ready,
    /// herdr never detected Claude, but something other than the shell is in
    /// the foreground; send the prompt as raw input.
    ReadyUndetected,
    Failed(String),
}

/// Boot-phase decisions: wait for Claude to be ready for its prompt without
/// ever typing the prompt into a bare shell (it would run as commands).
pub(crate) struct BootWatch {
    polls: u32,
    max_polls: u32,
    exit: ExitWatch,
}

impl BootWatch {
    pub(crate) fn new(timing: &Timing) -> Self {
        Self {
            polls: 0,
            max_polls: timing.boot_polls,
            // Until herdr has seen Claude, a shell in the foreground is the
            // normal state (the login shell is still starting), so only the
            // agent signal counts here.
            exit: ExitWatch::new(false, false, timing.gone_polls),
        }
    }

    pub(crate) fn step(&mut self, obs: &PaneObservation) -> BootStep {
        if let Some(reason) = self.exit.observe(obs) {
            return BootStep::Failed(format!("{reason} before it was ready for the prompt"));
        }
        if obs.agent_is_claude() {
            match obs.status() {
                Some("idle" | "done") => return BootStep::Ready,
                Some("blocked") => return BootStep::WaitingForUser,
                _ => {}
            }
        }
        self.polls += 1;
        if self.polls < self.max_polls {
            return BootStep::Wait;
        }
        if obs.shell_in_foreground() == Some(false) {
            BootStep::ReadyUndetected
        } else {
            BootStep::Failed(
                "Claude Code did not start in the herdr pane \u{2014} see the pane for its output"
                    .to_string(),
            )
        }
    }
}

/// Detects that Claude is gone: the pane closed, or Claude exited (`/exit`,
/// crash, Ctrl-C) for several consecutive polls.
pub(crate) struct ExitWatch {
    seen_running: bool,
    use_process_signal: bool,
    gone_streak: u32,
    gone_polls: u32,
}

impl ExitWatch {
    /// `seen_running`: Claude is known to have been running (after the prompt
    /// was submitted). `use_process_signal`: also treat a non-shell foreground
    /// process as Claude running, which covers herdr losing agent detection.
    pub(crate) fn new(seen_running: bool, use_process_signal: bool, gone_polls: u32) -> Self {
        Self {
            seen_running,
            use_process_signal,
            gone_streak: 0,
            gone_polls: gone_polls.max(1),
        }
    }

    /// `Some(reason)` once the run must fail.
    pub(crate) fn observe(&mut self, obs: &PaneObservation) -> Option<String> {
        if *obs == PaneObservation::Closed {
            return Some("the herdr pane was closed".to_string());
        }
        let running = obs.agent_is_claude()
            || (self.use_process_signal && obs.shell_in_foreground() == Some(false));
        if running {
            self.seen_running = true;
            self.gone_streak = 0;
            return None;
        }
        if !self.seen_running {
            return None;
        }
        self.gone_streak += 1;
        (self.gone_streak >= self.gone_polls)
            .then(|| "Claude Code exited in the herdr pane".to_string())
    }
}

/// How a cancel request was carried out in the pane.
#[derive(Debug, PartialEq)]
pub(crate) enum InterruptOutcome {
    /// Claude stopped after Esc / Ctrl-C; the pane stays open for inspection.
    Exited,
    /// Claude ignored the interrupt, so the pane was closed.
    PaneClosed,
    /// The pane was already gone.
    AlreadyGone,
    Failed(String),
}

impl InterruptOutcome {
    pub(crate) fn describe(&self, pane_id: &str) -> String {
        match self {
            InterruptOutcome::Exited => {
                format!("interrupted Claude Code in herdr pane {pane_id}; it exited")
            }
            InterruptOutcome::PaneClosed => format!(
                "Claude Code in herdr pane {pane_id} did not exit after Ctrl-C; closed the pane"
            ),
            InterruptOutcome::AlreadyGone => format!("herdr pane {pane_id} was already closed"),
            InterruptOutcome::Failed(e) => format!(
                "could not stop Claude Code in herdr pane {pane_id} ({e}); stop it in herdr"
            ),
        }
    }
}

/// Stop Claude in the pane: Esc ends the current turn, Ctrl-C twice exits
/// Claude Code. If it is still running afterwards, close the pane.
pub(crate) fn interrupt(
    api: &dyn HerdrApi,
    pane_id: &str,
    timing: &Timing,
    sleep: &dyn Fn(Duration),
) -> InterruptOutcome {
    for keys in [["escape"], ["ctrl+c"], ["ctrl+c"]] {
        match api.call(
            "pane.send_keys",
            json!({ "pane_id": pane_id, "keys": keys }),
        ) {
            Ok(_) => {}
            Err(e) if is_pane_gone(&e) => return InterruptOutcome::AlreadyGone,
            Err(e) => return InterruptOutcome::Failed(e.to_string()),
        }
        sleep(timing.submit_delay);
    }

    let mut exit = ExitWatch::new(true, true, 1);
    for _ in 0..timing.interrupt_polls {
        sleep(timing.poll);
        match observe(api, pane_id) {
            Ok(PaneObservation::Closed) => return InterruptOutcome::AlreadyGone,
            Ok(obs) => {
                if exit.observe(&obs).is_some() {
                    return InterruptOutcome::Exited;
                }
            }
            Err(_) => {}
        }
    }

    match api.call("pane.close", json!({ "pane_id": pane_id })) {
        Ok(_) => InterruptOutcome::PaneClosed,
        Err(e) if is_pane_gone(&e) => InterruptOutcome::AlreadyGone,
        Err(e) => InterruptOutcome::Failed(e.to_string()),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;

    type Handler = Box<dyn FnMut(&str, &Value) -> Result<Value, HerdrError>>;

    /// A herdr that answers from a closure and records every call.
    pub(crate) struct ScriptedHerdr {
        handler: RefCell<Handler>,
        pub calls: RefCell<Vec<(String, Value)>>,
    }

    impl ScriptedHerdr {
        pub(crate) fn new(
            handler: impl FnMut(&str, &Value) -> Result<Value, HerdrError> + 'static,
        ) -> Self {
            Self {
                handler: RefCell::new(Box::new(handler)),
                calls: RefCell::new(Vec::new()),
            }
        }

        pub(crate) fn methods(&self) -> Vec<String> {
            self.calls.borrow().iter().map(|(m, _)| m.clone()).collect()
        }

        pub(crate) fn params_of(&self, method: &str) -> Vec<Value> {
            self.calls
                .borrow()
                .iter()
                .filter(|(m, _)| m == method)
                .map(|(_, p)| p.clone())
                .collect()
        }
    }

    impl HerdrApi for ScriptedHerdr {
        fn call(&self, method: &str, params: Value) -> Result<Value, HerdrError> {
            self.calls
                .borrow_mut()
                .push((method.to_string(), params.clone()));
            (self.handler.borrow_mut())(method, &params)
        }
    }

    pub(crate) fn api_error(code: &str) -> HerdrError {
        HerdrError::Api {
            code: code.to_string(),
            message: format!("{code} message"),
        }
    }

    pub(crate) fn pane_info(agent: Option<&str>, status: &str) -> Value {
        json!({ "type": "pane_info", "pane": { "pane_id": "w1:p1", "agent": agent, "agent_status": status } })
    }

    pub(crate) fn process_info(shell_in_foreground: bool) -> Value {
        let fg = if shell_in_foreground { 100 } else { 200 };
        json!({ "type": "pane_process_info", "process_info": {
            "pane_id": "w1:p1", "shell_pid": 100, "foreground_process_group_id": fg } })
    }

    pub(crate) fn open(
        agent: Option<&str>,
        status: &str,
        shell_fg: Option<bool>,
    ) -> PaneObservation {
        PaneObservation::Open {
            agent: agent.map(String::from),
            status: Some(status.to_string()),
            shell_in_foreground: shell_fg,
        }
    }

    pub(crate) const TEST_TIMING: Timing = Timing {
        poll: Duration::ZERO,
        boot_polls: 5,
        ready_grace: Duration::ZERO,
        submit_delay: Duration::ZERO,
        gone_polls: 3,
        interrupt_polls: 3,
    };

    #[test]
    fn observe_reads_agent_status_and_foreground() {
        let api = ScriptedHerdr::new(|method, _| match method {
            "pane.get" => Ok(pane_info(Some("claude"), "working")),
            "pane.process_info" => Ok(process_info(false)),
            _ => unreachable!(),
        });
        assert_eq!(
            observe(&api, "w1:p1").unwrap(),
            open(Some("claude"), "working", Some(false))
        );
    }

    #[test]
    fn observe_maps_missing_pane_to_closed_and_tolerates_process_info_errors() {
        let closed = ScriptedHerdr::new(|_, _| Err(api_error("pane_not_found")));
        assert_eq!(observe(&closed, "w1:p1").unwrap(), PaneObservation::Closed);

        let no_proc = ScriptedHerdr::new(|method, _| match method {
            "pane.get" => Ok(pane_info(None, "unknown")),
            _ => Err(HerdrError::Transport("boom".into())),
        });
        assert_eq!(
            observe(&no_proc, "w1:p1").unwrap(),
            open(None, "unknown", None)
        );

        let broken = ScriptedHerdr::new(|_, _| Err(HerdrError::Transport("down".into())));
        assert!(observe(&broken, "w1:p1").is_err());
    }

    fn workspaces() -> Value {
        json!({ "type": "workspace_list", "workspaces": [
            { "workspace_id": "w1", "label": "Other", "focused": true },
            { "workspace_id": "w2", "label": "Dirigent", "focused": false },
            { "workspace_id": "w3", "label": "Dirigent", "focused": true },
        ]})
    }

    fn spec<'a>(env: &'a [(String, String)]) -> TabSpec<'a> {
        TabSpec {
            cwd: "/repo",
            workspace: "Dirigent",
            label: "Dirigent #7",
            env,
        }
    }

    #[test]
    fn run_tab_opens_in_the_project_workspace_with_env() {
        let api = ScriptedHerdr::new(|method, _| match method {
            "workspace.list" => Ok(workspaces()),
            "tab.create" => Ok(json!({ "type": "tab_created", "tab": { "tab_id": "w3:t2" },
                                      "root_pane": { "pane_id": "w3:p2" } })),
            other => panic!("unexpected {other}"),
        });
        let env = vec![("API_KEY".to_string(), "s3cret".to_string())];
        let ids = open_run_tab(&api, &spec(&env)).unwrap();
        assert_eq!(ids, ("w3:t2".to_string(), "w3:p2".to_string()));
        let params = &api.params_of("tab.create")[0];
        // Of two "Dirigent" workspaces, the focused one wins.
        assert_eq!(params["workspace_id"], "w3");
        assert_eq!(params["label"], "Dirigent #7");
        assert_eq!(params["env"]["API_KEY"], "s3cret");
        assert_eq!(params["cwd"], "/repo");
        assert_eq!(params["focus"], true);
    }

    #[test]
    fn run_tab_creates_the_project_workspace_when_missing() {
        let api = ScriptedHerdr::new(|method, _| match method {
            "workspace.list" => Ok(json!({ "workspaces": [] })),
            "workspace.create" => Ok(json!({ "type": "workspace_created",
                "workspace": { "workspace_id": "w9" }, "tab": { "tab_id": "w9:t1" },
                "root_pane": { "pane_id": "w9:p1" } })),
            "tab.rename" => Ok(json!({ "type": "ok" })),
            other => panic!("unexpected {other}"),
        });
        let env = vec![("API_KEY".to_string(), "s3cret".to_string())];
        let ids = open_run_tab(&api, &spec(&env)).unwrap();
        assert_eq!(ids, ("w9:t1".to_string(), "w9:p1".to_string()));
        let created = &api.params_of("workspace.create")[0];
        assert_eq!(created["label"], "Dirigent");
        assert_eq!(created["env"]["API_KEY"], "s3cret");
        assert_eq!(api.params_of("tab.rename")[0]["label"], "Dirigent #7");
    }

    #[test]
    fn submit_prompt_uses_agent_prompt_when_detected_else_raw_input() {
        let api = ScriptedHerdr::new(|_, _| Ok(json!({ "type": "ok" })));
        submit_prompt(&api, "w1:p1", "-v\nmulti", true, &TEST_TIMING, &|_| {}).unwrap();
        assert_eq!(api.methods(), vec!["agent.prompt"]);
        assert_eq!(api.params_of("agent.prompt")[0]["text"], "-v\nmulti");

        let api = ScriptedHerdr::new(|_, _| Ok(json!({ "type": "ok" })));
        submit_prompt(&api, "w1:p1", "fix it", false, &TEST_TIMING, &|_| {}).unwrap();
        assert_eq!(api.methods(), vec!["pane.send_text", "pane.send_keys"]);
    }

    #[test]
    fn boot_waits_for_idle_claude_and_never_times_out_on_a_dialog() {
        let mut boot = BootWatch::new(&TEST_TIMING);
        assert_eq!(
            boot.step(&open(None, "unknown", Some(true))),
            BootStep::Wait
        );
        assert_eq!(
            boot.step(&open(Some("claude"), "unknown", Some(false))),
            BootStep::Wait
        );
        for _ in 0..20 {
            assert_eq!(
                boot.step(&open(Some("claude"), "blocked", Some(false))),
                BootStep::WaitingForUser
            );
        }
        assert_eq!(
            boot.step(&open(Some("claude"), "idle", Some(false))),
            BootStep::Ready
        );
    }

    #[test]
    fn boot_without_detection_sends_only_when_a_program_is_in_the_foreground() {
        let mut boot = BootWatch::new(&TEST_TIMING);
        for _ in 0..TEST_TIMING.boot_polls - 1 {
            assert_eq!(
                boot.step(&open(None, "unknown", Some(false))),
                BootStep::Wait
            );
        }
        assert_eq!(
            boot.step(&open(None, "unknown", Some(false))),
            BootStep::ReadyUndetected
        );

        // A bare shell must never receive the prompt: it would run it.
        let mut boot = BootWatch::new(&TEST_TIMING);
        let mut last = BootStep::Wait;
        for _ in 0..TEST_TIMING.boot_polls {
            last = boot.step(&open(None, "unknown", Some(true)));
        }
        assert!(matches!(last, BootStep::Failed(ref m) if m.contains("did not start")));
    }

    #[test]
    fn boot_fails_when_the_pane_closes_or_claude_quits_early() {
        let mut boot = BootWatch::new(&TEST_TIMING);
        assert!(matches!(
            boot.step(&PaneObservation::Closed),
            BootStep::Failed(m) if m.contains("closed")
        ));

        let mut boot = BootWatch::new(&TEST_TIMING);
        boot.step(&open(Some("claude"), "unknown", Some(false)));
        let mut last = BootStep::Wait;
        for _ in 0..TEST_TIMING.gone_polls {
            last = boot.step(&open(None, "unknown", Some(true)));
        }
        assert!(matches!(last, BootStep::Failed(m) if m.contains("exited")));
    }

    #[test]
    fn exit_watch_needs_consecutive_misses_and_resets_on_recovery() {
        let mut watch = ExitWatch::new(true, true, 3);
        let gone = open(None, "unknown", Some(true));
        assert_eq!(watch.observe(&gone), None);
        assert_eq!(watch.observe(&gone), None);
        // Detection flickers back: the streak starts over.
        assert_eq!(
            watch.observe(&open(Some("claude"), "working", Some(false))),
            None
        );
        assert_eq!(watch.observe(&gone), None);
        assert_eq!(watch.observe(&gone), None);
        assert!(watch.observe(&gone).unwrap().contains("exited"));
    }

    #[test]
    fn exit_watch_trusts_the_foreground_process_when_detection_is_lost() {
        let mut watch = ExitWatch::new(true, true, 2);
        for _ in 0..10 {
            assert_eq!(watch.observe(&open(None, "unknown", Some(false))), None);
        }
        assert_eq!(
            ExitWatch::new(true, true, 5).observe(&PaneObservation::Closed),
            Some("the herdr pane was closed".to_string())
        );
    }

    /// herdr state for an interrupt test: whether Claude reacts to Ctrl-C.
    fn interrupt_api(claude_exits: bool) -> ScriptedHerdr {
        let mut interrupted = 0;
        ScriptedHerdr::new(move |method, params| match method {
            "pane.send_keys" => {
                if params["keys"][0] == "ctrl+c" {
                    interrupted += 1;
                }
                Ok(json!({ "type": "ok" }))
            }
            "pane.get" if claude_exits && interrupted >= 2 => Ok(pane_info(None, "unknown")),
            "pane.get" => Ok(pane_info(Some("claude"), "working")),
            "pane.process_info" => Ok(process_info(claude_exits && interrupted >= 2)),
            "pane.close" => Ok(json!({ "type": "ok" })),
            other => panic!("unexpected {other}"),
        })
    }

    #[test]
    fn interrupt_sends_escape_and_ctrl_c_and_keeps_the_pane_when_claude_exits() {
        let api = interrupt_api(true);
        assert_eq!(
            interrupt(&api, "w1:p1", &TEST_TIMING, &|_| {}),
            InterruptOutcome::Exited
        );
        let keys: Vec<Value> = api
            .params_of("pane.send_keys")
            .iter()
            .map(|p| p["keys"][0].clone())
            .collect();
        assert_eq!(keys, vec!["escape", "ctrl+c", "ctrl+c"]);
        assert!(!api.methods().contains(&"pane.close".to_string()));
    }

    #[test]
    fn interrupt_closes_the_pane_when_claude_keeps_running() {
        let api = interrupt_api(false);
        assert_eq!(
            interrupt(&api, "w1:p1", &TEST_TIMING, &|_| {}),
            InterruptOutcome::PaneClosed
        );
        assert_eq!(api.methods().last().map(String::as_str), Some("pane.close"));
    }

    #[test]
    fn interrupt_reports_a_pane_that_is_already_gone() {
        let api = ScriptedHerdr::new(|_, _| Err(api_error("pane_not_found")));
        assert_eq!(
            interrupt(&api, "w1:p1", &TEST_TIMING, &|_| {}),
            InterruptOutcome::AlreadyGone
        );
    }
}
