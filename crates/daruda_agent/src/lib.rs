//! Agent-provider integrations used by the GPUI app.
//!
//! This crate stays GPUI-free and owns provider-facing concerns that are shared
//! across the app: account recipes, usage/status fetches, local activity scans,
//! PTY links, and session-status state. Some modules are intentionally
//! provider-specific:
//!
//! - **`usage`** / **`accounts`** cover both Claude and Codex providers.
//! - **`activity`** scans Claude Code session logs; **`codex_activity`** scans
//!   Codex rollout logs.
//! - **`hooks`** — Claude Code's official hook system pushes JSON events.
//!   `daruda --hook <type>` parses stdin, runs the FSM, writes a status
//!   file under `~/.daruda/status/<session_id>.json`. Source of truth
//!   when installed.
//! - **`jsonl`** — Claude fallback for sessions without daruda hooks. Reads the
//!   tail of `~/.claude/projects/<encoded>/<session>.jsonl` and infers
//!   status from message structure. Ported from c9watch (MIT, see
//!   `LICENSE-THIRD-PARTY.md`).
//!
//! Claude hook/jsonl channels feed the same [`SessionStatus`] enum; app-side
//! aggregation merges them with hook-wins-on-tie semantics.

pub mod accounts;
pub mod activity;
mod activity_scan;
pub mod codex_activity;
pub mod hooks;
mod http;
pub mod jsonl;
pub mod providers;
pub mod pty_link;
pub mod service_status;
pub mod status;
pub mod store;
pub mod usage;

pub use accounts::PlanInfo;
pub use activity::{ActivityError, ActivityStats, DayActivity};
pub use http::FetchError;
pub use service_status::{ServiceStatus, StatusIndicator};
pub use status::SessionStatus;
pub use store::ClaudeStatusStore;
pub use usage::{LimitSeverity, ProviderUsage, UsageOutcome, UsageWindow, WindowScope, source_for};
