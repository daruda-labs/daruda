//! What a run spends and what bounds it: the three ceilings, the cost the
//! agents reported, and the report all of it is folded into.
//!
//! Separate from the drive loop because these answer one question — may the
//! run go on, and what did it cost — while the loop next door answers a
//! different one about a single node.
//!
//! The state those questions need lives in [`Accounting`] rather than on
//! `Run` beside the drive loop's own. Splitting the *file* alone left every
//! cell reachable from everywhere; a counter that only `account` may raise
//! is worth more than a counter anyone may.

use super::{BudgetLimit, Run, RunOutcome, RunReport};
use crate::record::NodeRecord;
use crate::request::{Budget, CostLimit};
use crate::runner::RunResult;
use std::cell::{Cell, RefCell};
use std::time::Duration;

/// What the run has spent, and what it has to say about it.
///
/// Interior mutability for the same reason the rest of `Run` has it: a
/// gate's repair records from inside a nested drive. Every field is private
/// — the only ways in are [`Accounting::account`] and
/// [`Accounting::warn`], which is what keeps "what the budget counts" and
/// "what the run actually paid for" from drifting apart.
#[derive(Default)]
pub(super) struct Accounting {
    /// Runner calls made so far — what `max_node_runs` bounds.
    node_runs: Cell<u32>,
    /// Time the run spent waiting for a person, summed over every call.
    ///
    /// The wall-clock ceiling is an absolute `Instant`, so a node clock
    /// that stops does nothing for it: without this, granting a permission
    /// after a long think would end the run at the next node boundary.
    /// Raised only by [`Accounting::account`], like `node_runs` — the
    /// budget's own rule.
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
    pub(super) fn warn(&self, message: String) {
        self.warnings.borrow_mut().push(message);
    }

    pub(super) fn record(&self, record: impl FnOnce(&mut Vec<NodeRecord>)) {
        record(&mut self.records.borrow_mut());
    }

    /// Every runner call passes through here, so what the budget counts
    /// cannot drift from the sessions the run actually paid for.
    pub(super) fn account(&self, result: &RunResult) {
        self.node_runs.set(self.node_runs.get() + 1);
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
            .is_some_and(|max| self.node_runs.get() >= max)
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
            self.node_runs.get(),
            self.cost.into_inner(),
            self.warnings.into_inner(),
            self.records.into_inner(),
        )
    }
}

impl Run<'_> {
    pub(super) fn finish(self, outcome: RunOutcome) -> RunReport {
        self.spent
            .finish(outcome, self.run_dir.to_path_buf(), self.budget)
    }

    pub(super) fn account(&self, result: &RunResult) {
        self.spent.account(result);
    }

    pub(super) fn budget_exhausted(&self) -> Option<BudgetLimit> {
        self.spent.exhausted(self.budget)
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
            outcome: Ok(()),
            artifacts: Vec::new(),
            usage: None,
            waiting: crate::runner::Waiting {
                total: parked,
                answers: Vec::new(),
            },
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

        spent.account(&call(Duration::from_secs(60)));
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
        spent.account(&call(Duration::from_secs(5)));
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
        spent.account(&call(Duration::from_secs(60)));
        assert!(spent.deadline(&Budget::unlimited()).is_none());
        assert!(spent.exhausted(&Budget::unlimited()).is_none());
    }

    /// `account` is the only raiser, and it is the funnel every runner call
    /// already passes through — so what the budget forgives cannot drift
    /// from what the run actually waited.
    #[test]
    fn every_call_s_waiting_adds_up() {
        let spent = Accounting::default();
        for _ in 0..3 {
            spent.account(&call(Duration::from_secs(10)));
        }
        let budget = deadline_in(Duration::from_secs(25), true);
        assert!(
            spent.exhausted(&budget).is_none(),
            "three ten-second waits did not add to thirty"
        );
        // And the same calls still count against the run-count ceiling: a
        // parked call is a call.
        let counted = Budget {
            max_node_runs: Some(3),
            ..Budget::unlimited()
        };
        assert!(matches!(
            spent.exhausted(&counted),
            Some(BudgetLimit::NodeRuns)
        ));
    }
}
