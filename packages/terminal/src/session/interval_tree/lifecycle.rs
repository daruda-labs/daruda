//! Lifecycle hooks specific to `IntervalTree<MarkPayload>`.
//!
//! Scrollback eviction, alt-screen visibility, and column clamping are
//! payload-aware operations that the generic `IntervalTree<P>` cannot
//! express on its own. Keeping them in a dedicated module preserves the
//! core data structure's payload-agnostic contract.
//!
//! ## Sink wiring (NDJSON persistence)
//!
//! The `*_payload` wrapper methods on this impl perform the underlying
//! generic mutation and then, if a sink is configured, build and append a
//! [`MarkRecord`]. Generic `IntervalTree<P>::insert / update / remove`
//! deliberately do NOT touch the sink — this keeps the data-structure
//! layer payload-agnostic. See `persistence.rs` for the layering rationale.
//!
//! Sink errors are logged via `tracing::warn!` and swallowed: a failing
//! sink must never corrupt in-memory tree state.

use std::time::SystemTime;

use super::coord::{LineCoord, LineRange};
use super::payload::MarkPayload;
use super::persistence::{MarkRecord, RECORD_VERSION};
use super::query::MarkRef;
use super::tree::{IntervalTree, MarkId};

impl IntervalTree<MarkPayload> {
    /// Remove every mark whose `range.end` is strictly less than `threshold`
    /// (i.e. the mark has slid off the bottom of the live scrollback window).
    /// Returns the IDs of removed marks in iteration order so callers can
    /// emit ordered NDJSON `Remove` records.
    ///
    /// When a sink is configured, this method also emits a `Remove` record
    /// per evicted id in the same iteration order.
    pub fn evict_below(&mut self, threshold: LineCoord) -> Vec<MarkId> {
        let to_remove: Vec<MarkId> = self
            .iter()
            .filter(|m| m.range.end < threshold)
            .map(|m| m.id)
            .collect();
        for id in &to_remove {
            let removed = self.remove(*id);
            debug_assert!(
                removed.is_some(),
                "iterated id should still exist at eviction"
            );
            self.emit_remove(*id);
        }
        to_remove
    }

    /// Toggle alt-screen visibility filtering. See `iter_visible`.
    ///
    /// This mutates view-only state and produces NO NDJSON records — the
    /// flag is reconstructed per session, not persisted.
    pub fn set_alt_screen_active(&mut self, active: bool) {
        self.alt_screen_active = active;
    }

    /// Whether `iter_visible` is currently filtering by payload visibility.
    pub fn alt_screen_active(&self) -> bool {
        self.alt_screen_active
    }

    /// Clamp every payload's column metadata to `max_cols`. Returns the IDs
    /// of marks that actually mutated, in iteration order.
    ///
    /// When a sink is configured, each mutated mark also emits an `Update`
    /// record with `range: None` and the clamped payload.
    pub fn clamp_payload_cols(&mut self, max_cols: u16) -> Vec<MarkId> {
        let ids: Vec<MarkId> = self.iter().map(|m| m.id).collect();
        let mut changed = Vec::new();
        for id in ids {
            let mut did_change = false;
            self.update(id, |p| {
                did_change = p.clamp_cols(max_cols);
            });
            if did_change {
                changed.push(id);
                self.emit_update(id, None);
            }
        }
        changed
    }

    /// In-order iterator that respects alt-screen visibility: when
    /// `alt_screen_active` is true, yields only marks whose payload reports
    /// `is_visible_in_alt_screen() == true`. When false, yields all marks.
    pub fn iter_visible(&self) -> impl Iterator<Item = MarkRef<'_, MarkPayload>> {
        let active = self.alt_screen_active;
        self.iter()
            .filter(move |m| !active || m.payload.is_visible_in_alt_screen())
    }

    // ----- Sink-aware wrapper methods -----

    /// Insert a payload and, if a sink is configured, emit an `Add` record.
    pub fn insert_payload(&mut self, range: LineRange, payload: MarkPayload) -> MarkId {
        let id = self.insert(range, payload.clone());
        self.emit_add(id, range, payload);
        id
    }

    /// Mutate a payload in place via `f` and, on success with a sink
    /// configured, emit an `Update` record carrying the post-mutation
    /// payload and `range: None`.
    pub fn update_payload<F>(&mut self, id: MarkId, f: F) -> bool
    where
        F: FnOnce(&mut MarkPayload),
    {
        let ok = self.update(id, f);
        if ok {
            self.emit_update(id, None);
        }
        ok
    }

    /// Change a mark's range and, on success with a sink configured, emit
    /// an `Update` record carrying the new range and the current payload.
    pub fn update_payload_range(&mut self, id: MarkId, new_range: LineRange) -> bool {
        let ok = self.update_range(id, new_range);
        if ok {
            self.emit_update(id, Some(new_range));
        }
        ok
    }

    /// Remove a mark and, on success with a sink configured, emit a
    /// `Remove` record.
    pub fn remove_payload(&mut self, id: MarkId) -> Option<MarkPayload> {
        let removed = self.remove(id);
        if removed.is_some() {
            self.emit_remove(id);
        }
        removed
    }

    // ----- Record builders -----

    fn next_seq(&mut self) -> u64 {
        self.seq = self.seq.saturating_add(1);
        self.seq
    }

    fn emit_add(&mut self, id: MarkId, range: LineRange, payload: MarkPayload) {
        if self.sink.is_none() {
            return;
        }
        let seq = self.next_seq();
        let record = MarkRecord::Add {
            v: RECORD_VERSION,
            ts: SystemTime::now(),
            seq,
            id,
            kind: payload.kind_tag().to_string(),
            range,
            payload,
        };
        self.append_to_sink(&record);
    }

    fn emit_update(&mut self, id: MarkId, range: Option<LineRange>) {
        if self.sink.is_none() {
            return;
        }
        // Read the current payload so the emitted record reflects post-mutation
        // state. Cloning is acceptable: emit frequency is bounded by user
        // mutation rate, not redraw rate.
        let payload = match self.iter().find(|m| m.id == id) {
            Some(m) => m.payload.clone(),
            None => return,
        };
        let seq = self.next_seq();
        let record = MarkRecord::Update {
            v: RECORD_VERSION,
            ts: SystemTime::now(),
            seq,
            id,
            range,
            payload,
        };
        self.append_to_sink(&record);
    }

    fn emit_remove(&mut self, id: MarkId) {
        if self.sink.is_none() {
            return;
        }
        let seq = self.next_seq();
        let record = MarkRecord::Remove {
            v: RECORD_VERSION,
            ts: SystemTime::now(),
            seq,
            id,
        };
        self.append_to_sink(&record);
    }

    /// Dispatch `record` to the configured sink. Errors are logged and
    /// swallowed — see module docs.
    fn append_to_sink(&mut self, record: &MarkRecord) {
        if let Some(sink) = self.sink.as_mut()
            && let Err(err) = sink.append(record)
        {
            tracing::warn!(
                target: "daruda_terminal::interval_tree",
                "mark record sink append failed: {err}"
            );
        }
    }
}
