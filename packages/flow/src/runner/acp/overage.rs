//! Stopping a run once Claude reports that a reply is billed as extra usage.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use daruda_acp::RateLimitView;

/// A run's overage switch. `enabled` is shared with the host's setting, so a
/// toggle mid-run applies at once; `tripped` is per run and keeps the first
/// billed report for the host's end-of-run notice. The default never trips.
#[derive(Debug, Clone, Default)]
pub struct OverageStop {
    enabled: Arc<AtomicBool>,
    tripped: Arc<OnceLock<RateLimitView>>,
}

impl OverageStop {
    /// A fresh run's switch, reading the host's live `enabled` flag.
    pub fn new(enabled: Arc<AtomicBool>) -> Self {
        Self {
            enabled,
            tripped: Arc::default(),
        }
    }

    /// Records the first billed report when enabled; `true` only for that
    /// first one, so the caller cancels and logs the stop once.
    pub fn trip(&self, limit: &RateLimitView) -> bool {
        limit.bills_overage()
            && self.enabled.load(Ordering::SeqCst)
            && self.tripped.set(limit.clone()).is_ok()
    }

    /// The report that stopped the run, if one did.
    pub fn tripped(&self) -> Option<&RateLimitView> {
        self.tripped.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_acp::RateLimitWindow;

    fn billed(resets_at: i64, is_using_overage: bool) -> RateLimitView {
        RateLimitView {
            window: Some(RateLimitWindow::FiveHour),
            resets_at: Some(resets_at),
            is_using_overage,
        }
    }

    #[test]
    fn a_disabled_switch_never_trips() {
        let stop = OverageStop::new(Arc::new(AtomicBool::new(false)));
        assert!(!stop.trip(&billed(1, true)));
        assert_eq!(stop.tripped(), None);
        assert!(!OverageStop::default().trip(&billed(1, true)));
    }

    #[test]
    fn an_enabled_switch_trips_on_a_billed_report_and_keeps_the_first() {
        let stop = OverageStop::new(Arc::new(AtomicBool::new(true)));
        assert!(!stop.trip(&billed(1, false)), "not billed");
        assert!(stop.trip(&billed(1, true)));
        assert!(!stop.trip(&billed(2, true)), "already stopped");
        assert_eq!(stop.tripped(), Some(&billed(1, true)));
    }

    #[test]
    fn flipping_the_shared_flag_applies_to_a_switch_built_earlier() {
        let enabled = Arc::new(AtomicBool::new(false));
        let stop = OverageStop::new(enabled.clone());
        let held_by_host = stop.clone();
        assert!(!stop.trip(&billed(1, true)));
        enabled.store(true, Ordering::SeqCst);
        assert!(stop.trip(&billed(1, true)));
        assert!(held_by_host.tripped().is_some(), "clones share `tripped`");
    }
}
