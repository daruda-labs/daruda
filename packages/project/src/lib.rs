//! Runtime project and lane model: a [`project::Project`] holding its
//! [`lane::Lane`]s, the git/worktree operations behind them, and where a
//! lane's agent session attaches.
//!
//! GPUI-free and app-free. Persisted shapes stay in `daruda_store`; the app
//! keeps GPUI, scheduling, and localized messages, re-exporting these modules.

pub mod lane;
pub mod project;
