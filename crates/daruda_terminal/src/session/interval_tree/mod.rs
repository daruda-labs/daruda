//! Augmented red-black interval tree used to track marks (highlights,
//! prompt regions, etc.) across the unified line-buffer / viewport frame.
//!
//! This module is the data-structure layer only — payload semantics,
//! lifecycle (rebinding viewport coords to scrollback positions), and
//! persistence are layered on top in later tasks.

mod coord;
mod lifecycle;
mod ndjson_adapter;
mod payload;
mod persistence;
mod query;
mod tree;

#[cfg(test)]
mod tests;

pub use coord::{LineCoord, LineRange};
pub use payload::{AnnotationPayload, MarkPayload};
pub use persistence::{MarkRecord, MarkRecordSink, RECORD_VERSION, ReplayStats, replay};
pub use query::MarkRef;
pub use tree::{IntervalTree, MarkId};
