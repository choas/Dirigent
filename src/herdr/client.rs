//! Talking to the herdr server.
//!
//! The `herdr` CLI is used once per run, to locate the server's socket
//! (`herdr status server --json` applies herdr's own session / env /
//! config-dir resolution rules). Every request after that goes straight over
//! the socket as newline-delimited JSON, so neither the prompt nor the
//! forwarded environment ever appears in a process argv (`ps`, `ARG_MAX`).

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

use crate::ai_backend::BackendError;

/// A failed herdr request.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum HerdrError {
    /// The server answered with an error body (`pane_not_found`,
    /// `agent_blocked`, …).
    Api { code: String, message: String },
    /// The request never got a well-formed answer.
    Transport(String),
}

impl HerdrError {
    pub(crate) fn code(&self) -> Option<&str> {
        match self {
            HerdrError::Api { code, .. } => Some(code),
            HerdrError::Transport(_) => None,
        }
    }
}

impl std::fmt::Display for HerdrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HerdrError::Api { code, message } => write!(f, "{message} ({code})"),
            HerdrError::Transport(msg) => write!(f, "{msg}"),
        }
    }
}

/// The herdr socket API as the run loop sees it: one request, one result.
/// [`SocketClient`] is the real thing; tests script a fake.
pub(crate) trait HerdrApi {
    fn call(&self, method: &str, params: Value) -> Result<Value, HerdrError>;
}

/// Resolve the `herdr` binary. A configured path that does not resolve is an
/// error — silently running whatever `herdr` is first on PATH instead would
/// hide a misconfiguration (and could talk to a different install).
pub(crate) fn resolve_herdr_binary(cli_path: &str) -> Result<String, BackendError> {
    resolve_herdr_binary_with(cli_path, crate::settings::resolve_in_path)
}

fn resolve_herdr_binary_with(
    cli_path: &str,
    resolve: impl Fn(&str) -> Option<String>,
) -> Result<String, BackendError> {
    let configured = cli_path.trim();
    if configured.is_empty() {
        return resolve("herdr").ok_or(BackendError::NotFound { backend: "herdr" });
    }
    resolve(configured).ok_or_else(|| BackendError::Failed {
        message: format!(
            "herdr CLI path `{configured}` from Settings was not found \
             (fix it in Settings \u{2192} General, or clear it to use herdr from PATH)"
        ),
    })
}

/// Ask the herdr CLI where the running server's socket is.
pub(crate) fn discover_socket(herdr: &str) -> Result<PathBuf, BackendError> {
    let output = Command::new(herdr)
        .args(["status", "server", "--json"])
        .output()
        .map_err(|source| BackendError::SpawnFailed {
            backend: "herdr",
            source,
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_server_status(&stdout).map_err(|message| {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        BackendError::Failed {
            message: if detail.is_empty() {
                message
            } else {
                format!("{message}: {detail}")
            },
        }
    })
}

/// Check that the configured herdr resolves, its server runs, and the socket
/// answers. Used by Settings → Test Connection.
pub(crate) fn check_server(cli_path: &str) -> Result<(), String> {
    let herdr = resolve_herdr_binary(cli_path).map_err(|e| e.to_string())?;
    let api = SocketClient::new(discover_socket(&herdr).map_err(|e| e.to_string())?);
    api.call("ping", serde_json::json!({}))
        .map(|_| ())
        .map_err(|e| format!("herdr server did not answer: {e}"))
}

/// Interpret `herdr status server --json`.
pub(crate) fn parse_server_status(stdout: &str) -> Result<PathBuf, String> {
    let status: Value = serde_json::from_str(stdout.trim())
        .map_err(|_| "could not read `herdr status server --json` output".to_string())?;
    if status.get("running").and_then(Value::as_bool) != Some(true) {
        return Err("the herdr server is not running \u{2014} start herdr first".to_string());
    }
    status
        .get("socket")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "herdr reported no socket path".to_string())
}

/// One connection per request, as the herdr CLI does. Requests are small and
/// rare (a few per second while a run is waiting), so pooling isn't worth the
/// reconnect handling.
pub(crate) struct SocketClient {
    socket: PathBuf,
    next_id: std::sync::atomic::AtomicU64,
}

/// Generous: `tab.create` starts a login shell, and a busy server may be slow.
#[cfg(unix)]
const SOCKET_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

impl SocketClient {
    pub(crate) fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            next_id: std::sync::atomic::AtomicU64::new(1),
        }
    }
}

impl HerdrApi for SocketClient {
    #[cfg(unix)]
    fn call(&self, method: &str, params: Value) -> Result<Value, HerdrError> {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixStream;

        let transport = |e: std::io::Error| {
            HerdrError::Transport(format!(
                "herdr socket {}: {e}",
                self.socket.to_string_lossy()
            ))
        };
        let id = format!(
            "dirigent-{}",
            self.next_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let mut line = serde_json::to_string(&serde_json::json!({
            "id": id,
            "method": method,
            "params": params,
        }))
        .map_err(|e| HerdrError::Transport(e.to_string()))?;
        line.push('\n');

        let mut stream = UnixStream::connect(&self.socket).map_err(transport)?;
        stream
            .set_read_timeout(Some(SOCKET_TIMEOUT))
            .map_err(transport)?;
        stream
            .set_write_timeout(Some(SOCKET_TIMEOUT))
            .map_err(transport)?;
        stream.write_all(line.as_bytes()).map_err(transport)?;

        let mut reply = String::new();
        BufReader::new(stream)
            .read_line(&mut reply)
            .map_err(transport)?;
        parse_response(&id, &reply)
    }

    #[cfg(not(unix))]
    fn call(&self, _method: &str, _params: Value) -> Result<Value, HerdrError> {
        Err(HerdrError::Transport(
            "the Herdr backend needs a Unix socket and is not supported on this platform"
                .to_string(),
        ))
    }
}

/// Split a response line into the `result` or the error body.
pub(crate) fn parse_response(id: &str, reply: &str) -> Result<Value, HerdrError> {
    if reply.trim().is_empty() {
        return Err(HerdrError::Transport(
            "herdr closed the connection without answering".to_string(),
        ));
    }
    let mut value: Value = serde_json::from_str(reply.trim())
        .map_err(|e| HerdrError::Transport(format!("unreadable herdr response ({e})")))?;
    if let Some(error) = value.get("error") {
        let field = |name: &str| {
            error
                .get(name)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        return Err(HerdrError::Api {
            code: field("code"),
            message: field("message"),
        });
    }
    if value.get("id").and_then(Value::as_str) != Some(id) {
        return Err(HerdrError::Transport(format!(
            "herdr answered a different request (expected id {id})"
        )));
    }
    value
        .get_mut("result")
        .map(Value::take)
        .ok_or_else(|| HerdrError::Transport("herdr response has no result".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_herdr_path_that_does_not_resolve_is_an_error() {
        let resolve = |name: &str| (name == "herdr").then(|| "/usr/bin/herdr".to_string());
        let err = resolve_herdr_binary_with("/opt/missing/herdr", resolve).unwrap_err();
        assert!(
            err.to_string().contains("/opt/missing/herdr"),
            "must name the bad path, not fall back to PATH: {err}"
        );
    }

    #[test]
    fn empty_herdr_path_uses_path_lookup() {
        let resolve = |name: &str| (name == "herdr").then(|| "/usr/bin/herdr".to_string());
        assert_eq!(
            resolve_herdr_binary_with("  ", resolve).unwrap(),
            "/usr/bin/herdr"
        );
        assert!(matches!(
            resolve_herdr_binary_with("", |_| None),
            Err(BackendError::NotFound { backend: "herdr" })
        ));
    }

    #[test]
    fn server_status_yields_socket_or_a_readable_reason() {
        let ok =
            r#"{"status":"running","running":true,"version":"0.9.0","socket":"/h/herdr.sock"}"#;
        assert_eq!(
            parse_server_status(ok).unwrap(),
            PathBuf::from("/h/herdr.sock")
        );

        let stopped = r#"{"status":"stopped","running":false,"socket":"/h/herdr.sock"}"#;
        assert!(
            parse_server_status(stopped)
                .unwrap_err()
                .contains("not running")
        );
        assert!(parse_server_status("herdr: unknown flag").is_err());
        assert!(parse_server_status(r#"{"running":true}"#).is_err());
    }

    #[test]
    fn response_parsing_separates_result_and_error() {
        assert_eq!(
            parse_response("r1", r#"{"id":"r1","result":{"type":"ok"}}"#).unwrap(),
            serde_json::json!({"type":"ok"})
        );
        let err = parse_response(
            "r2",
            r#"{"id":"r2","error":{"code":"pane_not_found","message":"pane w1:p1 not found"}}"#,
        )
        .unwrap_err();
        assert_eq!(err.code(), Some("pane_not_found"));
        // Errors for undecodable requests carry an empty id; still an API error.
        let err =
            parse_response("r3", r#"{"id":"","error":{"code":"bad","message":"m"}}"#).unwrap_err();
        assert_eq!(err.code(), Some("bad"));

        assert!(matches!(
            parse_response("r4", ""),
            Err(HerdrError::Transport(_))
        ));
        assert!(matches!(
            parse_response("r5", r#"{"id":"other","result":{}}"#),
            Err(HerdrError::Transport(_))
        ));
    }

    /// Real round trip against a stub server on a Unix socket.
    #[cfg(unix)]
    #[test]
    fn socket_client_round_trip() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("herdr.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let reply = serde_json::json!({
                "id": request["id"],
                "result": {"echo": request["params"], "method": request["method"]},
            });
            (&stream)
                .write_all(format!("{reply}\n").as_bytes())
                .unwrap();
        });

        let client = SocketClient::new(path);
        let result = client
            .call(
                "pane.send_text",
                serde_json::json!({"pane_id": "w1:p1", "text": "-rf\nline two"}),
            )
            .unwrap();
        server.join().unwrap();
        assert_eq!(result["method"], "pane.send_text");
        assert_eq!(result["echo"]["text"], "-rf\nline two");
    }
}
