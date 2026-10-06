//! NDJSON persistence layer for the interval tree.
//!
//! `MarkRecord` is the on-wire mutation event; an [`IntervalTree<MarkPayload>`]
//! can be configured with an [`MarkRecordSink`] so every payload-aware mutation
//! emits a record. File I/O is the responsibility of a later task — this
//! module ships the wire format and a [`replay`] function that rebuilds a
//! tree from a stream of NDJSON lines.
//!
//! ## Layering note
//!
//! `MarkRecord::payload` is typed as [`MarkPayload`], so the sink lives on
//! `impl IntervalTree<MarkPayload>` rather than on the generic core. The
//! generic [`IntervalTree<P>`] still holds the `Option<Box<dyn MarkRecordSink>>`
//! field but never emits on its own — see `tree.rs` and `lifecycle.rs`.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::IntervalTree;
use super::coord::LineRange;
use super::payload::MarkPayload;
use super::tree::MarkId;

/// On-wire schema version. Bump when [`MarkRecord`] gains an incompatible
/// change (renamed variant, removed field, changed semantics). Forward-
/// compatible additions (new optional fields) do not require a bump.
pub const RECORD_VERSION: u32 = 1;

/// One mutation event in the NDJSON log.
///
/// Serialized as an internally-tagged enum (`op` discriminator) with
/// `snake_case` operation names — so the on-disk form is one of:
///
/// ```text
/// {"op":"add","v":1,"ts":...,"seq":1,"id":7,"kind":"annotation","range":{...},"payload":{...}}
/// {"op":"update","v":1,"ts":...,"seq":2,"id":7,"range":null,"payload":{...}}
/// {"op":"remove","v":1,"ts":...,"seq":3,"id":7}
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MarkRecord {
    Add {
        v: u32,
        ts: SystemTime,
        seq: u64,
        id: MarkId,
        /// Stable kind discriminator (`MarkPayload::kind_tag()`); duplicated
        /// next to the tagged payload to make log-scanning tools cheap.
        kind: String,
        range: LineRange,
        payload: MarkPayload,
    },
    Update {
        v: u32,
        ts: SystemTime,
        seq: u64,
        id: MarkId,
        /// Present only when the range changed (i.e. emitted by
        /// `update_payload_range`). `None` for payload-only updates.
        range: Option<LineRange>,
        payload: MarkPayload,
    },
    Remove {
        v: u32,
        ts: SystemTime,
        seq: u64,
        id: MarkId,
    },
}

impl MarkRecord {
    /// Wire-schema version of this record.
    pub fn version(&self) -> u32 {
        match self {
            MarkRecord::Add { v, .. }
            | MarkRecord::Update { v, .. }
            | MarkRecord::Remove { v, .. } => *v,
        }
    }

    /// Monotonic sequence number assigned at emit time.
    pub fn seq(&self) -> u64 {
        match self {
            MarkRecord::Add { seq, .. }
            | MarkRecord::Update { seq, .. }
            | MarkRecord::Remove { seq, .. } => *seq,
        }
    }

    /// Identifier of the affected mark.
    pub fn id(&self) -> MarkId {
        match self {
            MarkRecord::Add { id, .. }
            | MarkRecord::Update { id, .. }
            | MarkRecord::Remove { id, .. } => *id,
        }
    }
}

/// Sink for outgoing mutation records.
///
/// Implementations may write to NDJSON files (later task), accumulate in
/// memory for tests, or no-op for ephemeral trees. The tree owns an
/// `Option<Box<dyn MarkRecordSink>>`; `None` means "do not log".
///
/// Errors from [`append`] are logged via `tracing::warn!` by the tree
/// wrapper methods but do NOT propagate — a failing sink must never corrupt
/// in-memory tree state.
pub trait MarkRecordSink: Send {
    fn append(&mut self, record: &MarkRecord) -> std::io::Result<()>;
}

/// Test-only sink that just stores everything it sees.
#[cfg(test)]
#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct VecSink(pub Vec<MarkRecord>);

#[cfg(test)]
impl MarkRecordSink for VecSink {
    fn append(&mut self, record: &MarkRecord) -> std::io::Result<()> {
        self.0.push(record.clone());
        Ok(())
    }
}

/// Per-replay accounting. Useful for diagnostics and for surfacing partial
/// writes (crash mid-flush) at the boundary between "data we trust" and
/// "data we threw away".
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReplayStats {
    /// Records that parsed successfully, carried the expected version, and
    /// were applied to the tree.
    pub applied: usize,
    /// The last line in the iterator failed to parse — almost always a
    /// crash mid-write. At most 1.
    pub skipped_partial: usize,
    /// Parsed cleanly but `v` did not match [`RECORD_VERSION`].
    pub skipped_version: usize,
    /// Parsed neither as the current schema nor as a wrong-version record.
    /// Forward-compatible junk from a future schema falls here.
    pub skipped_unknown: usize,
}

/// Reconstruct an [`IntervalTree<MarkPayload>`] from a stream of NDJSON
/// lines. The stream yields `io::Result<String>` to mirror the natural
/// iterator type over `BufRead::lines()`.
///
/// Apply rules:
/// - Each `Ok(line)` is parsed as a [`MarkRecord`]. Successful parses with
///   `v == RECORD_VERSION` are applied in order: `Add` preserves the
///   recorded id, `Update` calls the matching tree method, `Remove` deletes.
/// - A trailing parse failure (last line in the iterator) is treated as a
///   write-in-progress and counted as `skipped_partial`.
/// - An intermediate parse failure is counted as `skipped_unknown` and the
///   replay continues with the next line.
/// - Wrong-version records are counted as `skipped_version`.
/// - I/O errors from the stream (`Err(_)` items) abort the replay
///   immediately and the tree is returned in its partial state. This is
///   intentional: an I/O error mid-stream is not the same as garbled JSON,
///   so callers can decide whether to surface or retry.
///
/// After replay:
/// - `next_id` is bumped past every id seen in any `Add` record so subsequent
///   native inserts cannot collide with replayed ids.
/// - The returned tree has no sink configured. Callers wire one up (or not)
///   after replay completes.
///
/// Replay processes records strictly in the order they appear; the `seq`
/// field is recorded but not enforced for ordering since NDJSON line order
/// is the authoritative source.
///
/// Note: this function collects the entire iterator into memory before
/// processing in order to classify the last non-empty line as a partial
/// write. Log files should be bounded before calling (see T4 size cap).
pub fn replay<I>(lines: I) -> (IntervalTree<MarkPayload>, ReplayStats)
where
    I: IntoIterator<Item = std::io::Result<String>>,
{
    let mut tree: IntervalTree<MarkPayload> = IntervalTree::new();
    let mut stats = ReplayStats::default();

    // Collect into Vec so we can distinguish "last line failed" (partial
    // write) from "intermediate line failed" (forward-compatible junk).
    // Logs are bounded by file size, and the entire file lives in memory
    // during read anyway via String allocation per line — Vec is fine.
    let collected: Vec<std::io::Result<String>> = lines.into_iter().collect();
    // The "last non-empty Ok line" — not the absolute last index — is what
    // distinguishes a partial write from forward-compat junk. Trailing
    // empty lines after a truncated record would otherwise misclassify it.
    let last_non_empty = collected
        .iter()
        .rposition(|item| matches!(item, Ok(s) if !s.trim().is_empty()));
    let mut max_add_id: u64 = 0;

    for (idx, item) in collected.into_iter().enumerate() {
        let line = match item {
            Ok(s) => s,
            // I/O error from the underlying stream: abort with stats so far.
            Err(_) => return (tree, stats),
        };
        // Skip empty/whitespace-only lines silently — NDJSON tooling often
        // emits or accepts trailing newlines.
        if line.trim().is_empty() {
            continue;
        }
        let parse_attempt = serde_json::from_str::<MarkRecord>(&line);
        match parse_attempt {
            Ok(record) if record.version() == RECORD_VERSION => {
                apply_record(&mut tree, record, &mut max_add_id);
                stats.applied += 1;
            }
            Ok(_) => {
                stats.skipped_version += 1;
            }
            Err(_) => {
                let is_last = Some(idx) == last_non_empty;
                if is_last {
                    stats.skipped_partial += 1;
                } else {
                    stats.skipped_unknown += 1;
                }
            }
        }
    }

    tree.set_next_id_at_least(max_add_id + 1);
    (tree, stats)
}

/// Apply a single validated record. Tracks the maximum `MarkId` seen in any
/// `Add` so the caller can bump `next_id` after the loop.
fn apply_record(tree: &mut IntervalTree<MarkPayload>, record: MarkRecord, max_add_id: &mut u64) {
    match record {
        MarkRecord::Add {
            id, range, payload, ..
        } => {
            // Defensive: if a malformed log somehow contains a duplicate Add
            // for the same id, prefer the existing entry over panicking.
            if tree.contains(id) {
                return;
            }
            if id.0 > *max_add_id {
                *max_add_id = id.0;
            }
            tree.insert_with_id(id, range, payload);
        }
        MarkRecord::Update {
            id, range, payload, ..
        } => {
            if !tree.contains(id) {
                // Update for an id we never saw add — skip silently. A
                // future schema may legitimately drop the Add prefix when
                // log compaction lands.
                return;
            }
            if let Some(new_range) = range {
                tree.update_payload_range(id, new_range);
                // The wrapper does not let us set a payload+range in one
                // call, so also overwrite the payload.
                tree.update_payload(id, |p| *p = payload);
            } else {
                tree.update_payload(id, |p| *p = payload);
            }
        }
        MarkRecord::Remove { id, .. } => {
            tree.remove_payload(id);
        }
    }
}
