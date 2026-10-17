use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use super::client::{AcpConnection, ClientPolicy, Turn};
use super::types::{AcpError, StopReason};
use crate::ai_process;
use crate::claude;

pub(crate) struct AcpRunConfig<'a> {
    /// Agent command, e.g. `claude-code-acp`, `gemini` (with `--experimental-acp`),
    /// `goose` (with `acp`), or an absolute path.
    pub cli_path: &'a str,
    pub extra_args: &'a str,
    pub env_vars: &'a str,
    pub pre_run_script: &'a str,
    pub post_run_script: &'a str,
    /// Approve the agent's permission requests without asking.
    pub skip_permissions: bool,
    /// Analysis prompts: refuse file writes and non-read tool permissions.
    pub read_only: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct AcpResponse {
    /// All `agent_message_chunk` text of the turn.
    pub stdout: String,
    /// Paths the agent reported editing (diff content, edit-tool locations,
    /// `fs/write_text_file`). May be absolute.
    pub edited_files: Vec<String>,
    pub duration_ms: u64,
    pub num_turns: u64,
}

/// Run one prompt turn against an ACP agent: spawn → `initialize` →
/// `session/new` → `session/prompt` (streaming `session/update`s into
/// `on_log`) → close stdin. Every run gets a fresh agent process and session.
pub(crate) fn invoke_acp_streaming(
    prompt: &str,
    project_root: &Path,
    config: &AcpRunConfig<'_>,
    mut on_log: impl FnMut(&str),
    cancel: Arc<AtomicBool>,
) -> Result<AcpResponse, AcpError> {
    let agent_bin = resolve_agent_bin(config.cli_path)?;
    let args = shlex::split(config.extra_args)
        .ok_or_else(|| AcpError::InvalidExtraArgs(config.extra_args.to_string()))?;

    ai_process::run_hook_script(
        "pre-run",
        config.pre_run_script,
        project_root,
        &mut on_log,
        true,
    )
    .map_err(AcpError::Hook)?;

    let mut cmd = Command::new(&agent_bin);
    cmd.args(&args).current_dir(project_root);
    claude::apply_env_vars(&mut cmd, config.env_vars, &mut on_log);
    claude::apply_dirigent_env(&mut cmd, project_root, &mut on_log);

    let policy = ClientPolicy {
        allow_writes: !config.read_only,
        auto_approve: config.skip_permissions,
    };
    let run_start = Instant::now();
    let mut turn = Turn::new(&mut on_log);
    let stop_reason = run_turn(cmd, project_root, policy, prompt, &mut turn, &cancel)?;
    match stop_reason {
        StopReason::Cancelled => return Err(AcpError::Cancelled),
        StopReason::Refusal => return Err(AcpError::Refused),
        StopReason::MaxTokens => turn.log_line("\u{26A0} agent stopped: token limit reached"),
        StopReason::MaxTurnRequests => {
            turn.log_line("\u{26A0} agent stopped: turn request limit reached")
        }
        StopReason::EndTurn | StopReason::Other => {}
    }
    let response = AcpResponse {
        stdout: std::mem::take(&mut turn.text),
        edited_files: std::mem::take(&mut turn.edited_files),
        duration_ms: run_start.elapsed().as_millis() as u64,
        num_turns: turn.tool_calls_completed,
    };
    drop(turn);

    ai_process::run_hook_script(
        "post-run",
        config.post_run_script,
        project_root,
        &mut on_log,
        false,
    )
    .map_err(AcpError::Hook)?;

    Ok(response)
}

/// The connection lives only inside this function, so the agent process is
/// shut down (see `AcpConnection::drop`) before post-run hooks start.
fn run_turn(
    cmd: Command,
    project_root: &Path,
    policy: ClientPolicy,
    prompt: &str,
    turn: &mut Turn,
    cancel: &AtomicBool,
) -> Result<StopReason, AcpError> {
    let mut conn = AcpConnection::spawn(cmd, project_root, policy)?;
    conn.initialize(turn, cancel)?;
    conn.new_session(turn, cancel)?;
    conn.prompt(prompt, turn, cancel)
}

fn resolve_agent_bin(cli_path: &str) -> Result<PathBuf, AcpError> {
    let bin = cli_path.trim();
    if bin.is_empty() {
        return Err(AcpError::NotFound(
            "no agent command configured (Settings \u{2192} CLI Provider \u{2192} ACP Agent)"
                .to_string(),
        ));
    }
    which::which(bin).map_err(|_| AcpError::NotFound(bin.to_string()))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    /// A scripted ACP agent: answers `initialize` (id 1), `session/new` (id 2)
    /// and `session/prompt` (id 3), asking for one file read and one write in
    /// between. Request ids are deterministic because Dirigent numbers its
    /// requests from 1.
    const FAKE_AGENT: &str = r#"
read -r init
echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{},"authMethods":[],"agentInfo":{"name":"fake-agent","version":"0.1"}}}'
read -r new
echo '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}'
read -r prompt
echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Reading. "}}}}'
echo '{"jsonrpc":"2.0","id":"r1","method":"fs/read_text_file","params":{"sessionId":"s1","path":"in.txt"}}'
read -r read_reply
echo "$read_reply" > read_reply.json
echo '{"jsonrpc":"2.0","id":"p1","method":"session/request_permission","params":{"sessionId":"s1","toolCall":{"toolCallId":"t1","title":"Write out.txt","kind":"edit"},"options":[{"optionId":"yes","name":"Allow","kind":"allow_once"},{"optionId":"no","name":"Reject","kind":"reject_once"}]}}'
read -r perm_reply
echo "$perm_reply" > perm_reply.json
echo '{"jsonrpc":"2.0","id":"w1","method":"fs/write_text_file","params":{"sessionId":"s1","path":"out.txt","content":"written"}}'
read -r write_reply
echo '{"jsonrpc":"2.0","id":"x1","method":"terminal/create","params":{"sessionId":"s1","command":"ls"}}'
read -r term_reply
echo "$term_reply" > term_reply.json
echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Done."}}}}'
echo '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}'
"#;

    fn config<'a>(script: &'a str, read_only: bool) -> AcpRunConfig<'a> {
        AcpRunConfig {
            cli_path: "sh",
            extra_args: script,
            env_vars: "",
            pre_run_script: "",
            post_run_script: "",
            skip_permissions: true,
            read_only,
        }
    }

    fn write_script(dir: &Path, body: &str) -> String {
        let path = dir.join("agent.sh");
        std::fs::write(&path, body).unwrap();
        shlex::try_quote(&path.to_string_lossy())
            .unwrap()
            .to_string()
    }

    #[test]
    fn full_turn_against_scripted_agent() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("in.txt"), "input data").unwrap();
        let script = write_script(&root, FAKE_AGENT);

        let mut logged = String::new();
        let response = invoke_acp_streaming(
            "do it",
            &root,
            &config(&script, false),
            |s| logged.push_str(s),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        assert_eq!(response.stdout, "Reading. Done.");
        assert_eq!(
            std::fs::read_to_string(root.join("out.txt")).unwrap(),
            "written"
        );
        assert_eq!(
            response.edited_files,
            vec![root.join("out.txt").to_string_lossy().to_string()]
        );
        let read_reply = std::fs::read_to_string(root.join("read_reply.json")).unwrap();
        assert!(
            read_reply.contains(r#""content":"input data""#),
            "{read_reply}"
        );
        let perm_reply = std::fs::read_to_string(root.join("perm_reply.json")).unwrap();
        assert!(perm_reply.contains(r#""optionId":"yes""#), "{perm_reply}");
        let term_reply = std::fs::read_to_string(root.join("term_reply.json")).unwrap();
        assert!(term_reply.contains("-32601"), "{term_reply}");
        assert!(logged.contains("[ACP: fake-agent 0.1]"), "{logged}");
    }

    #[test]
    fn read_only_run_refuses_writes_and_edit_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("in.txt"), "input data").unwrap();
        let script = write_script(&root, FAKE_AGENT);

        let response = invoke_acp_streaming(
            "analyse",
            &root,
            &config(&script, true),
            |_| {},
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();

        assert!(!root.join("out.txt").exists());
        assert!(response.edited_files.is_empty());
        let perm_reply = std::fs::read_to_string(root.join("perm_reply.json")).unwrap();
        assert!(perm_reply.contains(r#""optionId":"no""#), "{perm_reply}");
    }

    #[test]
    fn auth_required_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let script = write_script(
            dir.path(),
            r#"
read -r init
echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"authMethods":[{"id":"login","name":"Log in"}]}}'
read -r new
echo '{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"Authentication required"}}'
"#,
        );
        let err = invoke_acp_streaming(
            "x",
            dir.path(),
            &config(&script, false),
            |_| {},
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap_err();
        assert!(
            matches!(err, AcpError::AuthRequired(ref m) if m == "Log in"),
            "{err}"
        );
    }

    #[test]
    fn agent_exit_carries_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let script = write_script(dir.path(), "read -r init\necho 'boom: no API key' >&2\n");
        let err = invoke_acp_streaming(
            "x",
            dir.path(),
            &config(&script, false),
            |_| {},
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap_err();
        assert!(
            matches!(err, AcpError::AgentExited(ref s) if s.contains("boom: no API key")),
            "{err}"
        );
    }

    #[test]
    fn cancel_sends_session_cancel_and_stops() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let script = write_script(
            &root,
            r#"
read -r init
echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}'
read -r new
echo '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}'
read -r prompt
read -r cancel
echo "$cancel" > cancel.json
echo '{"jsonrpc":"2.0","id":3,"result":{"stopReason":"cancelled"}}'
"#,
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            flag.store(true, Ordering::Relaxed);
        });
        let err =
            invoke_acp_streaming("x", &root, &config(&script, false), |_| {}, cancel).unwrap_err();
        assert!(matches!(err, AcpError::Cancelled), "{err}");
        let sent = std::fs::read_to_string(root.join("cancel.json")).unwrap();
        assert!(sent.contains(r#""method":"session/cancel""#), "{sent}");
    }

    /// Talks to a real agent, so it is opt-in:
    /// `ACP_SMOKE_AGENT="opencode acp" cargo test acp_smoke -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn acp_smoke_real_agent() {
        let spec = std::env::var("ACP_SMOKE_AGENT").expect("set ACP_SMOKE_AGENT");
        let mut parts = shlex::split(&spec).expect("valid ACP_SMOKE_AGENT");
        let bin = parts.remove(0);
        let args = shlex::try_join(parts.iter().map(String::as_str)).unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "The secret word is mango.\n").unwrap();

        let response = invoke_acp_streaming(
            "Read hello.txt and reply with only the secret word.",
            dir.path(),
            &AcpRunConfig {
                cli_path: &bin,
                extra_args: &args,
                env_vars: "",
                pre_run_script: "",
                post_run_script: "",
                skip_permissions: false,
                read_only: true,
            },
            |s| print!("{s}"),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(
            response.stdout.to_lowercase().contains("mango"),
            "{response:?}"
        );
    }

    /// Like `acp_smoke_real_agent`, but lets the agent edit a file.
    #[test]
    #[ignore]
    fn acp_smoke_real_agent_edit() {
        let spec = std::env::var("ACP_SMOKE_AGENT").expect("set ACP_SMOKE_AGENT");
        let mut parts = shlex::split(&spec).expect("valid ACP_SMOKE_AGENT");
        let bin = parts.remove(0);
        let args = shlex::try_join(parts.iter().map(String::as_str)).unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "fruit = mango\n").unwrap();

        let response = invoke_acp_streaming(
            "In hello.txt replace mango with banana. Do nothing else.",
            dir.path(),
            &AcpRunConfig {
                cli_path: &bin,
                extra_args: &args,
                env_vars: "",
                pre_run_script: "",
                post_run_script: "",
                skip_permissions: true,
                read_only: false,
            },
            |s| print!("{s}"),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        println!("\nedited: {:?}", response.edited_files);
        let content = std::fs::read_to_string(dir.path().join("hello.txt")).unwrap();
        assert!(content.contains("banana"), "{content}");
    }

    #[test]
    fn empty_agent_command_is_a_clear_error() {
        let err = resolve_agent_bin("  ").unwrap_err();
        assert!(err.to_string().contains("no agent command configured"));
    }
}
