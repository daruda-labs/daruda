//! Agent chat pane (native ACP) — module root.
//!
//! Splits into three responsibilities:
//! - [`view`] — the self-owned `AgentChatView` entity: the chat model + UI
//!   state, the per-event fold (`apply_event`), and every render-listener op
//!   (`toggle_fold`, `on_scroll`, `respond_permission`, `set_mode`, …). It
//!   `cx.notify()`s itself, so a scroll / fold dirties only its cached subtree.
//! - [`render`] — the pure view of an `&AgentChatView`: the scrolling
//!   conversation and inline permission cards. Carries no state-transition
//!   logic (MVU view purity); event closures one-line dispatch into view ops.
//! - [`agent_chat_ops`] — `Workspace` ops that need workspace state: pane/tab
//!   construction, the live ACP connection + event pump (reads `syntax_theme`,
//!   owns `report_error`), the desktop-notification pipeline, and the
//!   bottom-dock prompt / cancel routing. Plus the GPUI-free helpers the
//!   view's reconcilers reuse.
//! - [`telegram_ops`] — the Telegram relay domain: outbound pings to the
//!   bridge and inbound phone-relayed replies/permission decisions routed
//!   back into a pane. Sibling of `agent_chat_ops`, which still owns the tee
//!   points (`maybe_notify_agent_event` / `fire_activity_completion`) that
//!   call into this file's `relay_*` methods.

pub(in crate::workspace) mod agent_chat_helpers;
pub(in crate::workspace) mod agent_chat_ops;
pub(in crate::workspace) mod autoscroll_ops;
pub(in crate::workspace) mod config_chip;
pub(in crate::workspace) mod fold;
pub(in crate::workspace) mod mode_chip;
pub(in crate::workspace) mod reconcile;
pub(in crate::workspace) mod render;
pub(in crate::workspace) mod rows;
pub(in crate::workspace) mod slash_dispatch;
pub(in crate::workspace) mod telegram_ops;
pub(in crate::workspace) mod view;
