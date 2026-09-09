//! External control surface: the command vocabulary daruda accepts from
//! outside the app, and what it answers with.
//!
//! Split three ways. `spec` and `result` are GPUI-free and pure — they are the
//! contract every adapter shares. `exec` is the app-level dispatcher that turns
//! a resolved command into a result, and is the only file here that touches
//! GPUI.
//!
//! The core is stateless. Which pane an ordinal names, and which pane is
//! currently selected, are conversation context owned by the adapter — see
//! `crate::telegram::command`.

pub(crate) mod agent_text;
pub(crate) mod approval;
pub(crate) mod ask;
pub(crate) mod exec;
pub(crate) mod guards;
pub(crate) mod mcp;
pub(crate) mod result;
pub(crate) mod spec;
