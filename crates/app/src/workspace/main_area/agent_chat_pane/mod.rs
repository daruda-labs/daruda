//! Agent chat pane (native ACP) — module root.
//!
//! Splits into two responsibilities:
//! - [`agent_chat_ops`] — `Workspace` ops: pane construction, the live
//!   ACP connection + event pump, and the prompt / cancel / permission
//!   handlers (GPUI-side state transitions).
//! - [`render`] — the pure view of an `&AgentChatContent`: the scrolling
//!   conversation, the prompt input + send/stop, and inline permission
//!   cards. Carries no state-transition logic (MVU view purity); event
//!   closures one-line dispatch into the ops above.

pub(in crate::workspace) mod agent_chat_ops;
pub(in crate::workspace) mod fold;
pub(in crate::workspace) mod render;

pub(in crate::workspace) use render::render;
