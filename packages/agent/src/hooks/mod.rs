//! Hook channel — push events from Claude Code.
//!
//! - [`events`] — serde models for the 9 hook events daruda subscribes to
//! - [`fsm`] — `HookEvent` → [`crate::SessionStatus`] transitions
//! - [`status_file`] — `~/.daruda/status/*.json` read/write (atomic)
//! - [`cold_restore`] — startup-time TTL cleanup + stale → Connecting reset
//! - [`turn_end`] — which status-file updates end a turn, and how

pub mod cold_restore;
pub mod events;
pub mod fsm;
pub mod status_file;
pub mod turn_end;
