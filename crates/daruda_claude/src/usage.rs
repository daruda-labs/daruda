//! Token-usage accounting for Claude sessions.
//!
//! Each `assistant`-role message in a session JSONL carries a `usage`
//! block (`input_tokens`, `output_tokens`, `cache_read_input_tokens`,
//! `cache_creation_input_tokens`). The JSONL parser
//! ([`crate::jsonl`]) extracts those into [`UsageDelta`]s; the
//! workspace folds them into [`UsageState`] keyed by `session_id`.
//! The right-dock Usage tab renders the totals + per-session rows
//! using [`SessionUsage::estimated_cost`] for cost estimation.
//!
//! GPUI-free — every type here is plain data. The runtime
//! `SystemTime` value comes from the apply call site, so
//! `#[cfg(test)]` callers can inject fake clocks (or just rely on
//! `SystemTime::now()` and check ordering).

use std::{collections::HashMap, path::PathBuf, time::SystemTime};

/// Accumulated token usage for one Claude session.
///
/// Also serves as the return type of [`UsageState::total`]; in the
/// total case `session_id`, `worktree_path`, and `last_updated` carry
/// their `Default` values and callers should ignore them — only the
/// token + message counters are meaningful.
#[derive(Clone, Debug)]
pub struct SessionUsage {
    pub session_id: String,
    pub worktree_path: PathBuf,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub message_count: u32,
    pub last_updated: SystemTime,
}

impl Default for SessionUsage {
    /// `SystemTime` has no `Default` so we anchor on `UNIX_EPOCH`. The
    /// first `UsageState::apply` call replaces it with `now()`, so
    /// runtime sessions never expose this sentinel; only freshly-built
    /// totals from `UsageState::total` do, and callers ignore the
    /// `last_updated` field there per the doc comment above.
    fn default() -> Self {
        Self {
            session_id: String::new(),
            worktree_path: PathBuf::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            message_count: 0,
            last_updated: SystemTime::UNIX_EPOCH,
        }
    }
}

impl SessionUsage {
    /// Estimated cost in USD using the supplied per-million-token
    /// `pricing`. Defaults to Sonnet-4.6 list pricing when the caller
    /// passes [`UsagePricing::default`].
    pub fn estimated_cost(&self, pricing: &UsagePricing) -> f64 {
        let mtok = 1_000_000.0;
        (self.input_tokens as f64 / mtok) * pricing.input_per_mtok
            + (self.output_tokens as f64 / mtok) * pricing.output_per_mtok
            + (self.cache_read_tokens as f64 / mtok) * pricing.cache_read_per_mtok
            + (self.cache_creation_tokens as f64 / mtok) * pricing.cache_write_per_mtok
    }
}

/// Per-million-token prices in USD.
#[derive(Clone, Debug, PartialEq)]
pub struct UsagePricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_write_per_mtok: f64,
}

impl Default for UsagePricing {
    /// Sonnet-4.6 list pricing as of 2026-05.
    fn default() -> Self {
        Self {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: 0.30,
            cache_write_per_mtok: 3.75,
        }
    }
}

/// In-memory mirror of all tracked sessions, keyed by `session_id`.
#[derive(Clone, Debug, Default)]
pub struct UsageState {
    pub sessions: HashMap<String, SessionUsage>,
}

impl UsageState {
    /// Sum of all sessions' token + message counters. Identity fields
    /// (`session_id`, `worktree_path`, `last_updated`) on the result
    /// are at their `Default` values — callers reading totals should
    /// only consume the counter fields.
    pub fn total(&self) -> SessionUsage {
        let mut t = SessionUsage::default();
        for s in self.sessions.values() {
            t.input_tokens += s.input_tokens;
            t.output_tokens += s.output_tokens;
            t.cache_read_tokens += s.cache_read_tokens;
            t.cache_creation_tokens += s.cache_creation_tokens;
            t.message_count += s.message_count;
        }
        t
    }

    /// Fold one parsed JSONL entry into the matching session row.
    /// Creates the row if it does not yet exist; otherwise accumulates
    /// the delta and bumps `message_count`. Updates `last_updated`
    /// to `when` only if it is newer than the stored value, so a
    /// late-arriving older entry (rare, but possible if a session's
    /// jsonl is re-read with a different tail window) doesn't move
    /// the session backwards in time.
    ///
    /// `when` should be the **JSONL entry's own timestamp**, not the
    /// caller's wall clock. The watcher derives this from
    /// `last_meaningful_timestamp`; passing `SystemTime::now()` would
    /// stamp every cold-restore session with the launch instant and
    /// defeat any time-window filter on the rendered output.
    pub fn apply(
        &mut self,
        session_id: &str,
        worktree_path: PathBuf,
        delta: &UsageDelta,
        when: SystemTime,
    ) {
        let entry = self.sessions.entry(session_id.to_string()).or_default();
        entry.session_id = session_id.to_string();
        entry.worktree_path = worktree_path;
        entry.input_tokens += delta.input_tokens;
        entry.output_tokens += delta.output_tokens;
        entry.cache_read_tokens += delta.cache_read_tokens;
        entry.cache_creation_tokens += delta.cache_creation_tokens;
        entry.message_count += 1;
        if when > entry.last_updated {
            entry.last_updated = when;
        }
    }

    /// Drop a session from the map (e.g. after the JSONL file is
    /// deleted or rotated). Silently no-ops if the id is unknown.
    pub fn remove_session(&mut self, session_id: &str) {
        self.sessions.remove(session_id);
    }

    /// Iterator over sessions whose `last_updated` is at or after
    /// `cutoff`. `None` means "no filter" — all sessions pass.
    /// The iterator order matches `HashMap` iteration (unordered);
    /// callers that need a stable order should sort the result.
    pub fn filtered_sessions(
        &self,
        cutoff: Option<SystemTime>,
    ) -> impl Iterator<Item = &SessionUsage> {
        self.sessions
            .values()
            .filter(move |s| cutoff.is_none_or(|c| s.last_updated >= c))
    }

    /// Sum of token + message counters for sessions inside `cutoff`.
    /// `None` cutoff returns the full lifetime total (same as
    /// [`Self::total`]). Identity fields on the returned
    /// `SessionUsage` are at their `Default` values; callers should
    /// only consume the counters.
    pub fn filtered_total(&self, cutoff: Option<SystemTime>) -> SessionUsage {
        let mut t = SessionUsage::default();
        for s in self.filtered_sessions(cutoff) {
            t.input_tokens += s.input_tokens;
            t.output_tokens += s.output_tokens;
            t.cache_read_tokens += s.cache_read_tokens;
            t.cache_creation_tokens += s.cache_creation_tokens;
            t.message_count += s.message_count;
        }
        t
    }
}

/// Incremental usage extracted from one JSONL entry's `usage` block.
/// JSON keys map as:
/// - `input_tokens`               → [`Self::input_tokens`]
/// - `output_tokens`              → [`Self::output_tokens`]
/// - `cache_read_input_tokens`    → [`Self::cache_read_tokens`]
/// - `cache_creation_input_tokens` → [`Self::cache_creation_tokens`]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageDelta {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
}

impl UsageDelta {
    /// Convert the strongly-typed `parser::Usage` (where every field
    /// is `Option<u32>`, missing keys → `None`) into a [`UsageDelta`].
    /// Missing fields become 0; widening to `u64` removes the JSONL
    /// crate's per-message u32 ceiling when callers accumulate over
    /// long sessions.
    pub fn from_jsonl_usage(u: &crate::jsonl::parser::Usage) -> Self {
        Self {
            input_tokens: u.input_tokens.unwrap_or(0) as u64,
            output_tokens: u.output_tokens.unwrap_or(0) as u64,
            cache_read_tokens: u.cache_read_input_tokens.unwrap_or(0) as u64,
            cache_creation_tokens: u.cache_creation_input_tokens.unwrap_or(0) as u64,
        }
    }

    /// `true` if every counter is zero — used by callers to skip
    /// emitting empty events.
    pub fn is_empty(&self) -> bool {
        self.input_tokens == 0
            && self.output_tokens == 0
            && self.cache_read_tokens == 0
            && self.cache_creation_tokens == 0
    }

    /// Add another delta into self in place. Saturating on overflow —
    /// realistic token counts stay far below `u64::MAX` so overflow
    /// would only happen on a malicious or corrupted log.
    pub fn add_assign(&mut self, other: &UsageDelta) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(other.cache_read_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(other.cache_creation_tokens);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn delta(input: u64, output: u64, read: u64, write: u64) -> UsageDelta {
        UsageDelta {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: read,
            cache_creation_tokens: write,
        }
    }

    /// Build a `SystemTime` from "seconds since UNIX_EPOCH" — keeps
    /// the test arithmetic readable without dragging in `chrono`.
    fn ts(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn apply_creates_new_session_with_one_message() {
        let mut state = UsageState::default();
        state.apply(
            "sess-1",
            PathBuf::from("/repo"),
            &delta(100, 50, 30, 20),
            ts(1_700_000_000),
        );

        assert_eq!(state.sessions.len(), 1);
        let s = state.sessions.get("sess-1").unwrap();
        assert_eq!(s.session_id, "sess-1");
        assert_eq!(s.worktree_path, PathBuf::from("/repo"));
        assert_eq!(s.input_tokens, 100);
        assert_eq!(s.output_tokens, 50);
        assert_eq!(s.cache_read_tokens, 30);
        assert_eq!(s.cache_creation_tokens, 20);
        assert_eq!(s.message_count, 1);
        assert_eq!(s.last_updated, ts(1_700_000_000));
    }

    #[test]
    fn apply_accumulates_repeated_calls_for_same_session() {
        let mut state = UsageState::default();
        state.apply(
            "sess-1",
            PathBuf::from("/repo"),
            &delta(10, 5, 0, 0),
            ts(1_700_000_000),
        );
        state.apply(
            "sess-1",
            PathBuf::from("/repo"),
            &delta(20, 8, 4, 0),
            ts(1_700_000_000),
        );
        state.apply(
            "sess-1",
            PathBuf::from("/repo"),
            &delta(0, 1, 0, 2),
            ts(1_700_000_000),
        );

        let s = state.sessions.get("sess-1").unwrap();
        assert_eq!(s.input_tokens, 30);
        assert_eq!(s.output_tokens, 14);
        assert_eq!(s.cache_read_tokens, 4);
        assert_eq!(s.cache_creation_tokens, 2);
        assert_eq!(s.message_count, 3);
    }

    #[test]
    fn apply_keeps_sessions_separate() {
        let mut state = UsageState::default();
        state.apply(
            "a",
            PathBuf::from("/a"),
            &delta(10, 0, 0, 0),
            ts(1_700_000_000),
        );
        state.apply(
            "b",
            PathBuf::from("/b"),
            &delta(0, 5, 0, 0),
            ts(1_700_000_000),
        );

        assert_eq!(state.sessions.len(), 2);
        assert_eq!(state.sessions["a"].input_tokens, 10);
        assert_eq!(state.sessions["a"].output_tokens, 0);
        assert_eq!(state.sessions["b"].input_tokens, 0);
        assert_eq!(state.sessions["b"].output_tokens, 5);
        assert_eq!(state.sessions["a"].worktree_path, PathBuf::from("/a"));
        assert_eq!(state.sessions["b"].worktree_path, PathBuf::from("/b"));
    }

    #[test]
    fn apply_overwrites_worktree_path_for_existing_session() {
        // A session that switches cwd mid-stream (rare but possible
        // when the user `cd`s inside a Claude shell session).
        let mut state = UsageState::default();
        state.apply(
            "sess-1",
            PathBuf::from("/old"),
            &delta(1, 0, 0, 0),
            ts(1_700_000_000),
        );
        state.apply(
            "sess-1",
            PathBuf::from("/new"),
            &delta(1, 0, 0, 0),
            ts(1_700_000_000),
        );

        assert_eq!(
            state.sessions["sess-1"].worktree_path,
            PathBuf::from("/new")
        );
        // Token counters still accumulate.
        assert_eq!(state.sessions["sess-1"].input_tokens, 2);
    }

    #[test]
    fn total_sums_all_session_counters() {
        let mut state = UsageState::default();
        state.apply(
            "a",
            PathBuf::from("/a"),
            &delta(100, 50, 30, 20),
            ts(1_700_000_000),
        );
        state.apply(
            "b",
            PathBuf::from("/b"),
            &delta(200, 80, 10, 5),
            ts(1_700_000_000),
        );
        state.apply(
            "a",
            PathBuf::from("/a"),
            &delta(50, 25, 0, 0),
            ts(1_700_000_000),
        );

        let t = state.total();
        assert_eq!(t.input_tokens, 350);
        assert_eq!(t.output_tokens, 155);
        assert_eq!(t.cache_read_tokens, 40);
        assert_eq!(t.cache_creation_tokens, 25);
        assert_eq!(t.message_count, 3);
    }

    #[test]
    fn total_on_empty_state_returns_zero_counters() {
        let state = UsageState::default();
        let t = state.total();
        assert_eq!(t.input_tokens, 0);
        assert_eq!(t.output_tokens, 0);
        assert_eq!(t.cache_read_tokens, 0);
        assert_eq!(t.cache_creation_tokens, 0);
        assert_eq!(t.message_count, 0);
    }

    #[test]
    fn remove_session_drops_entry() {
        let mut state = UsageState::default();
        state.apply(
            "a",
            PathBuf::from("/a"),
            &delta(10, 0, 0, 0),
            ts(1_700_000_000),
        );
        state.apply(
            "b",
            PathBuf::from("/b"),
            &delta(20, 0, 0, 0),
            ts(1_700_000_000),
        );
        state.remove_session("a");

        assert_eq!(state.sessions.len(), 1);
        assert!(state.sessions.contains_key("b"));
    }

    #[test]
    fn remove_unknown_session_is_noop() {
        let mut state = UsageState::default();
        state.apply(
            "a",
            PathBuf::from("/a"),
            &delta(10, 0, 0, 0),
            ts(1_700_000_000),
        );
        state.remove_session("does-not-exist");
        assert_eq!(state.sessions.len(), 1);
    }

    #[test]
    fn estimated_cost_is_zero_for_zero_tokens() {
        let s = SessionUsage::default();
        assert_eq!(s.estimated_cost(&UsagePricing::default()), 0.0);
    }

    #[test]
    fn estimated_cost_charges_each_bucket_separately() {
        // 1M of each kind so the result equals the sum of the four
        // unit prices.
        let s = SessionUsage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_read_tokens: 1_000_000,
            cache_creation_tokens: 1_000_000,
            ..Default::default()
        };

        let p = UsagePricing::default();
        let expected =
            p.input_per_mtok + p.output_per_mtok + p.cache_read_per_mtok + p.cache_write_per_mtok;
        assert!((s.estimated_cost(&p) - expected).abs() < 1e-9);
    }

    #[test]
    fn estimated_cost_scales_linearly_with_tokens() {
        let p = UsagePricing::default();
        let half = SessionUsage {
            input_tokens: 500_000,
            ..Default::default()
        }
        .estimated_cost(&p);
        let full = SessionUsage {
            input_tokens: 1_000_000,
            ..Default::default()
        }
        .estimated_cost(&p);
        assert!((full - 2.0 * half).abs() < 1e-9);
    }

    #[test]
    fn estimated_cost_uses_custom_pricing() {
        let s = SessionUsage {
            input_tokens: 1_000_000,
            ..Default::default()
        };
        let p = UsagePricing {
            input_per_mtok: 2.5,
            output_per_mtok: 0.0,
            cache_read_per_mtok: 0.0,
            cache_write_per_mtok: 0.0,
        };
        assert!((s.estimated_cost(&p) - 2.5).abs() < 1e-9);
    }

    #[test]
    fn usage_pricing_default_matches_sonnet_4_6_list_prices() {
        let p = UsagePricing::default();
        assert_eq!(p.input_per_mtok, 3.0);
        assert_eq!(p.output_per_mtok, 15.0);
        assert_eq!(p.cache_read_per_mtok, 0.30);
        assert_eq!(p.cache_write_per_mtok, 3.75);
    }

    #[test]
    fn delta_from_jsonl_usage_zeros_missing_fields() {
        let u = crate::jsonl::parser::Usage {
            input_tokens: None,
            output_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        };
        assert_eq!(UsageDelta::from_jsonl_usage(&u), UsageDelta::default());
    }

    #[test]
    fn delta_from_jsonl_usage_passes_present_fields_through() {
        let u = crate::jsonl::parser::Usage {
            input_tokens: Some(123),
            output_tokens: Some(45),
            cache_creation_input_tokens: Some(67),
            cache_read_input_tokens: Some(89),
        };
        let d = UsageDelta::from_jsonl_usage(&u);
        assert_eq!(d.input_tokens, 123);
        assert_eq!(d.output_tokens, 45);
        assert_eq!(d.cache_creation_tokens, 67);
        assert_eq!(d.cache_read_tokens, 89);
    }

    #[test]
    fn delta_from_jsonl_usage_handles_partial_presence() {
        let u = crate::jsonl::parser::Usage {
            input_tokens: Some(100),
            output_tokens: None,
            cache_creation_input_tokens: Some(5),
            cache_read_input_tokens: None,
        };
        let d = UsageDelta::from_jsonl_usage(&u);
        assert_eq!(d.input_tokens, 100);
        assert_eq!(d.output_tokens, 0);
        assert_eq!(d.cache_creation_tokens, 5);
        assert_eq!(d.cache_read_tokens, 0);
    }

    #[test]
    fn delta_is_empty_only_when_all_zero() {
        assert!(UsageDelta::default().is_empty());
        let d = UsageDelta {
            input_tokens: 1,
            ..Default::default()
        };
        assert!(!d.is_empty());
    }

    #[test]
    fn delta_add_assign_sums_each_bucket() {
        let mut a = UsageDelta {
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 3,
            cache_creation_tokens: 2,
        };
        let b = UsageDelta {
            input_tokens: 20,
            output_tokens: 8,
            cache_read_tokens: 1,
            cache_creation_tokens: 4,
        };
        a.add_assign(&b);
        assert_eq!(a.input_tokens, 30);
        assert_eq!(a.output_tokens, 13);
        assert_eq!(a.cache_read_tokens, 4);
        assert_eq!(a.cache_creation_tokens, 6);
    }

    #[test]
    fn delta_add_assign_saturates_on_overflow() {
        let mut a = UsageDelta {
            input_tokens: u64::MAX,
            ..Default::default()
        };
        a.add_assign(&UsageDelta {
            input_tokens: 5,
            ..Default::default()
        });
        assert_eq!(a.input_tokens, u64::MAX);
    }

    #[test]
    fn filtered_sessions_with_none_cutoff_returns_everything() {
        let mut state = UsageState::default();
        state.apply("a", PathBuf::from("/a"), &delta(10, 0, 0, 0), ts(1_000));
        state.apply("b", PathBuf::from("/b"), &delta(20, 0, 0, 0), ts(2_000));
        let count = state.filtered_sessions(None).count();
        assert_eq!(count, 2);
    }

    #[test]
    fn filtered_sessions_excludes_entries_older_than_cutoff() {
        let mut state = UsageState::default();
        state.apply("old", PathBuf::from("/old"), &delta(10, 0, 0, 0), ts(1_000));
        state.apply("new", PathBuf::from("/new"), &delta(20, 0, 0, 0), ts(5_000));
        let inside: Vec<_> = state
            .filtered_sessions(Some(ts(3_000)))
            .map(|s| s.session_id.clone())
            .collect();
        assert_eq!(inside, vec!["new".to_string()]);
    }

    #[test]
    fn filtered_sessions_includes_boundary_timestamp() {
        // Cutoff is inclusive (>=), so a session whose last_updated
        // equals the cutoff still passes.
        let mut state = UsageState::default();
        state.apply("a", PathBuf::from("/a"), &delta(1, 0, 0, 0), ts(3_000));
        let count = state.filtered_sessions(Some(ts(3_000))).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn filtered_total_aggregates_only_sessions_inside_cutoff() {
        let mut state = UsageState::default();
        state.apply(
            "old",
            PathBuf::from("/old"),
            &delta(100, 50, 30, 20),
            ts(1_000),
        );
        state.apply(
            "new",
            PathBuf::from("/new"),
            &delta(200, 80, 10, 5),
            ts(5_000),
        );

        let lifetime = state.filtered_total(None);
        assert_eq!(lifetime.input_tokens, 300);
        assert_eq!(lifetime.message_count, 2);

        let recent = state.filtered_total(Some(ts(3_000)));
        assert_eq!(recent.input_tokens, 200);
        assert_eq!(recent.message_count, 1);
    }

    #[test]
    fn filtered_total_on_empty_state_returns_zeros() {
        let state = UsageState::default();
        let t = state.filtered_total(Some(ts(1_000)));
        assert_eq!(t.input_tokens, 0);
        assert_eq!(t.message_count, 0);
    }

    #[test]
    fn apply_uses_caller_supplied_timestamp() {
        let mut state = UsageState::default();
        let when = ts(1_700_000_000);
        state.apply("a", PathBuf::from("/a"), &delta(1, 0, 0, 0), when);
        assert_eq!(state.sessions["a"].last_updated, when);
    }

    #[test]
    fn apply_advances_last_updated_to_newer_timestamp() {
        let mut state = UsageState::default();
        let earlier = ts(1_700_000_000);
        let later = ts(1_700_000_500);
        state.apply("a", PathBuf::from("/a"), &delta(1, 0, 0, 0), earlier);
        state.apply("a", PathBuf::from("/a"), &delta(1, 0, 0, 0), later);
        assert_eq!(state.sessions["a"].last_updated, later);
    }

    #[test]
    fn apply_does_not_move_last_updated_backwards() {
        // Re-reading a stale tail of the same jsonl could replay an
        // older entry; that must not retro-set `last_updated`.
        let mut state = UsageState::default();
        let later = ts(1_700_000_500);
        let earlier = ts(1_700_000_000);
        state.apply("a", PathBuf::from("/a"), &delta(1, 0, 0, 0), later);
        state.apply("a", PathBuf::from("/a"), &delta(1, 0, 0, 0), earlier);
        // Latest stays at the newer value.
        assert_eq!(state.sessions["a"].last_updated, later);
        // Token counters still accumulate even when the timestamp
        // didn't advance.
        assert_eq!(state.sessions["a"].input_tokens, 2);
    }
}
