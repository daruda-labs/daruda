//! Unified right-click menu for pane surfaces.
//!
//! Four surfaces (terminal body, agent-chat body, pane header, and — for the
//! shared Split section only — the tab bar) build one menu from one source.
//!
//! ```text
//! ops.rs      begin_pane_menu   snapshot + focus the target pane
//! context.rs  PaneMenuContext   plain data, no GPUI context
//! sections.rs compose           pure: context -> Vec<MenuEntry>
//! spec.rs     MenuEntry         pure menu description
//! adapter.rs  build_popup_menu  the only GPUI boundary
//! ```
//!
//! `begin_pane_menu` is the sole stateful entry point: it snapshots
//! selection / link / annotation state and *then* points the model's focused
//! pane at the right-clicked one, so `split_focused_pane_kind` and friends
//! act on the pane the user aimed at.

mod adapter;
mod context;
mod ops;
mod sections;
mod spec;

pub(in crate::workspace) use adapter::pane_context_menu;
