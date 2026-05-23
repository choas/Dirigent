use serde::{Deserialize, Serialize};

/// ACP protocol version supported by this client.
pub(crate) const PROTOCOL_VERSION: u32 = 1;

/// JSON-RPC 2.0 request envelope.
#[derive(Debug, Serialize)]
pub(super) struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: &str, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        }
    }
}

/// JSON-RPC 2.0 response envelope.
#[derive(Debug, Deserialize)]
pub(super) struct JsonRpcResponse {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub id: Option<u64>,
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonRpcError>,
    pub method: Option<String>,
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(super) struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

/// Capabilities the client (Dirigent) advertises to the agent.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ClientCapabilities {
    pub fs: FsCapabilities,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FsCapabilities {
    pub read_text_file: bool,
    pub write_text_file: bool,
}

/// Parameters for the `initialize` request.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InitializeParams {
    pub protocol_version: u32,
    pub client_info: ClientInfo,
    pub capabilities: ClientCapabilities,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ClientInfo {
    pub name: String,
    pub version: String,
}

/// Result from the `initialize` response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InitializeResult {
    #[allow(dead_code)]
    pub protocol_version: Option<u32>,
    pub agent_info: Option<AgentInfo>,
    pub capabilities: Option<AgentCapabilities>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentInfo {
    pub name: Option<String>,
    #[allow(dead_code)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentCapabilities {
    #[allow(dead_code)]
    pub load_session: Option<bool>,
}

/// Parameters for `session/new`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionNewParams {
    pub cwd: String,
}

/// Result from `session/new`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionNewResult {
    pub session_id: String,
}

/// Content block in a prompt.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(super) enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
}

/// Parameters for `session/prompt`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionPromptParams {
    pub session_id: String,
    pub content: Vec<ContentBlock>,
}

/// Stop reasons from `session/prompt` response.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub(super) enum StopReason {
    EndTurn,
    Cancelled,
    MaxTokens,
    #[serde(other)]
    Other,
}

/// Result from `session/prompt`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub(super) struct SessionPromptResult {
    pub stop_reason: Option<StopReason>,
}

/// A `session/update` notification payload.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionUpdateParams {
    #[allow(dead_code)]
    pub session_id: String,
    pub kind: SessionUpdateKind,
}

/// Kinds of session update notifications from the agent.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum SessionUpdateKind {
    #[serde(rename = "agent_message_chunk")]
    AgentMessageChunk { text: Option<String> },
    #[serde(rename = "tool_call")]
    ToolCall(ToolCallUpdate),
    #[serde(rename = "tool_call_update")]
    ToolCallUpdate(ToolCallUpdate),
    #[serde(rename = "plan")]
    Plan(PlanUpdate),
    #[serde(other)]
    Unknown,
}

/// Tool call information from the agent.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ToolCallUpdate {
    #[allow(dead_code)]
    pub tool_call_id: Option<String>,
    pub title: Option<String>,
    pub kind: Option<ToolCallKind>,
    pub status: Option<ToolCallStatus>,
    pub content: Option<Vec<ToolCallContent>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolCallKind {
    Read,
    Edit,
    Delete,
    Search,
    Execute,
    Think,
    Fetch,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolCallStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    #[serde(other)]
    Other,
}

/// Content within a tool call result.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ToolCallContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "diff")]
    Diff(DiffContent),
    #[serde(other)]
    Unknown,
}

/// A first-class diff content block from ACP.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiffContent {
    pub path: String,
    pub old_text: String,
    pub new_text: String,
}

/// Plan update from the agent.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PlanUpdate {
    pub entries: Option<Vec<PlanEntry>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlanEntry {
    pub content: Option<String>,
    #[allow(dead_code)]
    pub priority: Option<String>,
    pub status: Option<String>,
}

/// A request from the agent to the client (e.g. fs/read_text_file).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FsReadTextFileParams {
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FsWriteTextFileParams {
    pub path: String,
    pub content: String,
}

/// Parameters for `session/cancel`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionCancelParams {
    pub session_id: String,
}

/// Error type for ACP operations.
#[derive(Debug)]
pub(crate) enum AcpError {
    NotFound(String),
    SpawnFailed(std::io::Error),
    ProtocolError(String),
    Cancelled,
    IoError(std::io::Error),
}

impl std::fmt::Display for AcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcpError::NotFound(bin) => write!(f, "ACP agent binary not found: {bin}"),
            AcpError::SpawnFailed(e) => write!(f, "failed to spawn ACP agent: {e}"),
            AcpError::ProtocolError(msg) => write!(f, "ACP protocol error: {msg}"),
            AcpError::Cancelled => write!(f, "cancelled"),
            AcpError::IoError(e) => write!(f, "ACP I/O error: {e}"),
        }
    }
}

impl std::error::Error for AcpError {}

/// Response from a successful ACP session/prompt run.
#[derive(Debug, Clone)]
pub(crate) struct AcpResponse {
    pub text: String,
    pub diffs: Vec<DiffContent>,
    pub edited_files: Vec<String>,
    pub tool_calls_completed: u64,
}
