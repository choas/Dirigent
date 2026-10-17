//! Agent Client Protocol (ACP) client: Dirigent drives any ACP-speaking agent
//! (claude-code-acp, Gemini CLI, Goose, …) as a CLI provider.
//!
//! The protocol is implemented directly on top of `serde_json` rather than the
//! `agent-client-protocol` crate — see `docs/acp.md` for the rationale and for
//! the (not implemented) reverse direction, Dirigent as an ACP agent.

mod client;
mod invoke;
mod types;

pub(crate) use invoke::{invoke_acp_streaming, AcpRunConfig};
