//! Hook channel — push events from Claude Code.
//!
//! - [`events`] — serde models for the 9 hook events daruda subscribes to
//! - [`fsm`] — `HookEvent` → [`crate::SessionStatus`] transitions
//! - [`status_file`] — `~/.daruda/status/*.json` read/write (atomic)
//! - [`cold_restore`] — startup-time TTL cleanup + stale → Connecting reset
//! - [`turn_end`] — which status-file updates end a turn, and how
//! - [`handler`] — the `daruda --hook <eventType>` subcommand body: stdin →
//!   FSM → status file
//! - [`installer`] — adding and removing those hook entries in
//!   `~/.claude/settings.json`, plus the `notify.sh` they run

pub mod cold_restore;
pub mod events;
pub mod fsm;
pub mod handler;
pub mod installer;
pub mod status_file;
pub mod turn_end;
