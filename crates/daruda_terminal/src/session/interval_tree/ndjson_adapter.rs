//! Adapter wiring [`daruda_store::marks::NdjsonFileSink`] to the
//! [`MarkRecordSink`] trait.
//!
//! The trait and the type live in separate crates to avoid a
//! dependency cycle (`daruda_terminal` → `daruda_store`, never the
//! reverse). The impl is placed here — in the crate that owns the
//! trait — to satisfy the orphan rule.

use std::io;

use daruda_store::marks::NdjsonFileSink;

use super::persistence::{MarkRecord, MarkRecordSink};

impl MarkRecordSink for NdjsonFileSink {
    fn append(&mut self, record: &MarkRecord) -> io::Result<()> {
        let line = serde_json::to_string(record).map_err(io::Error::other)?;
        self.append_line(&line).map(|_| ())
    }
}
