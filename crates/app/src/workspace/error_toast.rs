//! Toast queue — Layer 1 of the error-reporting pipeline.
//!
//! A bounded ring of [`ErrorToast`]s mirrors the most recent
//! [`ErrorReport`]s (Layer 0 of the pipeline) into a UI-bound shape:
//! repeat counters, severity-driven expiry, and FIFO eviction when
//! the cap is hit. The renderer (`workspace/render/error_toast_overlay.rs`)
//! reads this struct snapshot-style; mutations route through
//! [`Workspace::report_error`].
//!
//! - **D3** — capacity 3; pushing onto a full queue evicts the oldest.
//! - **D4** — auto-dismiss = `severity.auto_dismiss_after()` from the
//!   moment of the most recent push (refreshed on dedup hit).
//!
//! GPUI-free: takes [`Instant`] from the caller so unit tests can
//! drive the clock without a `TestAppContext`.

use std::time::Instant;

use daruda_store::observability::error_report::ErrorReport;

/// Default capacity (D3). Public so tests can pin the value
/// without copying the literal.
pub(super) const TOAST_CAP: usize = 3;

/// Stable identifier for a live toast. Allocated by the queue at push
/// time and never reused within the queue's lifetime. The renderer
/// captures the id into its click handlers so a dismiss click sent
/// after an unrelated auto-expire shifts indices around still removes
/// the right toast (the index-based version was racy by 1 s).
pub(super) type ToastId = u64;

/// One live toast. Cheap to clone — the underlying [`ErrorReport`] is
/// already cheap-clone (small heap fields, stable timestamp).
#[derive(Clone, Debug)]
pub(super) struct ErrorToast {
    pub(super) id: ToastId,
    pub(super) report: ErrorReport,
    /// 1 on first push; incremented every time a report with the same
    /// `dedup_key` arrives while this toast is alive. The renderer
    /// only shows the badge when this is `>= 2`.
    pub(super) repeat_count: u32,
    /// Wall-clock instant after which the toast auto-dismisses. Set
    /// to `last_push_time + severity.auto_dismiss_after()`. Refreshed
    /// on every dedup hit so a busy error keeps its toast on screen.
    pub(super) expires_at: Instant,
}

/// Bounded FIFO of [`ErrorToast`]s. Capacity-stable across pushes —
/// pushing onto a full queue evicts the oldest entry.
#[derive(Debug)]
pub(super) struct ErrorToastQueue {
    toasts: Vec<ErrorToast>,
    capacity: usize,
    next_id: ToastId,
}

impl Default for ErrorToastQueue {
    fn default() -> Self {
        Self::new(TOAST_CAP)
    }
}

impl ErrorToastQueue {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            toasts: Vec::with_capacity(capacity),
            capacity,
            next_id: 1,
        }
    }

    /// Apply a freshly-built report to the queue.
    ///
    /// - When the report's `dedup_key` matches an existing toast, that
    ///   toast's `repeat_count` increments and its `expires_at`
    ///   refreshes from `now`. The toast is *not* moved within the
    ///   queue; visual position stays stable.
    /// - Otherwise the report becomes a new toast appended to the
    ///   end. If the queue is at capacity the oldest toast is evicted
    ///   (same path as user ✕ — the renderer treats both identically).
    ///
    /// Returns `true` when the queue's visible state changed and the
    /// renderer should be notified.
    pub(super) fn push(&mut self, report: ErrorReport, now: Instant) -> bool {
        let dismiss_after = report.severity.auto_dismiss_after();
        let expires_at = now + dismiss_after;

        if let Some(key) = report.dedup_key.as_deref()
            && let Some(existing) = self
                .toasts
                .iter_mut()
                .find(|t| t.report.dedup_key.as_deref() == Some(key))
        {
            existing.repeat_count = existing.repeat_count.saturating_add(1);
            existing.expires_at = expires_at;
            // Keep the most recent payload so the title / message / context
            // visible to the user reflect the latest occurrence.
            existing.report = report;
            return true;
        }

        if self.toasts.len() == self.capacity && !self.toasts.is_empty() {
            self.toasts.remove(0);
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.toasts.push(ErrorToast {
            id,
            report,
            repeat_count: 1,
            expires_at,
        });
        true
    }

    /// Remove the toast with the given stable id. Returns `true` when
    /// a toast was found + removed. Stale ids (e.g. the toast already
    /// auto-expired between the user's click and this call) are a
    /// no-op — the caller can ignore the return value.
    pub(super) fn dismiss_id(&mut self, id: ToastId) -> bool {
        if let Some(pos) = self.toasts.iter().position(|t| t.id == id) {
            self.toasts.remove(pos);
            true
        } else {
            false
        }
    }

    /// Drop every toast whose `expires_at` is at or before `now`.
    /// Returns `true` when at least one toast was removed.
    pub(super) fn expire_tick(&mut self, now: Instant) -> bool {
        let before = self.toasts.len();
        self.toasts.retain(|t| t.expires_at > now);
        self.toasts.len() != before
    }

    pub(super) fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }

    #[allow(dead_code)] // Reserved for future test / palette surfaces; kept for symmetry with iter().
    pub(super) fn len(&self) -> usize {
        self.toasts.len()
    }

    pub(super) fn iter(&self) -> std::slice::Iter<'_, ErrorToast> {
        self.toasts.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::observability::error_report::ErrorSeverity;
    use std::time::Duration;

    fn report(title: &str, severity: ErrorSeverity, dedup: Option<&str>) -> ErrorReport {
        let mut b = ErrorReport::new(title)
            .severity(severity)
            .message(format!("synthetic: {title}"));
        if let Some(k) = dedup {
            b = b.dedup(k);
        }
        b.build()
    }

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn push_appends_and_increments_count_on_dedup() {
        let mut q = ErrorToastQueue::new(3);
        let now = t0();

        q.push(report("First", ErrorSeverity::Error, Some("k1")), now);
        q.push(report("Second", ErrorSeverity::Warning, Some("k2")), now);
        q.push(report("First again", ErrorSeverity::Error, Some("k1")), now);

        assert_eq!(q.len(), 2, "second push with same key dedupes");
        let toasts: Vec<_> = q.iter().collect();
        // Existing position preserved (k1 entry stays first), but the
        // payload is the most recent.
        assert_eq!(toasts[0].repeat_count, 2);
        assert_eq!(toasts[0].report.title, "First again");
        assert_eq!(toasts[1].repeat_count, 1);
        assert_eq!(toasts[1].report.title, "Second");
    }

    #[test]
    fn push_without_dedup_key_never_merges() {
        let mut q = ErrorToastQueue::new(3);
        let now = t0();
        q.push(report("A", ErrorSeverity::Info, None), now);
        q.push(report("B", ErrorSeverity::Info, None), now);
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn capacity_evicts_oldest_when_full() {
        let mut q = ErrorToastQueue::new(3);
        let now = t0();
        q.push(report("A", ErrorSeverity::Error, Some("a")), now);
        q.push(report("B", ErrorSeverity::Error, Some("b")), now);
        q.push(report("C", ErrorSeverity::Error, Some("c")), now);
        q.push(report("D", ErrorSeverity::Error, Some("d")), now);

        let titles: Vec<&str> = q.iter().map(|t| t.report.title.as_str()).collect();
        assert_eq!(titles, vec!["B", "C", "D"], "A should be evicted");
        assert_eq!(q.len(), 3);
    }

    #[test]
    fn expire_tick_drops_severity_appropriate_window() {
        let mut q = ErrorToastQueue::new(3);
        let t0 = t0();
        q.push(report("info", ErrorSeverity::Info, Some("i")), t0);
        q.push(report("warn", ErrorSeverity::Warning, Some("w")), t0);
        q.push(report("err", ErrorSeverity::Error, Some("e")), t0);

        // Just past Info's 5 s — only `info` expires.
        let t1 = t0 + Duration::from_secs(6);
        assert!(q.expire_tick(t1));
        let titles: Vec<&str> = q.iter().map(|t| t.report.title.as_str()).collect();
        assert_eq!(titles, vec!["warn", "err"]);

        // Past Warning's 8 s — `warn` expires too, `err` (30 s) stays.
        let t2 = t0 + Duration::from_secs(10);
        assert!(q.expire_tick(t2));
        let titles: Vec<&str> = q.iter().map(|t| t.report.title.as_str()).collect();
        assert_eq!(titles, vec!["err"]);

        // Past Error's 30 s.
        let t3 = t0 + Duration::from_secs(31);
        assert!(q.expire_tick(t3));
        assert!(q.is_empty());
    }

    #[test]
    fn dedup_refreshes_expires_at_so_repeat_keeps_toast_alive() {
        let mut q = ErrorToastQueue::new(3);
        let t0 = t0();
        q.push(report("flap", ErrorSeverity::Info, Some("flap")), t0);

        // 4 s in — push the same key again. The 5 s window resets.
        let t1 = t0 + Duration::from_secs(4);
        q.push(report("flap", ErrorSeverity::Info, Some("flap")), t1);

        // 6 s after t0 = only 2 s after the refresh — toast still alive.
        let t2 = t0 + Duration::from_secs(6);
        assert!(!q.expire_tick(t2));
        assert_eq!(q.len(), 1);
        assert_eq!(q.iter().next().unwrap().repeat_count, 2);

        // 10 s after t0 = 6 s after refresh — past Info window.
        let t3 = t0 + Duration::from_secs(10);
        assert!(q.expire_tick(t3));
        assert!(q.is_empty());
    }

    #[test]
    fn dismiss_id_removes_specific_toast() {
        let mut q = ErrorToastQueue::new(3);
        let now = t0();
        q.push(report("A", ErrorSeverity::Info, Some("a")), now);
        q.push(report("B", ErrorSeverity::Info, Some("b")), now);
        q.push(report("C", ErrorSeverity::Info, Some("c")), now);

        let b_id = q.iter().find(|t| t.report.title == "B").unwrap().id;
        assert!(q.dismiss_id(b_id));
        let titles: Vec<&str> = q.iter().map(|t| t.report.title.as_str()).collect();
        assert_eq!(titles, vec!["A", "C"]);

        assert!(
            !q.dismiss_id(b_id),
            "dismissing the same id twice is a no-op",
        );
        assert!(
            !q.dismiss_id(9999),
            "unknown id (e.g. already auto-expired) is a no-op",
        );
    }

    /// Regression: stable ids prevent the dismiss-vs-expire race —
    /// a click captures the id at render time, so even if an unrelated
    /// auto-expire shifts indices between click and handler, the
    /// dismissed toast is still the one the user pressed.
    #[test]
    fn dismiss_id_survives_concurrent_expire() {
        let mut q = ErrorToastQueue::new(3);
        let t0 = t0();
        // A is Info (5 s window), B is Warning (8 s window). At t0+6s
        // A expires but B still has 2 s of life left.
        q.push(report("A", ErrorSeverity::Info, Some("a")), t0);
        q.push(report("B", ErrorSeverity::Warning, Some("b")), t0);
        let b_id = q.iter().find(|t| t.report.title == "B").unwrap().id;

        // Simulate the timeline: user observed B at index 1, then A
        // expires before the click handler runs. Index 1 is now stale
        // and would dismiss the wrong toast under the index-based API.
        q.expire_tick(t0 + std::time::Duration::from_secs(6));
        assert_eq!(q.len(), 1);
        assert_eq!(q.iter().next().unwrap().report.title, "B");

        // Stable id still hits B even though its position shifted.
        assert!(
            q.dismiss_id(b_id),
            "stable id removes B even after A expired",
        );
        assert!(q.is_empty());
    }
}
