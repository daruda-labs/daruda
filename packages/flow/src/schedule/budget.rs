//! What a run spends and what bounds it: the three ceilings, the cost the
//! agents reported, and the report all of it is folded into.
//!
//! Separate from the drive loop because these answer one question — may the
//! run go on, and what did it cost — while the loop next door answers a
//! different one about a single node.
//!
//! The state those questions need lives in [`Accounting`] rather than on
//! `Run` beside the drive loop's own. Splitting the *file* alone left every
//! cell reachable from everywhere; a counter that only the accounted-call
//! path and the correction reservation may raise is worth more than a
//! counter anyone may.

use super::{BudgetLimit, Run, RunOutcome, RunReport};
use crate::record::NodeRecord;
use crate::request::{Budget, CostLimit};
use crate::runner::RunResult;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::time::Duration;

/// What the run has spent, and what it has to say about it.
///
/// Interior mutability for the same reason the rest of `Run` has it: a
/// gate's repair records from inside a nested drive. Every field is private
/// — the only ways into its totals are the accounted-call path, the
/// correction reservation and [`Accounting::warn`]. That is what keeps
/// "what the budget counts" and "what the run actually consumed" from
/// drifting apart.
#[derive(Default)]
pub(super) struct Accounting {
    /// Budget units consumed so far — what `max_node_runs` bounds. Every
    /// runner call consumes one, and every turn past its first reserves
    /// another — so a node allowed five turns can take five, not two.
    budget_units: Cell<u32>,
    /// Time the run spent waiting for a person, summed over every call.
    ///
    /// The wall-clock ceiling is an absolute `Instant`, so a node clock
    /// that stops does nothing for it: without this, granting a permission
    /// after a long think would end the run at the next node boundary.
    /// Raised only by [`Accounting::account_result`] — the budget's own rule.
    parked: Cell<Duration>,
    /// The run's reported cost so far, in the first currency seen.
    cost: RefCell<Option<CostLimit>>,
    /// Set once a second currency appears: the total stops being a total,
    /// so it stops accumulating and the warning is issued once.
    cost_mixed: Cell<bool>,
    warnings: RefCell<Vec<String>>,
    /// Every attempt so far.
    records: RefCell<Vec<NodeRecord>>,
}

impl Accounting {
    /// Start from what an earlier process had already spent.
    ///
    /// Cost and budget units carry: they are money and work that really
    /// happened, and a resume that forgot them would let a `max_node_runs`
    /// of 50 run 50 more. **`parked` does not carry.** It exists only to
    /// push out a wall-clock deadline that was set before the waiting
    /// happened, and a resumed run is given a fresh deadline measured from
    /// now — adding the old waiting to it would hand the run extra time for
    /// a wait that is already behind it.
    pub(super) fn resumed(spent: crate::journal::Spent, records: Vec<NodeRecord>) -> Self {
        Self {
            budget_units: Cell::new(spent.node_runs),
            parked: Cell::new(Duration::ZERO),
            cost: RefCell::new(spent.cost),
            cost_mixed: Cell::new(spent.cost_mixed),
            warnings: RefCell::new(spent.warnings),
            // Carried so the finished run's account covers both halves. A
            // resume that started its record afresh would leave `run.md`
            // beginning in the middle of the run it describes.
            records: RefCell::new(records),
        }
    }

    pub(super) fn warn(&self, message: String) {
        self.warnings.borrow_mut().push(message);
    }

    /// What the run has spent so far, for the journal. A copy, not a
    /// handle: the cells stay private, which is the whole reason the
    /// counters live here.
    pub(super) fn spent(&self) -> crate::journal::Spent {
        crate::journal::Spent {
            node_runs: self.budget_units.get(),
            parked: self.parked.get(),
            cost: self.cost.borrow().clone(),
            cost_mixed: self.cost_mixed.get(),
            warnings: self.warnings.borrow().clone(),
        }
    }

    pub(super) fn record(&self, record: impl FnOnce(&mut Vec<NodeRecord>)) {
        record(&mut self.records.borrow_mut());
    }

    /// Charge the budget unit every runner call consumes.
    fn charge_call(&self) {
        self.budget_units.set(self.budget_units.get() + 1);
    }

    /// Fold in the figures known only once a runner call has settled.
    fn account_result(&self, result: &RunResult) {
        self.parked.set(self.parked.get() + result.waiting.total);

        let Some(cost) = result.usage.as_ref().and_then(|u| u.cost.as_ref()) else {
            return;
        };
        if self.cost_mixed.get() {
            return;
        }
        // Each node is its own session, so a session's last reported cost is
        // that node's total and the run's is their sum. A second currency
        // means the sum is meaningless — stop accumulating and say so,
        // rather than adding numbers that do not add.
        let mut total = self.cost.borrow_mut();
        match total.as_mut() {
            Some(total) if total.currency == cost.currency => total.amount += cost.amount,
            Some(total) => {
                self.cost_mixed.set(true);
                let message = format!(
                    "costs were reported in both `{}` and `{}`; only the `{}` total is counted, \
                     so a cost limit does not cover the rest",
                    total.currency, cost.currency, total.currency
                );
                // Not `self.warn`: `cost` is already borrowed mutably here.
                self.warnings.borrow_mut().push(message);
            }
            None => {
                *total = Some(CostLimit {
                    amount: cost.amount,
                    currency: cost.currency.clone(),
                });
            }
        }
    }

    /// Permission to spend one more budget unit on a correction turn inside
    /// a call already under way.
    ///
    /// **A reservation, not a question** — granting it raises the count here
    /// and now. The call itself was already charged by
    /// [`Accounting::charge_call`], so the ceiling covers both units and
    /// concurrent calls cannot spend the same remaining allowance. Asking
    /// [`Accounting::exhausted`] rather than the unit count alone is what
    /// makes the wall clock and the cost ceiling cover a correction too.
    fn try_reserve_extra_turn(&self, budget: &Budget) -> bool {
        if self.exhausted(budget).is_some() {
            return false;
        }
        self.budget_units.set(self.budget_units.get() + 1);
        true
    }

    /// Two currencies are not comparable, so a total reported in another
    /// currency leaves the limit unenforced rather than wrongly tripped;
    /// [`Accounting::finish`] is what makes that visible.
    fn cost_exhausted(&self, budget: &Budget) -> bool {
        let Some(limit) = budget.max_cost.as_ref() else {
            return false;
        };
        self.cost
            .borrow()
            .as_ref()
            .is_some_and(|total| total.currency == limit.currency && total.amount >= limit.amount)
    }

    /// The run's expiry, pushed out by whatever it spent waiting for a
    /// person. A budget bounds work, and waiting is the absence of it.
    pub(super) fn deadline(&self, budget: &Budget) -> Option<std::time::Instant> {
        budget.deadline.map(|deadline| deadline + self.parked.get())
    }

    /// The defence that tripped, if any.
    fn exhausted(&self, budget: &Budget) -> Option<BudgetLimit> {
        // Comparing against the host's expiry, never building one: a
        // deadline the engine invented could not be tested in under 2h.
        if self
            .deadline(budget)
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            return Some(BudgetLimit::WallClock);
        }
        if budget
            .max_node_runs
            .is_some_and(|max| self.budget_units.get() >= max)
        {
            return Some(BudgetLimit::NodeRuns);
        }
        self.cost_exhausted(budget).then_some(BudgetLimit::Cost)
    }

    /// Turn the accounting into the report, adding the warnings that can
    /// only be known once the run is over: a cost limit nothing ever
    /// reported against is a limit the user believed was protecting them.
    fn finish(
        self,
        outcome: RunOutcome,
        run_dir: std::path::PathBuf,
        budget: &Budget,
        provenance: super::report::Provenance,
    ) -> RunReport {
        // A ceiling the last node crossed is a fact about what the run
        // spent, and every other check asks "may I start more work" — after
        // the last node there is none to ask about, so this is the one case
        // a cost limit would otherwise never cover. It warns rather than
        // failing: the nodes all passed, and calling that a failure would
        // misreport the run to undo a spend that already happened.
        if self.cost_exhausted(budget)
            && let Some(limit) = &budget.max_cost
        {
            self.warn(format!(
                "the run cost more than its limit of {} {}",
                limit.amount, limit.currency
            ));
        }
        if let Some(limit) = &budget.max_cost {
            match self.cost.borrow().as_ref() {
                None => self.warn(format!(
                    "no agent reported a cost, so the limit of {} {} never applied",
                    limit.amount, limit.currency
                )),
                Some(total) if total.currency != limit.currency => self.warn(format!(
                    "costs were reported in `{}` but the limit is {} `{}`, so it never applied",
                    total.currency, limit.amount, limit.currency
                )),
                Some(_) => {}
            }
        }
        RunReport::completed(
            outcome,
            run_dir,
            self.budget_units.get(),
            self.cost.into_inner(),
            self.warnings.into_inner(),
            self.records.into_inner(),
            provenance,
        )
    }
}

impl Run<'_> {
    pub(super) fn finish(self, outcome: RunOutcome) -> RunReport {
        // The account is **not** reordered, and that is deliberate.
        //
        // It grows as nodes settle, so it reads in the order things
        // happened. Sorting it by the file would misreport a run where two
        // nodes really did overlap, and would move the repair session — a
        // synthetic id that is in no graph — away from the gate whose
        // failure explains it.
        self.spent.finish(
            outcome,
            self.run_dir.to_path_buf(),
            self.budget,
            super::report::Provenance {
                profile: self.profile.clone(),
                until: self.until.clone(),
                pinned: self.pinned.clone(),
                carried_over: self.already_passed.len(),
            },
        )
    }

    /// Run one scheduler call through the complete accounting path. The
    /// closure matters: constructing a runner future may execute synchronous
    /// code, so it must happen only after this call's budget unit is charged.
    pub(super) async fn accounted_call<F, Fut>(&self, call: F) -> RunResult
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = RunResult>,
    {
        self.spent.charge_call();
        let result = call().await;
        self.spent.account_result(&result);
        result
    }

    pub(super) fn budget_exhausted(&self) -> Option<BudgetLimit> {
        self.spent.exhausted(self.budget)
    }

    /// What a runner asks through `RunContext::reserve_extra_turn`. Charged
    /// on the spot — see [`budget::Accounting::try_reserve_extra_turn`].
    ///
    /// [`budget::Accounting::try_reserve_extra_turn`]: Accounting::try_reserve_extra_turn
    pub(super) fn reserve_extra_turn(&self) -> bool {
        self.spent.try_reserve_extra_turn(self.budget)
    }

    /// A node's timeout, clipped to what is left of the run's deadline. The
    /// node then reports a `Timeout`, which is honest — it ran out of time,
    /// and the boundary check that follows names the run's reason.
    pub(super) fn bounded_timeout(&self, node: Duration) -> Duration {
        // The run's own deadline, already pushed out by what earlier calls
        // spent waiting — clipping against the raw one would hand the next
        // node a zero budget after a long approval.
        let Some(deadline) = self.spent.deadline(self.budget) else {
            return node;
        };
        node.min(deadline.saturating_duration_since(std::time::Instant::now()))
    }

    /// Asked only where the run is about to begin another unit of work — a
    /// budget is permission to start, not a verdict on what just finished.
    /// Checked after a call instead, `max_node_runs: 3` would throw away the
    /// third node's result and fail a flow that fits exactly.
    ///
    /// Cancel comes first: an explicit stop is a more accurate reason than a
    /// limit that expired in the same moment.
    pub(super) fn stop_before_more_work(&self) -> Option<RunOutcome> {
        if self.cancel.is_canceled() {
            return Some(RunOutcome::Canceled { node: None });
        }
        self.budget_exhausted()
            .map(|limit| RunOutcome::BudgetExhausted { limit })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn call(parked: Duration) -> RunResult {
        RunResult {
            tools: Vec::new(),
            outcome: Ok(()),
            artifacts: Vec::new(),
            usage: None,
            waiting: crate::runner::Waiting {
                total: parked,
                answers: Vec::new(),
            },
            turns: 1,
        }
    }

    fn capped(runs: u32) -> Budget {
        Budget {
            max_node_runs: Some(runs),
            ..Budget::unlimited()
        }
    }

    fn deadline_in(offset: Duration, past: bool) -> Budget {
        let now = Instant::now();
        Budget {
            deadline: Some(if past { now - offset } else { now + offset }),
            ..Budget::unlimited()
        }
    }

    /// **The run ceiling is an absolute `Instant`**, so the node clock
    /// stopping while a person thinks does nothing for it. Unless the run's
    /// own deadline moves by the same waiting, granting a permission after a
    /// long think ends the run at the very next boundary — the node survives
    /// its wait and the run dies of it.
    ///
    /// Stated against the accounting rather than through a run: what makes
    /// this correct is arithmetic, and a full run with a fake runner returns
    /// too fast for any wall clock to prove it either way.
    #[test]
    fn waiting_for_a_person_pushes_the_run_deadline_out() {
        let spent = Accounting::default();
        let budget = deadline_in(Duration::from_secs(30), true);
        assert!(
            matches!(spent.exhausted(&budget), Some(BudgetLimit::WallClock)),
            "a deadline half an hour gone must trip on its own"
        );

        spent.account_result(&call(Duration::from_secs(60)));
        assert!(
            spent.exhausted(&budget).is_none(),
            "an hour of waiting did not move a deadline thirty seconds past"
        );
    }

    /// The ceiling is moved, not removed: waiting less than the overrun
    /// leaves the run over.
    #[test]
    fn waiting_less_than_the_overrun_still_ends_the_run() {
        let spent = Accounting::default();
        spent.account_result(&call(Duration::from_secs(5)));
        let budget = deadline_in(Duration::from_secs(30), true);
        assert!(matches!(
            spent.exhausted(&budget),
            Some(BudgetLimit::WallClock)
        ));
    }

    /// A run with no ceiling has nothing to move, and waiting must not
    /// invent one.
    #[test]
    fn a_run_without_a_deadline_never_gets_one() {
        let spent = Accounting::default();
        spent.account_result(&call(Duration::from_secs(60)));
        assert!(spent.deadline(&Budget::unlimited()).is_none());
        assert!(spent.exhausted(&Budget::unlimited()).is_none());
    }

    /// `account_result` is the funnel every settled runner call passes
    /// through, so what the budget forgives cannot drift from what the run
    /// actually waited.
    #[test]
    fn every_call_s_waiting_adds_up() {
        let spent = Accounting::default();
        for _ in 0..3 {
            spent.charge_call();
            spent.account_result(&call(Duration::from_secs(10)));
        }
        let budget = deadline_in(Duration::from_secs(25), true);
        assert!(
            spent.exhausted(&budget).is_none(),
            "three ten-second waits did not add to thirty"
        );
        // And the same calls still count against the run-count ceiling: a
        // parked call is a call.
        assert!(matches!(
            spent.exhausted(&capped(3)),
            Some(BudgetLimit::NodeRuns)
        ));
    }

    /// A reservation is spent the moment it is granted, so two callers
    /// cannot both be told yes on the strength of one remaining turn. A
    /// predicate — "is the budget exhausted?" — consumes nothing and grants
    /// both, which is the parallel double-spend.
    #[test]
    fn a_reservation_is_consumed_so_one_spare_turn_is_granted_once() {
        let spent = Accounting::default();
        spent.charge_call();
        spent.account_result(&call(Duration::ZERO));
        let budget = capped(2);

        assert!(spent.try_reserve_extra_turn(&budget), "one turn was left");
        assert!(
            !spent.try_reserve_extra_turn(&budget),
            "the same turn was granted twice"
        );
        assert_eq!(spent.spent().node_runs, 2, "the grant is what raises it");
    }

    /// A correction asks from inside the call that spent the first turn. If
    /// that in-flight turn is absent from the count, a cap of one grants two.
    #[test]
    fn an_in_flight_first_turn_leaves_no_room_for_a_correction() {
        let spent = Accounting::default();
        spent.charge_call();

        assert!(!spent.try_reserve_extra_turn(&capped(1)));
        assert_eq!(spent.spent().node_runs, 1, "a refusal charges nothing");
    }

    /// Both turns are counted before they start. Settling the call adds its
    /// waiting and cost without charging either turn again.
    #[test]
    fn a_reserved_turn_is_counted_alongside_the_call_that_spent_it() {
        let spent = Accounting::default();
        let budget = capped(2);
        spent.charge_call();
        assert!(spent.try_reserve_extra_turn(&budget));
        spent.account_result(&call(Duration::ZERO));
        assert_eq!(spent.spent().node_runs, 2, "one call, two turns");
        assert!(matches!(
            spent.exhausted(&budget),
            Some(BudgetLimit::NodeRuns)
        ));
    }

    /// One rule for all three ceilings: the wall clock refuses a correction
    /// the same way a spent run count does.
    #[test]
    fn an_expired_deadline_refuses_a_correction_too() {
        let spent = Accounting::default();
        assert!(!spent.try_reserve_extra_turn(&deadline_in(Duration::from_secs(30), true)));
        assert_eq!(spent.spent().node_runs, 0, "a refusal charges nothing");
    }
}
