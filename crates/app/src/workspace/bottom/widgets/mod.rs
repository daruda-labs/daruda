//! Per-widget renderers for the bottom dock panel body.
//!
//! Today we only ship `button.rs` (click-to-send macro). Future widget
//! kinds (Text, Bar, Gauge, …) get their own sibling file and dispatch
//! through the `Widget` enum match in `bottom::render_body`.

pub(in crate::workspace) mod button;
