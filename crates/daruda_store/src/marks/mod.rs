//! Persistence layer for interval-tree mark records.
//!
//! This module owns the raw file I/O side of mark persistence. The
//! trait implementation (`impl MarkRecordSink for NdjsonFileSink`)
//! lives in `daruda_terminal::session::interval_tree::ndjson_adapter`
//! to avoid a dependency cycle — `daruda_terminal` already depends on
//! `daruda_store`.
//!
//! # Primary types
//!
//! - [`NdjsonFileSink`] — append-only NDJSON writer for a single lane.
//! - [`replay_iter`] — open a marks file and return a line iterator
//!   suitable for passing to `daruda_terminal::persistence::replay`.

pub mod ndjson_sink;

pub use ndjson_sink::{NdjsonFileSink, marks_path, replay_iter};

#[cfg(test)]
mod tests;
