//! daruda as an MCP server.
//!
//! Everything about the protocol that needs no app state lives here; the one
//! method that does — `tools/call` — is intercepted by
//! `orchestrator::control`, which frames its results through
//! [`convert`] and asks [`protocol::Session`] the same admission question.
//! `daruda --mcp` is a byte relay: MCP's stdio framing is newline-delimited
//! JSON with no embedded newlines, which is the same framing the socket uses,
//! so the shim never has to understand a message to forward it.
//!
//! Only `initialize`, `notifications/initialized`, `notifications/cancelled`,
//! `tools/list`, `tools/call` and `ping` are implemented — a tools-only server
//! needs nothing else.

pub(crate) mod convert;
pub(crate) mod protocol;
pub(crate) mod shim;
pub(crate) mod socket;
pub(crate) mod tools;
