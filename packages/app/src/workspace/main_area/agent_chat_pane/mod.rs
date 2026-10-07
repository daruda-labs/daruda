//! Agent chat pane (native ACP) — module root.
//!
//! - [`view`] — the self-owned `AgentChatView` entity: chat model + UI state,
//!   per-event fold (`apply_event`), and render-listener ops; `cx.notify()`s
//!   itself so a scroll / fold dirties only its cached subtree.
//! - [`render`] — pure view of an `&AgentChatView` (MVU view purity); event
//!   closures one-line dispatch into view ops.
//!
//! The `Workspace` side — pane construction, the connection lifecycle, the
//! queue routing and the Telegram relay — is `super::agent_chat_host`. It
//! reaches the view only through `view::host_surface` / `host_commands`.

pub(in crate::workspace) mod agent_chat_helpers;
pub(super) mod autoscroll_ops;
pub(super) mod config_chip;
/// Which rows a pane has folded, and the mode those defaults come from.
pub(in crate::workspace) mod fold;
pub(super) mod mode_chip;
pub(in crate::workspace) mod output_editor;
/// A pane-local view preference plus whether the user or config set it.
pub(in crate::workspace) mod pane_choice;
/// The phone's side of one agent turn, owned by the view.
pub(in crate::workspace) mod phone_turn;
pub(super) mod reconcile;
pub(in crate::workspace) mod render;
pub(in crate::workspace) mod rows;
pub(super) mod session_config;
/// The fixed conversation the `--screenshot` agent-chat scenarios seed.
#[cfg(feature = "screenshot")]
pub(in crate::workspace) mod shot_transcript;
pub(super) mod slash_dispatch;
/// Parent/child structure of a conversation's tool calls — the one place the
/// nesting rules live.
pub(super) mod tool_hierarchy;
pub(super) mod tool_status;
/// The `[agent]` transcript defaults a pane follows until the user chooses.
pub(in crate::workspace) mod transcript_defaults;
pub(super) mod transcript_structure;
pub(in crate::workspace) mod view;
pub(super) mod window_access;
