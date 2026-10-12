//! Agent chat pane (native ACP).
//!
//! - [`view`] — the self-owned `AgentChatView` entity: chat model + UI state,
//!   per-event fold (`apply_event`), and render-listener ops; `cx.notify()`s
//!   itself so a scroll / fold dirties only its cached subtree.
//! - [`render`] — pure view of an `&AgentChatView` (MVU view purity); event
//!   closures one-line dispatch into view ops.
//!
//! The host side — pane construction, the connection lifecycle, the queue
//! routing and the Telegram relay — lives in the app. It reaches the view only
//! through `view::host_surface` / `host_commands` and the `AgentChatEvent`s
//! the view emits.

rust_i18n::i18n!("locales", fallback = "en");

/// Pending prompts one pane may hold. The queue drains one per turn, so an
/// agent pushing faster than turns complete would grow it without bound.
pub const QUEUE_DEPTH_MAX: usize = 10;

pub mod agent_chat_helpers;
pub(crate) mod autoscroll_ops;
/// Which rows a pane has folded, and the mode those defaults come from.
pub mod fold;
pub mod labels;
mod localized;
pub mod output_editor;
/// A pane-local view preference plus whether the user or config set it.
pub mod pane_choice;
/// The phone's side of one agent turn, owned by the view.
pub mod phone_turn;
pub(crate) mod reconcile;
pub mod render;
pub mod rows;
pub mod session_config;
/// The fixed conversation the `--screenshot` agent-chat scenarios seed.
#[cfg(feature = "screenshot")]
pub mod shot_transcript;
pub mod slash_dispatch;
pub mod stamp;
mod strings;
#[cfg(test)]
mod test_support;
/// Parent/child structure of a conversation's tool calls — the one place the
/// nesting rules live.
pub(crate) mod tool_hierarchy;
pub(crate) mod tool_status;
/// The `[agent]` transcript defaults a pane follows until the user chooses.
pub mod transcript_defaults;
pub(crate) mod transcript_structure;
pub mod view;
pub(crate) mod window_access;
