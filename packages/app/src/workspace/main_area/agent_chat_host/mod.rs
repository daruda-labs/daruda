//! The `Workspace` side of the agent chat pane: everything that needs
//! workspace state to act on a chat. The pane itself is
//! `super::agent_chat_pane`, which this module drives only through the
//! view's host surface.
//!
//! - [`agent_chat_ops`] — pane/tab construction, desktop notifications,
//!   mode/config switching, and misc pane accessors.
//! - [`agent_chat_connect_ops`] — the ACP connection lifecycle: lazy-connect,
//!   manual retry, the background connect + event pump, and the `/clear` reset.
//! - [`agent_chat_event_ops`] — folding one ACP event into a pane.
//! - [`agent_chat_queue_ops`] — bottom-dock prompt send / queue / edit /
//!   cancel routing.
//! - [`host_event_ops`] — what the view asks of its host (`AgentChatEvent`).
//! - [`telegram_ops`] — Telegram relay: outbound pings and inbound
//!   phone-relayed replies / permission decisions routed back into a pane.

pub(super) mod agent_chat_connect_ops;
pub(in crate::workspace) mod agent_chat_event_ops;
pub(in crate::workspace) mod agent_chat_ops;
pub(super) mod agent_chat_queue_ops;
mod attachment_ops;
mod host_event_ops;
pub(in crate::workspace) mod telegram_ops;
