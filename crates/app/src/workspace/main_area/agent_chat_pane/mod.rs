//! Agent chat pane (native ACP) — module root.
//!
//! - [`view`] — the self-owned `AgentChatView` entity: chat model + UI state,
//!   per-event fold (`apply_event`), and render-listener ops; `cx.notify()`s
//!   itself so a scroll / fold dirties only its cached subtree.
//! - [`render`] — pure view of an `&AgentChatView` (MVU view purity); event
//!   closures one-line dispatch into view ops.
//! - [`agent_chat_ops`] — `Workspace` ops needing workspace state: pane/tab
//!   construction, desktop notifications, mode/config switching, and misc
//!   pane accessors.
//! - [`agent_chat_connect_ops`] — the ACP connection lifecycle: lazy-connect,
//!   manual retry, the background connect + event pump, and the `/clear` reset.
//! - [`agent_chat_queue_ops`] — bottom-dock prompt send / queue / edit / cancel
//!   routing.
//! - [`telegram_ops`] — Telegram relay: outbound pings and inbound
//!   phone-relayed replies / permission decisions routed back into a pane.

pub(in crate::workspace) mod agent_chat_connect_ops;
pub(in crate::workspace) mod agent_chat_helpers;
pub(in crate::workspace) mod agent_chat_ops;
pub(in crate::workspace) mod agent_chat_queue_ops;
pub(in crate::workspace) mod autoscroll_ops;
pub(in crate::workspace) mod config_chip;
/// Which conversation items a pane shows — the GPUI-free filter decision.
pub(in crate::workspace) mod display_filter;
pub(in crate::workspace) mod fold;
/// How much of a turn a pane opens by default — the GPUI-free mode matrix.
pub(in crate::workspace) mod fold_mode;
pub(in crate::workspace) mod mode_chip;
pub(in crate::workspace) mod output_editor;
/// A pane-local view preference plus whether the user or config set it.
pub(in crate::workspace) mod pane_choice;
pub(in crate::workspace) mod reconcile;
pub(in crate::workspace) mod render;
pub(in crate::workspace) mod rows;
pub(in crate::workspace) mod session_config;
/// The fixed conversation the `--screenshot` agent-chat scenarios seed.
#[cfg(feature = "screenshot")]
pub(in crate::workspace) mod shot_transcript;
pub(in crate::workspace) mod slash_dispatch;
pub(in crate::workspace) mod telegram_ops;
/// Parent/child structure of a conversation's tool calls — the one place the
/// nesting rules live.
pub(in crate::workspace) mod tool_hierarchy;
/// The `[agent]` transcript defaults a pane follows until the user chooses.
pub(in crate::workspace) mod transcript_defaults;
pub(in crate::workspace) mod view;
pub(in crate::workspace) mod window_access;
