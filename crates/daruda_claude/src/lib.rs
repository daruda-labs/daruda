//! Claude Code session status detection.
//!
//! Two channels, same `SessionStatus` output:
//!
//! - **`hooks`** — Claude Code's official hook system pushes JSON events.
//!   `daruda --hook <type>` parses stdin, runs the FSM, writes a status
//!   file under `~/.daruda/status/<session_id>.json`. Source of truth
//!   when installed.
//! - **`jsonl`** — fallback for sessions without daruda hooks. Reads the
//!   tail of `~/.claude/projects/<encoded>/<session>.jsonl` and infers
//!   status from message structure. Ported from c9watch (MIT, see
//!   `LICENSE-THIRD-PARTY.md`).
//!
//! Both channels feed the same [`SessionStatus`] enum; the consumer
//! (`app/src/hooks/store.rs`) merges them with hook-wins-on-tie semantics.

pub mod hooks;
mod http;
pub mod jsonl;
pub mod limits;
pub mod pty_link;
pub mod service_status;
pub mod status;
pub mod store;
pub mod usage;

pub use http::FetchError;
pub use limits::{LimitSeverity, LimitWindow, PlanLimits};
pub use service_status::{ServiceStatus, StatusIndicator};
pub use status::SessionStatus;
pub use store::ClaudeStatusStore;
