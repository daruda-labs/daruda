//! The three ceilings, and the cost accounting they compare against.
//! Each fixture hands the run a budget the others do not, which is why
//! these live apart from the flows that just run.

use super::*;

/// Deadline is an `Instant` the host computes, so a test can hand over one
/// that has already passed instead of waiting two hours.
#[test]
fn an_expired_deadline_stops_the_run_before_the_first_node() {
    let runner = FakeRunner::new();
    let budget = Budget {
        deadline: Some(std::time::Instant::now() - std::time::Duration::from_secs(1)),
        ..Budget::unlimited()
    };
    let (report, _dir) = run_with_budget(CHAIN, &runner, budget);
    assert!(
        matches!(
            report.outcome,
            RunOutcome::BudgetExhausted {
                limit: BudgetLimit::WallClock
            }
        ),
        "{:?}",
        report.outcome
    );
    assert!(runner.calls().is_empty());
}

/// The backstop that does not depend on the agent reporting anything. A
/// repair loop that keeps re-deriving is exactly what this catches, so the
/// flow here is the gated one rather than a straight chain.
#[test]
fn the_node_run_cap_counts_every_attempt_including_reruns_and_fixes() {
    let runner = FakeRunner::new().script(
        "gate",
        vec![Step::fail(NodeFailure::Exit { code: Some(1) })],
    );
    let budget = Budget {
        max_node_runs: Some(4),
        ..Budget::unlimited()
    };
    let (report, _dir) = run_with_budget(GATED, &runner, budget);
    assert!(
        matches!(
            report.outcome,
            RunOutcome::BudgetExhausted {
                limit: BudgetLimit::NodeRuns
            }
        ),
        "{:?}",
        report.outcome
    );
    assert_eq!(report.node_runs, 4, "the cap is the count, not an estimate");
    // implement, review, gate, __fix__ — the fix session counts, because it
    // is a session the run paid for.
    assert_eq!(runner.ids().len(), 4);
}

/// What the check after `call` is for. The node boundary alone would stop
/// the run one step late — after the failed gate had already paid for a fix
/// session it had no budget left for.
#[test]
fn a_budget_spent_by_a_failing_gate_stops_before_the_repair_pays_again() {
    let runner = FakeRunner::new().script(
        "gate",
        vec![Step::fail(NodeFailure::Exit { code: Some(1) })],
    );
    let budget = Budget {
        max_node_runs: Some(3),
        ..Budget::unlimited()
    };
    let (report, _dir) = run_with_budget(GATED, &runner, budget);
    assert!(
        matches!(
            report.outcome,
            RunOutcome::BudgetExhausted {
                limit: BudgetLimit::NodeRuns
            }
        ),
        "{:?}",
        report.outcome
    );
    assert_eq!(
        runner.ids(),
        vec!["implement", "review", "gate"],
        "the cap is spent, so no fix session may be started"
    );
}

/// Cost only works when the agent reports it, so a run that never sees a
/// cost must say so rather than implying the limit held.
#[test]
fn a_run_with_no_reported_cost_warns_that_the_limit_never_applied() {
    let runner = FakeRunner::new();
    let budget = Budget {
        max_cost: Some(CostLimit {
            amount: 5.0,
            currency: "USD".to_string(),
        }),
        ..Budget::unlimited()
    };
    let (report, _dir) = run_with_budget(CHAIN, &runner, budget);
    assert!(matches!(report.outcome, RunOutcome::Done));
    assert!(
        report.warnings().iter().any(|w| w.contains("cost")),
        "{:?}",
        report.warnings()
    );
}

#[test]
fn reported_cost_accumulates_across_nodes_and_trips_the_limit() {
    let runner = FakeRunner::new().cost_per_call(2.0, "USD");
    let budget = Budget {
        max_cost: Some(CostLimit {
            amount: 3.0,
            currency: "USD".to_string(),
        }),
        ..Budget::unlimited()
    };
    let (report, _dir) = run_with_budget(CHAIN, &runner, budget);
    assert!(
        matches!(
            report.outcome,
            RunOutcome::BudgetExhausted {
                limit: BudgetLimit::Cost
            }
        ),
        "{:?}",
        report.outcome
    );
}

/// Currencies are not summed. Adding 2 USD to 2 EUR and calling it 4 is
/// worse than not enforcing the limit at all.
#[test]
fn a_mixed_currency_run_warns_instead_of_summing() {
    let runner = FakeRunner::new()
        .cost_for("design", 2.0, "USD")
        .cost_for("review", 2.0, "EUR");
    let budget = Budget {
        max_cost: Some(CostLimit {
            amount: 3.0,
            currency: "USD".to_string(),
        }),
        ..Budget::unlimited()
    };
    let (report, _dir) = run_with_budget(CHAIN, &runner, budget);
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert!(
        report.warnings().iter().any(|w| w.contains("EUR")),
        "{:?}",
        report.warnings()
    );
}

/// The quieter half of the same defect: the run does have a total, it just
/// is not in the limit's currency, so comparing them would be the mistake
/// and staying silent would hide a limit that never applied.
#[test]
fn a_cost_limit_in_another_currency_warns_that_it_never_applied() {
    let runner = FakeRunner::new().cost_per_call(9.0, "EUR");
    let budget = Budget {
        max_cost: Some(CostLimit {
            amount: 3.0,
            currency: "USD".to_string(),
        }),
        ..Budget::unlimited()
    };
    let (report, _dir) = run_with_budget(CHAIN, &runner, budget);
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert!(
        report
            .warnings()
            .iter()
            .any(|w| w.contains("EUR") && w.contains("USD")),
        "{:?}",
        report.warnings()
    );
}

/// A budget is permission to start work, not a verdict on what just
/// finished. Checked after a call instead, this flow spends its last
/// allowance on `review` and then throws `review`'s result away.
#[test]
fn a_flow_that_fits_its_node_budget_exactly_finishes() {
    let runner = FakeRunner::new();
    let budget = Budget {
        max_node_runs: Some(3),
        ..Budget::unlimited()
    };
    let (report, _dir) = run_with_budget(CHAIN, &runner, budget);
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert_eq!(report.node_runs, 3);
    assert_eq!(runner.ids(), vec!["design", "test", "review"]);
}

/// One more node than the budget allows stops before that node runs, not
/// after — the run must not pay for work it has no allowance for.
#[test]
fn a_flow_one_node_over_its_budget_stops_before_paying_for_it() {
    let runner = FakeRunner::new();
    let budget = Budget {
        max_node_runs: Some(2),
        ..Budget::unlimited()
    };
    let (report, _dir) = run_with_budget(CHAIN, &runner, budget);
    assert!(
        matches!(
            report.outcome,
            RunOutcome::BudgetExhausted {
                limit: BudgetLimit::NodeRuns
            }
        ),
        "{:?}",
        report.outcome
    );
    assert_eq!(runner.ids(), vec!["design", "test"]);
}

/// A runner races only the timeout it is handed, so a run deadline shorter
/// than a node's own budget has to reach the node as its timeout — otherwise
/// a one-minute run ceiling cannot stop a ten-minute node.
#[test]
fn a_node_never_gets_longer_than_the_run_has_left() {
    /// Records the timeout each attempt was given.
    struct Timekeeper(FakeRunner, std::cell::RefCell<Vec<Duration>>);

    impl NodeRunner for Timekeeper {
        fn run_agent<'a>(
            &'a self,
            ctx: &'a RunContext<'a>,
            agent: &'a crate::model::AgentSpec,
            prompt: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
            self.1.borrow_mut().push(ctx.timeout);
            self.0.run_agent(ctx, agent, prompt)
        }

        fn run_command<'a>(
            &'a self,
            ctx: &'a RunContext<'a>,
            run: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
            self.1.borrow_mut().push(ctx.timeout);
            self.0.run_command(ctx, run)
        }
    }

    let keeper = Timekeeper(FakeRunner::new(), std::cell::RefCell::new(Vec::new()));
    let dir = tempfile::tempdir().expect("tempdir");
    let loaded = load(CHAIN).expect("valid flow");
    let budget = Budget {
        // Far less than the nodes' own 10m default.
        deadline: Some(std::time::Instant::now() + Duration::from_secs(2)),
        ..Budget::unlimited()
    };
    let _ = smol::block_on(run_flow(
        RunInputs {
            loaded: &loaded,
            flow_dir: dir.path(),
            cwd: dir.path(),
            run_dir: &dir.path().join("run"),
            cancel: &CancelToken::default(),
            budget: &budget,
            git_status: None,
            events: None,
            ask: None,
        },
        &keeper,
    ));

    for given in keeper.1.into_inner() {
        assert!(
            given <= Duration::from_secs(2),
            "a node outlived the run's deadline by its own budget: {given:?}"
        );
    }
}

/// The one ceiling no boundary check can cover: the last node crossed it,
/// and there is no next node to stop. The run still succeeded — every node
/// passed — so it says so and warns rather than reporting a failure.
#[test]
fn a_run_that_ends_over_its_cost_limit_says_so() {
    let runner = FakeRunner::new().cost_per_call(2.0, "USD");
    let budget = Budget {
        max_cost: Some(CostLimit {
            amount: 5.0,
            currency: "USD".to_string(),
        }),
        ..Budget::unlimited()
    };
    let (report, _dir) = run_with_budget(CHAIN, &runner, budget);
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "every node passed: {:?}",
        report.outcome
    );
    assert!(
        report
            .warnings()
            .iter()
            .any(|w| w.contains("more than its limit")),
        "{:?}",
        report.warnings()
    );
}
