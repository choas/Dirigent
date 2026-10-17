//! Wire types for the client side of the Agent Client Protocol (ACP) v1.
//!
//! Only the subset Dirigent needs as a client is modelled. Every inbound type
//! is lenient (optional fields, `#[serde(other)]` catch-alls) so that newer
//! agents adding fields or update kinds do not break a run.
//!
//! Spec: <https://agentclientprotocol.com/protocol/overview>

use serde::{Deserialize, Serialize};

/// ACP major protocol version spoken by this client.
pub(super) const PROTOCOL_VERSION: u32 = 1;

/// JSON-RPC error code agents return when `session/new` needs authentication.
pub(super) const AUTH_REQUIRED_CODE: i64 = -32000;

/// JSON-RPC "method not found", returned for agent requests we do not serve
/// (e.g. `terminal/*`, which we do not advertise).
pub(super) const METHOD_NOT_FOUND_CODE: i64 = -32601;

/// JSON-RPC "invalid params".
pub(super) const INVALID_PARAMS_CODE: i64 = -32602;

/// Any newline-delimited JSON-RPC message read from the agent: a response to
/// one of our requests (`id` + `result`/`error`), a request from the agent
/// (`id` + `method`), or a notification (`method` only).
#[derive(Debug, Deserialize)]
pub(super) struct IncomingMessage {
    pub id: Option<serde_json::Value>,
    pub method: Option<String>,
    pub params: Option<serde_json::Value>,
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

// ---------------------------------------------------------------------------
// initialize
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InitializeParams {
    pub protocol_version: u32,
    pub client_capabilities: ClientCapabilities,
    pub client_info: Implementation,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ClientCapabilities {
    pub fs: FsCapabilities,
    /// Dirigent does not host terminals for the agent.
    pub terminal: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FsCapabilities {
    pub read_text_file: bool,
    pub write_text_file: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Implementation {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub version: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InitializeResult {
    pub protocol_version: Option<u32>,
    #[serde(default)]
    pub auth_methods: Vec<AuthMethod>,
    pub agent_info: Option<Implementation>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AuthMethod {
    #[serde(default)]
    pub name: String,
}

// ---------------------------------------------------------------------------
// session/new, session/prompt, session/cancel
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionNewParams {
    /// Absolute path of the project root.
    pub cwd: String,
    /// MCP servers the agent should connect to. Required by the spec; Dirigent
    /// does not forward any yet.
    pub mcp_servers: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionNewResult {
    pub session_id: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum PromptContent {
    Text { text: String },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionPromptParams {
    pub session_id: String,
    pub prompt: Vec<PromptContent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionPromptResult {
    pub stop_reason: StopReason,
}

/// Why the agent ended a prompt turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    #[serde(other)]
    Other,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionCancelParams {
    pub session_id: String,
}

// ---------------------------------------------------------------------------
// session/update notifications
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionNotification {
    pub session_id: String,
    pub update: SessionUpdate,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "sessionUpdate", rename_all = "snake_case")]
pub(super) enum SessionUpdate {
    AgentMessageChunk {
        content: ContentBlock,
    },
    ToolCall(ToolCall),
    ToolCallUpdate(ToolCall),
    Plan {
        entries: Vec<PlanEntry>,
    },
    /// `user_message_chunk`, `agent_thought_chunk`, `available_commands_update`,
    /// `current_mode_update`, and anything added after v1.
    #[serde(other)]
    Other,
}

/// Content block sent by the agent. Only text is rendered; images, audio and
/// resources are ignored.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ContentBlock {
    Text {
        text: String,
    },
    #[serde(other)]
    Other,
}

/// A `tool_call` (first report) or `tool_call_update` (later delta). Updates
/// only carry the fields that changed, hence everything but the id is optional.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ToolCall {
    pub tool_call_id: String,
    pub title: Option<String>,
    pub kind: Option<ToolKind>,
    pub status: Option<ToolCallStatus>,
    pub content: Option<Vec<ToolCallContent>>,
    pub locations: Option<Vec<ToolCallLocation>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ToolCallStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ToolCallContent {
    Content {
        content: ContentBlock,
    },
    /// Only the path is kept: the working tree is the source of truth for the
    /// run diff (see `resolve_run_diff` in `app/claude_run.rs`).
    Diff {
        path: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub(super) struct ToolCallLocation {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct PlanEntry {
    pub content: String,
    pub status: Option<PlanEntryStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PlanEntryStatus {
    Pending,
    InProgress,
    Completed,
    #[serde(other)]
    Other,
}

// ---------------------------------------------------------------------------
// Agent → client requests
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReadTextFileParams {
    pub path: String,
    /// 1-based first line to return.
    pub line: Option<u32>,
    /// Maximum number of lines to return.
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WriteTextFileParams {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RequestPermissionParams {
    pub tool_call: ToolCall,
    pub options: Vec<PermissionOption>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PermissionOption {
    pub option_id: String,
    pub kind: PermissionOptionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PermissionOptionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
    #[serde(other)]
    Other,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) enum AcpError {
    NotFound(String),
    SpawnFailed(std::io::Error),
    InvalidExtraArgs(String),
    /// The pre-run (or post-run) hook script failed.
    Hook(std::io::Error),
    Io(std::io::Error),
    /// The agent sent something we cannot work with.
    Protocol(String),
    /// The agent answered one of our requests with a JSON-RPC error.
    Agent {
        method: &'static str,
        code: i64,
        message: String,
    },
    /// `session/new` was rejected until the user authenticates the agent.
    AuthRequired(String),
    /// The agent closed stdout before the turn finished; carries a stderr tail.
    AgentExited(String),
    Refused,
    Cancelled,
}

impl std::fmt::Display for AcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcpError::NotFound(bin) => write!(f, "ACP agent not found: {bin}"),
            AcpError::SpawnFailed(e) => write!(f, "failed to spawn ACP agent: {e}"),
            AcpError::InvalidExtraArgs(args) => {
                write!(f, "failed to parse extra_args (unmatched quote?): {args}")
            }
            AcpError::Hook(e) => write!(f, "hook script failed: {e}"),
            AcpError::Io(e) => write!(f, "ACP I/O error: {e}"),
            AcpError::Protocol(msg) => write!(f, "ACP protocol error: {msg}"),
            AcpError::Agent {
                method,
                code,
                message,
            } => write!(f, "ACP agent rejected {method} ({code}): {message}"),
            AcpError::AuthRequired(methods) => write!(
                f,
                "ACP agent requires authentication ({methods}) — log in with the agent's own CLI first"
            ),
            AcpError::AgentExited(stderr) if stderr.is_empty() => {
                write!(f, "ACP agent exited before finishing the turn")
            }
            AcpError::AgentExited(stderr) => {
                write!(f, "ACP agent exited before finishing the turn: {stderr}")
            }
            AcpError::Refused => write!(f, "ACP agent refused the prompt"),
            AcpError::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for AcpError {}
