//! Observability — error reports, log writer, system info.
//!
//! GPUI-free. The three modules can be used from any crate that
//! depends on `daruda_store`, including the GPUI-free `pty.rs` and
//! background-thread code paths.
//!
//! # Modules
//! - [`error_report`] — `ErrorReport` data model + builder + plain-text
//!   and NDJSON serialization. The unit any error / panic / failed
//!   operation surfaces as.
//! - [`system_info`] — cached `daruda VERSION · OS · ARCH` summary +
//!   `redact_home()` helper. Strict allow-list — no hostname, username,
//!   or environment-variable leakage.
//! - [`log_writer`] — NDJSON append + 30-day rotation, gated by
//!   `cfg!(debug_assertions)` into `~/.daruda/logs/{debug,release}/`.

pub mod error_report;
pub mod log_writer;
pub mod system_info;
