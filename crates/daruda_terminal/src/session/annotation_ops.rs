//! Public annotation API on top of the interval tree.
//!
//! Wraps the payload-aware methods on `IntervalTree<MarkPayload>` with
//! `TerminalSession`-level validation: a single-line range constraint
//! (SP-1 dialog-side enforcement is the first layer, this is the second)
//! and a typed error enum that surfaces the few cases the caller actually
//! needs to distinguish from "succeeded".
//!
//! No persistence handling lives here — the tree's sink machinery does
//! its own `tracing::warn!` logging when an `append` fails, and never
//! propagates the error to the mutation method's caller. The
//! [`AnnotationError::PersistenceFailed`] variant is reserved for a
//! future revision where sink errors do surface (Task 6 hookup).

use std::fmt;
use std::io;

use super::TerminalSession;
use super::interval_tree::{AnnotationPayload, LineCoord, LineRange, MarkId, MarkPayload, MarkRef};

/// Failure modes surfaced from the annotation API.
///
/// The `PersistenceFailed` variant is reserved for a future change where
/// sink errors propagate out of the tree (today they are logged via
/// `tracing::warn!` inside `interval_tree::lifecycle`). It is *not*
/// constructed by any code path in this module today; keeping the variant
/// avoids a downstream API break when sink-error reporting lands.
#[derive(Debug)]
pub enum AnnotationError {
    /// The supplied range was outside the live coordinate space, or
    /// violates the SP-1 single-line constraint (`range.start != range.end`).
    CoordOutOfRange,
    /// No annotation with the given id exists.
    NotFound,
    /// The persistence sink returned an io error. Reserved for future
    /// use — not constructed today.
    #[allow(dead_code)] // reserved for Task 6 / SP-2 when sink errors propagate
    PersistenceFailed(io::Error),
}

impl fmt::Display for AnnotationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnnotationError::CoordOutOfRange => {
                f.write_str("annotation range is outside the live coordinate space")
            }
            AnnotationError::NotFound => f.write_str("annotation not found"),
            AnnotationError::PersistenceFailed(err) => {
                write!(f, "annotation persistence failed: {err}")
            }
        }
    }
}

impl std::error::Error for AnnotationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AnnotationError::PersistenceFailed(err) => Some(err),
            _ => None,
        }
    }
}

impl TerminalSession {
    /// Create a new annotation covering `range` with the supplied `text`.
    ///
    /// SP-1 only supports single-line ranges (`range.start == range.end`).
    /// Multi-line input is rejected with [`AnnotationError::CoordOutOfRange`].
    /// Newlines inside `text` are preserved — the constraint is only on the
    /// y-axis coordinates of `range`.
    pub fn add_annotation(
        &mut self,
        range: LineRange,
        text: String,
    ) -> Result<MarkId, AnnotationError> {
        if range.start != range.end {
            return Err(AnnotationError::CoordOutOfRange);
        }
        let payload = MarkPayload::Annotation(AnnotationPayload::new(text));
        Ok(self.interval_tree.insert_payload(range, payload))
    }

    /// Replace the text of an existing annotation. Returns
    /// [`AnnotationError::NotFound`] if `id` does not refer to a current
    /// annotation, or if the payload is no longer an `Annotation` variant
    /// (future-proofing — only `Annotation` exists in SP-1).
    pub fn update_annotation_text(
        &mut self,
        id: MarkId,
        text: String,
    ) -> Result<(), AnnotationError> {
        let ok = self.interval_tree.update_payload(id, |p| {
            // Single-variant today; the `if let` future-proofs against
            // additional `MarkPayload` variants in later SPs.
            #[allow(irrefutable_let_patterns)]
            if let MarkPayload::Annotation(a) = p {
                a.text = text;
                a.touch_updated();
            }
        });
        if !ok {
            return Err(AnnotationError::NotFound);
        }
        Ok(())
    }

    /// Delete the annotation with `id`. Returns [`AnnotationError::NotFound`]
    /// if no such annotation exists.
    pub fn remove_annotation(&mut self, id: MarkId) -> Result<(), AnnotationError> {
        match self.interval_tree.remove_payload(id) {
            Some(_) => Ok(()),
            None => Err(AnnotationError::NotFound),
        }
    }

    /// All annotations overlapping `range`, filtered by alt-screen
    /// visibility. Marks whose payload variant is not `Annotation` are
    /// skipped (future variants will gain their own accessors).
    pub fn annotations_in_range(
        &self,
        range: LineRange,
    ) -> Vec<(MarkId, &AnnotationPayload, LineRange)> {
        let alt_active = self.interval_tree.alt_screen_active();
        self.interval_tree
            .overlap(range)
            .filter_map(|m: MarkRef<'_, MarkPayload>| match m.payload {
                MarkPayload::Annotation(a) => {
                    if alt_active && a.hidden_in_alt_screen {
                        None
                    } else {
                        Some((m.id, a, m.range))
                    }
                }
            })
            .collect()
    }

    /// Look up an annotation by id. Returns `None` when no mark exists
    /// with that id, when the payload is not the `Annotation` variant,
    /// or when the mark is hidden by the alt-screen filter.
    ///
    /// O(1) lookup via the tree's `by_id` index — used by the edit
    /// dialog so the lookup works even when the annotation is scrolled
    /// out of the visible viewport.
    pub fn annotation_by_id(&self, id: MarkId) -> Option<&AnnotationPayload> {
        let alt_active = self.interval_tree.alt_screen_active();
        #[allow(irrefutable_let_patterns)]
        let MarkPayload::Annotation(a) = self.interval_tree.get_payload(id)? else {
            return None;
        };
        if alt_active && a.hidden_in_alt_screen {
            return None;
        }
        Some(a)
    }

    /// First annotation whose range contains `line` and whose column span
    /// (when present) contains `col`. Returns `None` when no annotation
    /// matches or the matching mark is hidden by the alt-screen filter.
    pub fn annotation_at_point(
        &self,
        line: LineCoord,
        col: u16,
    ) -> Option<(MarkId, &AnnotationPayload)> {
        let alt_active = self.interval_tree.alt_screen_active();
        for m in self.interval_tree.at_line(line) {
            // Single-variant today; the `let-else` future-proofs against
            // additional `MarkPayload` variants in later SPs.
            #[allow(irrefutable_let_patterns)]
            let MarkPayload::Annotation(a) = m.payload else {
                continue;
            };
            if alt_active && a.hidden_in_alt_screen {
                continue;
            }
            let col_match = match (a.start_col, a.end_col) {
                (Some(s), Some(e)) => col >= s && col <= e,
                // `None`-bounded spans mean "whole line": always match.
                _ => true,
            };
            if col_match {
                return Some((m.id, a));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::TerminalConfig;
    use crate::session::interval_tree::{MarkRecord, MarkRecordSink};
    use crate::session::line_buffer::LineBufferPosition;

    fn buf(i: u64) -> LineCoord {
        LineCoord::Buffered(LineBufferPosition { abs_index: i })
    }

    fn single_line_range(i: u64) -> LineRange {
        LineRange::new(buf(i), buf(i))
    }

    /// Test sink that records every append into a shared vector so the
    /// test can read it back. Never errors.
    struct RecordingSink {
        records: Arc<Mutex<Vec<MarkRecord>>>,
    }

    impl MarkRecordSink for RecordingSink {
        fn append(&mut self, record: &MarkRecord) -> std::io::Result<()> {
            self.records.lock().unwrap().push(record.clone());
            Ok(())
        }
    }

    #[test]
    fn add_then_lookup_round_trip() {
        let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
        let id = session
            .add_annotation(single_line_range(7), "hello".into())
            .expect("add succeeds");
        let hit = session
            .annotation_at_point(buf(7), 0)
            .expect("point lookup hits");
        assert_eq!(hit.0, id);
        assert_eq!(hit.1.text, "hello");
    }

    #[test]
    fn annotation_by_id_returns_payload_for_known_mark() {
        let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
        let id = session
            .add_annotation(single_line_range(0), "note".into())
            .expect("add");
        let payload = session
            .annotation_by_id(id)
            .expect("known id resolves to payload");
        assert_eq!(payload.text, "note");
        // Unknown id should miss.
        let missing = MarkId(u64::MAX);
        assert!(session.annotation_by_id(missing).is_none());
    }

    #[test]
    fn annotation_by_id_filters_alt_screen() {
        let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
        let id = session
            .add_annotation(single_line_range(0), "primary".into())
            .expect("add");
        // Default: hidden_in_alt_screen = true → invisible on alt screen.
        session.interval_tree.set_alt_screen_active(true);
        assert!(session.annotation_by_id(id).is_none());
        session.interval_tree.set_alt_screen_active(false);
        assert!(session.annotation_by_id(id).is_some());
    }

    #[test]
    fn rejects_multi_line_range() {
        let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
        let multi = LineRange::new(buf(3), buf(5));
        let err = session
            .add_annotation(multi, "spans".into())
            .expect_err("multi-line is rejected");
        assert!(matches!(err, AnnotationError::CoordOutOfRange));
    }

    #[test]
    fn evicted_annotation_drops_from_query() {
        let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
        // Mark sits at abs_index=2; we will evict everything <= 4.
        let id = session
            .add_annotation(single_line_range(2), "old".into())
            .expect("add");
        let visible_before = session.annotations_in_range(LineRange::new(buf(0), buf(100)));
        assert_eq!(visible_before.len(), 1);
        assert_eq!(visible_before[0].0, id);

        // Direct call — synthesising enough rows to push LineBuffer past
        // abs_index=2 is >50 lines of setup. Threshold semantics are
        // covered by the interval_tree's own tests; here we just verify
        // the session-level query reflects the eviction.
        session.interval_tree.evict_below(buf(5));
        let visible_after = session.annotations_in_range(LineRange::new(buf(0), buf(100)));
        assert!(
            visible_after.is_empty(),
            "evicted annotation must not surface"
        );
    }

    #[test]
    fn resize_clamps_payload_cols() {
        let cfg = TerminalConfig {
            cols: 120,
            ..TerminalConfig::default()
        };
        let mut session = TerminalSession::new(cfg).unwrap();
        // Build a payload with a wide column span, then resize down.
        let id = session
            .add_annotation(single_line_range(0), "wide".into())
            .expect("add");
        session.interval_tree.update_payload(id, |p| {
            #[allow(irrefutable_let_patterns)]
            if let MarkPayload::Annotation(a) = p {
                a.start_col = Some(100);
                a.end_col = Some(118);
            }
        });
        session.resize(80, session.config.rows).expect("resize");
        let hits = session.annotations_in_range(LineRange::new(buf(0), buf(0)));
        assert_eq!(hits.len(), 1);
        let (_, payload, _) = hits[0];
        assert_eq!(payload.start_col, Some(80));
        assert_eq!(payload.end_col, Some(80));
    }

    #[test]
    fn alt_screen_filters_hidden_annotations() {
        let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
        // First mark: visible everywhere.
        let visible_id = session
            .add_annotation(single_line_range(10), "always".into())
            .expect("add visible");
        session.interval_tree.update_payload(visible_id, |p| {
            #[allow(irrefutable_let_patterns)]
            if let MarkPayload::Annotation(a) = p {
                a.hidden_in_alt_screen = false;
            }
        });
        // Second mark keeps the default `hidden_in_alt_screen = true`.
        let hidden_id = session
            .add_annotation(single_line_range(11), "primary-only".into())
            .expect("add hidden");

        // Primary screen: both visible.
        session.interval_tree.set_alt_screen_active(false);
        let primary = session.annotations_in_range(LineRange::new(buf(0), buf(100)));
        let mut ids: Vec<_> = primary.iter().map(|(id, _, _)| *id).collect();
        ids.sort_by_key(|MarkId(n)| *n);
        assert_eq!(ids, vec![visible_id, hidden_id]);

        // Alt screen: only the visible-tagged one survives.
        session.interval_tree.set_alt_screen_active(true);
        let in_alt = session.annotations_in_range(LineRange::new(buf(0), buf(100)));
        assert_eq!(in_alt.len(), 1);
        assert_eq!(in_alt[0].0, visible_id);

        // Point lookup obeys the same filter.
        assert!(session.annotation_at_point(buf(11), 0).is_none());
        // Toggle back: hidden mark resurfaces.
        session.interval_tree.set_alt_screen_active(false);
        assert!(session.annotation_at_point(buf(11), 0).is_some());
    }

    #[test]
    fn add_and_remove_without_sink() {
        let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
        // No sink attached; default state.
        let id = session
            .add_annotation(single_line_range(0), "no-sink".into())
            .expect("add");
        assert_eq!(session.interval_tree.len(), 1);
        session.remove_annotation(id).expect("remove");
        assert!(session.interval_tree.is_empty());
    }

    #[test]
    fn sink_setter_routes_records() {
        let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
        let records: Arc<Mutex<Vec<MarkRecord>>> = Arc::new(Mutex::new(Vec::new()));
        session.set_marks_sink(Box::new(RecordingSink {
            records: Arc::clone(&records),
        }));
        let id = session
            .add_annotation(single_line_range(0), "log".into())
            .expect("add");
        session.remove_annotation(id).expect("remove");
        let captured = records.lock().unwrap();
        assert_eq!(captured.len(), 2, "add + remove emit one record each");
        assert!(matches!(captured[0], MarkRecord::Add { .. }));
        assert!(matches!(captured[1], MarkRecord::Remove { .. }));
    }

    #[test]
    fn update_then_remove_round_trip() {
        let mut session = TerminalSession::new(TerminalConfig::default()).unwrap();
        let id = session
            .add_annotation(single_line_range(4), "first".into())
            .expect("add");
        session
            .update_annotation_text(id, "second".into())
            .expect("update");
        let (_, payload, _) = session.annotations_in_range(single_line_range(4))[0];
        assert_eq!(payload.text, "second");
        session.remove_annotation(id).expect("remove");
        assert!(session.annotation_at_point(buf(4), 0).is_none());
        assert!(matches!(
            session.update_annotation_text(id, "ghost".into()),
            Err(AnnotationError::NotFound)
        ));
    }

    #[test]
    fn viewport_annotation_rebinds_to_buffered_on_scrollout() {
        // Build a session with a small viewport so we can control exactly
        // when capture_scrolled_out fires.
        let rows: u16 = 4;
        let cols: u16 = 20;
        let cfg = crate::TerminalConfig {
            cols,
            rows,
            max_scrollback: 1024,
            ..crate::TerminalConfig::default()
        };
        let mut session = crate::TerminalSession::new(cfg).expect("session");

        // Fill rows-1 lines so the viewport is nearly full but no scrolling
        // has occurred yet.  Feeding the last \r\n on a full viewport is what
        // triggers the scroll, so we stop one line short here.
        let mut payload = String::new();
        for i in 0..(rows - 1) {
            payload.push_str(&format!("line {i}\r\n"));
        }
        session.feed(payload.as_bytes()).expect("fill viewport");

        // After feeding exactly `rows` lines the viewport is full but the
        // first row has not yet been pushed into LineBuffer — it sits at the
        // top of the live ghost viewport.  Confirm it resolves to Viewport.
        let top_coord = session
            .screen_row_to_line_coord(0)
            .expect("screen row 0 resolves");
        assert!(
            matches!(top_coord, LineCoord::Viewport { .. }),
            "row 0 must still be Viewport before scrolling; got {top_coord:?}"
        );

        // Add an annotation on that viewport row.
        let vp_range = LineRange::new(top_coord, top_coord);
        let id = session
            .add_annotation(vp_range, "will travel".into())
            .expect("add viewport annotation");

        // Verify the mark is stored as Viewport in the tree.
        let mark_range_before = session
            .interval_tree()
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.range)
            .expect("mark present before scroll");
        assert!(
            matches!(mark_range_before.start, LineCoord::Viewport { .. }),
            "mark start must be Viewport before scroll; got {:?}",
            mark_range_before.start
        );

        // Feed one more line: the topmost row scrolls into LineBuffer and
        // capture_scrolled_out fires, triggering the rebind.
        session.feed(b"pushed\r\n").expect("push one row");

        // The mark must now be stored as Buffered.
        let mark_range_after = session
            .interval_tree()
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.range)
            .expect("mark still present after rebind");
        assert!(
            matches!(mark_range_after.start, LineCoord::Buffered(_)),
            "mark start must be Buffered after scrollback capture; got {:?}",
            mark_range_after.start
        );
        assert!(
            matches!(mark_range_after.end, LineCoord::Buffered(_)),
            "mark end must be Buffered after scrollback capture; got {:?}",
            mark_range_after.end
        );

        // The annotation must also be findable via annotations_in_range over
        // the buffered coordinate space.
        let buffered_coord = mark_range_after.start;
        let hits = session.annotations_in_range(LineRange::new(buffered_coord, buffered_coord));
        assert_eq!(
            hits.len(),
            1,
            "annotation must surface in buffered range query"
        );
        assert_eq!(hits[0].0, id);
        assert_eq!(hits[0].1.text, "will travel");
    }

    /// Feed enough `\r\n`-terminated lines through `session` so that
    /// at least the first `min_buffered` rows have scrolled into
    /// `LineBuffer`. Returns the session ready for screen-row queries.
    fn session_with_buffered_rows(cols: u16, rows: u16, lines: &[&str]) -> crate::TerminalSession {
        let cfg = crate::TerminalConfig {
            cols,
            rows,
            max_scrollback: 1024,
            ..crate::TerminalConfig::default()
        };
        let mut s = crate::TerminalSession::new(cfg).expect("session");
        let mut payload = String::new();
        for line in lines {
            payload.push_str(line);
            payload.push_str("\r\n");
        }
        s.feed(payload.as_bytes()).expect("feed");
        s
    }

    #[test]
    fn screen_row_to_line_coord_resolves_buffered_row() {
        // 5 sealed lines × 3-row viewport → at least the first 2 lines
        // land in `LineBuffer`. Row 0 must therefore resolve to a
        // `Buffered` anchor.
        let session =
            session_with_buffered_rows(20, 3, &["alpha", "bravo", "charlie", "delta", "echo"]);
        assert!(
            session.line_buffer().len() >= 2,
            "expected line_buffer to retain captured rows; got len={}",
            session.line_buffer().len()
        );
        let coord = session
            .screen_row_to_line_coord(0)
            .expect("buffered row resolves");
        assert!(
            matches!(coord, LineCoord::Buffered(_)),
            "row 0 must resolve to Buffered; got {coord:?}"
        );
    }

    #[test]
    fn screen_row_to_line_coord_resolves_viewport_row() {
        // Same fixture as above. The last visible row sits in the
        // live ghostty viewport, beyond `wrapped_row_count`.
        let session =
            session_with_buffered_rows(20, 3, &["alpha", "bravo", "charlie", "delta", "echo"]);
        let cols = session.cols();
        let lb_rows = session.line_buffer().wrapped_row_count(cols);
        // A row strictly inside the viewport region.
        let vp_row = lb_rows; // first viewport row
        let coord = session
            .screen_row_to_line_coord(vp_row)
            .expect("viewport row resolves");
        match coord {
            LineCoord::Viewport { abs_y } => {
                assert_eq!(
                    abs_y,
                    session.line_buffer().overflow() + vp_row as u64,
                    "viewport abs_y must equal overflow + row"
                );
            }
            other => panic!("expected Viewport; got {other:?}"),
        }
    }

    #[test]
    fn screen_row_to_line_coord_boundary_is_first_viewport_row() {
        // `wrapped_row_count` is exclusive: a row equal to `lb_rows`
        // is the *first* viewport row, not the last buffered row.
        let session =
            session_with_buffered_rows(20, 3, &["alpha", "bravo", "charlie", "delta", "echo"]);
        let lb_rows = session.line_buffer().wrapped_row_count(session.cols());
        if lb_rows == 0 {
            // The fixture must have produced at least one buffered row,
            // otherwise the boundary check is vacuous.
            panic!("fixture failed to populate line_buffer");
        }
        // Last buffered row.
        let last_buffered = session
            .screen_row_to_line_coord(lb_rows - 1)
            .expect("last buffered row resolves");
        assert!(
            matches!(last_buffered, LineCoord::Buffered(_)),
            "row lb_rows-1 must be Buffered; got {last_buffered:?}"
        );
        // First viewport row.
        let first_viewport = session
            .screen_row_to_line_coord(lb_rows)
            .expect("first viewport row resolves");
        assert!(
            matches!(first_viewport, LineCoord::Viewport { .. }),
            "row lb_rows must be Viewport; got {first_viewport:?}"
        );
    }

    #[test]
    fn screen_row_to_line_coord_empty_buffer_is_viewport() {
        // Fresh session, no PTY output: every row is in the viewport.
        let session = TerminalSession::new(TerminalConfig::default()).unwrap();
        assert_eq!(session.line_buffer().len(), 0);
        let coord = session
            .screen_row_to_line_coord(0)
            .expect("row 0 always resolves in empty buffer");
        match coord {
            LineCoord::Viewport { abs_y } => {
                assert_eq!(abs_y, 0, "empty buffer overflow is 0; abs_y == row");
            }
            other => panic!("expected Viewport on empty buffer; got {other:?}"),
        }
    }

    /// Verifies that `is_wrap_continuation` correctly identifies head vs.
    /// continuation rows for a wrapped logical line in `LineBuffer`.
    ///
    /// With cols=10 a 25-character line wraps into 3 visual rows:
    ///   row 0 — head      (sub_row == 0)
    ///   row 1 — cont.     (sub_row == 1)
    ///   row 2 — cont.     (sub_row == 2)
    ///
    /// This documents WHY the paint-side skip is needed: `annotation_at_point`
    /// returns the same match on all three rows (the logical-line id is shared),
    /// so without the skip the overlay would paint one box per wrapped visual row.
    #[test]
    fn wrap_continuation_is_detected_for_buffered_rows() {
        // cols=10 forces a 25-char line to wrap into 3 visual rows.
        // rows=4: feed 5 lines to push the first one into LineBuffer.
        let cols: u16 = 10;
        let rows: u16 = 4;
        let cfg = crate::TerminalConfig {
            cols,
            rows,
            max_scrollback: 1024,
            ..crate::TerminalConfig::default()
        };
        let mut session = crate::TerminalSession::new(cfg).expect("session");

        // Feed the long line (25 ASCII chars) as a soft-wrap sequence:
        // ghostty receives no newline at the right margin, so the row wraps.
        // We then feed 4 more short hard-newline-terminated lines to push the
        // wrapped logical line out of the viewport and into LineBuffer.
        //
        // Build one PTY chunk: the 25-char line followed by \r\n to seal it,
        // then 4 more lines to fill the viewport.
        let long_line = "ABCDEFGHIJKLMNOPQRSTUVWXY"; // exactly 25 chars
        assert_eq!(long_line.len(), 25);
        let mut payload = String::new();
        payload.push_str(long_line);
        payload.push_str("\r\n");
        for i in 0..rows {
            payload.push_str(&format!("extra{i}\r\n"));
        }
        session.feed(payload.as_bytes()).expect("feed");

        // The 25-char line should now be in LineBuffer wrapped at cols=10.
        // Confirm the buffer captured at least 3 visual rows (the 3 wrap rows).
        let lb_rows = session.line_buffer().wrapped_row_count(cols);
        assert!(
            lb_rows >= 3,
            "expected >= 3 buffered rows (wrap segments); got {lb_rows}"
        );

        // Locate the first visual row in LineBuffer occupied by the long line.
        // It wraps into 3 rows: row 0 = head, 1 = continuation, 2 = continuation.
        // Find the first logical line whose rows_at_width(cols) == 3.
        let head_row = (0..lb_rows)
            .find(|&r| {
                session
                    .line_buffer()
                    .position_for_visual_row(r, cols)
                    .map(|(_, sub, _)| sub == 0)
                    .unwrap_or(false)
                    && session
                        .line_buffer()
                        .position_for_visual_row(r + 1, cols)
                        .zip(session.line_buffer().position_for_visual_row(r, cols))
                        .map(|((pos1, sub1, _), (pos0, _, _))| pos1 == pos0 && sub1 == 1)
                        .unwrap_or(false)
            })
            .expect("wrapped head row must exist");

        let cont_row = head_row + 1;
        let cont_row2 = head_row + 2;

        // Head row is NOT a continuation.
        assert!(
            !session.is_wrap_continuation(head_row),
            "head row {head_row} must not be a wrap continuation"
        );
        // First continuation row IS a continuation.
        assert!(
            session.is_wrap_continuation(cont_row),
            "row {cont_row} must be a wrap continuation (sub_row == 1)"
        );
        // Second continuation row IS also a continuation.
        assert!(
            session.is_wrap_continuation(cont_row2),
            "row {cont_row2} must be a wrap continuation (sub_row == 2)"
        );

        // Demonstrate WHY the paint skip is needed: annotation_at_point
        // returns the same mark on the head row AND the continuation rows,
        // because LineBufferPosition is the logical-line id shared by all wrapped rows.
        let head_coord = session
            .screen_row_to_line_coord(head_row)
            .expect("head coord resolves");
        let cont_coord = session
            .screen_row_to_line_coord(cont_row)
            .expect("cont coord resolves");

        // Register an annotation on the head coordinate.
        let id = session
            .add_annotation(LineRange::new(head_coord, head_coord), "wrap-test".into())
            .expect("add annotation");

        // Both head and continuation resolve to the same LineCoord::Buffered
        // position (same logical line) so annotation_at_point matches both.
        let head_hit = session.annotation_at_point(head_coord, 0);
        let cont_hit = session.annotation_at_point(cont_coord, 0);
        assert!(head_hit.is_some(), "annotation must match on head row");
        assert!(
            cont_hit.is_some(),
            "annotation also matches on continuation row — \
             this is WHY the paint loop must skip continuation rows"
        );
        assert_eq!(head_hit.unwrap().0, id);
        assert_eq!(cont_hit.unwrap().0, id);
    }
}
