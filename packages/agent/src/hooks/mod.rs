//! Hook channel — push events from Claude Code and codex.
//!
//! - [`route`] — the environment a daruda terminal hands its agent hooks
//! - [`events`] — serde models for the hook events daruda subscribes to
//! - [`fsm`] — `HookEvent` → [`crate::SessionStatus`] transitions
//! - [`status_file`] — `<data dir>/status/*.json` read/write (atomic)
//! - [`agent_process`] — the agent process above a hook invocation
//! - [`cold_restore`] — startup-time TTL cleanup + stale → Connecting reset
//! - [`turn_end`] — which status-file updates end a turn, and how
//! - [`handler`] — the `daruda --hook <eventType>` subcommand body: stdin →
//!   FSM → status file
//! - [`installer`] — adding and removing daruda's hook entries in each
//!   agent's configuration

pub mod agent_process;
pub mod cold_restore;
pub mod events;
pub mod fsm;
pub mod handler;
pub mod history;
pub mod installer;
pub mod route;
pub mod status_file;
pub mod turn_end;
