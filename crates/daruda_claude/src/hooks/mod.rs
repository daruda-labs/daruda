//! Hook channel — push events from Claude Code.
//!
//! - [`events`] — serde models for the 9 hook events daruda subscribes to
//! - [`fsm`] — `HookEvent` → [`crate::SessionStatus`] transitions
//! - [`status_file`] — `~/.daruda/status/*.json` read/write (atomic)
//! - [`cold_restore`] — startup-time TTL cleanup + stale → Connecting reset

pub mod cold_restore;
pub mod events;
pub mod fsm;
pub mod status_file;
