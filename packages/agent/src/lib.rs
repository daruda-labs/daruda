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
//! - **`hooks`** receives Claude Code and Codex lifecycle events and writes
//!   one profile-scoped status file per terminal ID.
//! - **`jsonl`** extracts the last assistant answer for CLI execution results.

pub mod accounts;
pub mod activity;
mod activity_scan;
pub mod codex_activity;
pub mod hooks;
mod http;
pub mod jsonl;
pub mod providers;
pub mod pty_link;
pub mod pty_tracker;
pub mod service_status;
pub mod skills;
pub mod status;
pub mod store;
pub mod usage;

pub use accounts::PlanInfo;
pub use activity::{ActivityError, ActivityStats, DayActivity};
pub use http::FetchError;
pub use service_status::{ServiceStatus, StatusIndicator};
pub use status::{AgentOutcome, SessionStatus};
pub use store::ClaudeStatusStore;
pub use usage::{LimitSeverity, ProviderUsage, UsageOutcome, UsageWindow, WindowScope, source_for};
