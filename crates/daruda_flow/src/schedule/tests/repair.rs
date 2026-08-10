//! The retry and repair loops. Every test here shares the `GATED` flow,
//! because what is under test is the *sequence* a failed gate provokes,
//! not the work any node does.

use super::*;

/// The design's flagship path: the gate fails, a fix session runs, then
/// `review` re-derives the verdict and the gate passes on attempt 2.
///
/// The attempt column is asserted too, because it is what distinguishes
/// a correct generation rule from one that resets the driving node and
/// never terminates.
#[test]
fn a_repair_reruns_the_declared_root_and_the_gate() {
    let runner = FakeRunner::new().script(
        "gate",
        vec![
            Step::fail(NodeFailure::Exit { code: Some(1) }),
            Step::Ok { writes: None },
        ],
    );
    let (report, _dir) = run(GATED, &runner);
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    let seen: Vec<(String, u32)> = runner
        .calls()
        .into_iter()
        .map(|c| (c.node, c.attempt))
        .collect();
    assert_eq!(
        seen,
        vec![
            ("implement".to_string(), 1),
            ("review".to_string(), 1),
            ("gate".to_string(), 1),
            ("__fix__".to_string(), 1),
            // `review` is re-run in a fresh generation, so its own
            // counter starts over…
            ("review".to_string(), 1),
            // …while the gate driving the loop counts up, which is what
            // `max_attempts` bounds.
            ("gate".to_string(), 2),
        ]
    );
}

/// Invariant 2 made observable: the fix prompt must already name the
/// archived evidence. Reversing archive and fix passes every test that
/// only looks at call order, so the text is what pins it.
#[test]
fn the_fix_prompt_names_the_evidence_archived_before_it_ran() {
    let runner = FakeRunner::new().script(
        "gate",
        vec![
            Step::fail(NodeFailure::Exit { code: Some(1) }),
            Step::Ok { writes: None },
        ],
    );
    let (_report, _dir) = run(GATED, &runner);
    let calls = runner.calls();
    let fix = calls
        .iter()
        .find(|c| c.node == "__fix__")
        .expect("a fix ran");
    assert!(
        fix.text.contains("review.attempt-1.evidence-"),
        "the fix must see the archived review, got: {}",
        fix.text
    );
}

/// A fix that fails has changed nothing, so re-deriving the verdict
/// would only burn the attempt cap on a certain failure.
#[test]
fn a_failing_fix_session_stops_the_run_without_re_deriving() {
    let runner = FakeRunner::new()
        .script(
            "gate",
            vec![Step::fail(NodeFailure::Exit { code: Some(1) })],
        )
        .script(
            "__fix__",
            vec![Step::fail(NodeFailure::TurnFailed("no".into()))],
        );
    let (report, _dir) = run(GATED, &runner);
    assert!(
        matches!(report.outcome, RunOutcome::Failed { .. }),
        "{:?}",
        report.outcome
    );
    let ids = runner.ids();
    assert_eq!(
        ids.iter().filter(|id| *id == "review").count(),
        1,
        "review must not be re-derived after a failed fix: {ids:?}"
    );
}

/// The generation rule's actual purpose: a gate nested inside another
/// gate's rerun set gets its own cap back each time the outer one
/// retries. Nothing else in the suite covers this.
#[test]
fn a_nested_gate_gets_a_fresh_cap_each_outer_generation() {
    let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: build
    kind: agent
    output: build.md
    prompt: write
  - id: inner
    kind: command
    deps: [build]
    run: \"true\"
    on_fail:
      repair:
        fix: inner fix, see {{attempts}}
        max_attempts: 2
        wait: 0s
  - id: outer
    kind: command
    deps: [inner]
    run: \"true\"
    on_fail:
      repair:
        fix: outer fix, see {{attempts}}
        rerun: [inner]
        max_attempts: 2
        wait: 0s
";
    // `inner` fails once per generation, then succeeds — so it must be
    // asked twice in each of the outer gate's two generations.
    let runner = FakeRunner::new()
        .script(
            "inner",
            vec![
                Step::fail(NodeFailure::Exit { code: Some(1) }),
                Step::Ok { writes: None },
            ],
        )
        .script(
            "outer",
            vec![
                Step::fail(NodeFailure::Exit { code: Some(1) }),
                Step::Ok { writes: None },
            ],
        );
    let (report, _dir) = run(flow, &runner);
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    let inner_attempts: Vec<u32> = runner
        .calls()
        .into_iter()
        .filter(|c| c.node == "inner")
        .map(|c| c.attempt)
        .collect();
    assert_eq!(
        inner_attempts,
        vec![1, 2, 1, 2],
        "the inner gate's counter restarts in the outer gate's second generation"
    );
}

/// `rerun` omitted means the gate alone re-runs — not an empty set,
/// which would leave the fix with nothing to re-derive.
#[test]
fn an_omitted_rerun_still_reruns_the_gate_itself() {
    let flow = GATED.replace("        rerun: [review]\n", "");
    let runner = FakeRunner::new().script(
        "gate",
        vec![
            Step::fail(NodeFailure::Exit { code: Some(1) }),
            Step::Ok { writes: None },
        ],
    );
    let (report, _dir) = run(&flow, &runner);
    assert!(matches!(report.outcome, RunOutcome::Done));
    assert_eq!(
        runner.ids(),
        vec!["implement", "review", "gate", "__fix__", "gate"]
    );
}

/// A refusal cannot be argued with, so it halts regardless of policy.
#[test]
fn a_refusal_halts_even_when_a_repair_is_declared() {
    let runner = FakeRunner::new().script("gate", vec![Step::fail(NodeFailure::Refused)]);
    let (report, _dir) = run(GATED, &runner);
    assert!(matches!(
        report.outcome,
        RunOutcome::Failed {
            failure: NodeFailure::Refused,
            ..
        }
    ));
    assert!(
        !runner.ids().iter().any(|id| id == "__fix__"),
        "no fix session may run for a refusal"
    );
}

/// A pinned `model` the adapter does not advertise fails before the prompt
/// is ever sent, comparing two things that do not move within a run — so
/// every further attempt reaches that same point and stops there. Left
/// retryable, a node with `max_attempts` opens that many sessions and sits
/// through the waits between them to be told the same thing.
#[test]
fn a_setting_the_adapter_cannot_offer_is_not_retried() {
    let runner = FakeRunner::new().script(
        "design",
        vec![Step::fail(NodeFailure::UnsupportedSetting {
            field: "model",
            value: "opus".to_string(),
            available: vec!["sonnet".to_string()],
        })],
    );
    let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
    on_fail:
      retry:
        max_attempts: 4
        wait: 0s
        hint: it failed with {{failure}}
";
    let (report, _dir) = run(flow, &runner);

    assert!(matches!(
        report.outcome,
        RunOutcome::Failed {
            failure: NodeFailure::UnsupportedSetting { .. },
            ..
        }
    ));
    assert_eq!(
        runner.calls().len(),
        1,
        "opened more than one session for a failure a second one cannot change"
    );
}

/// The termination guarantee. If the driving node's counter were reset
/// each generation — it is in its own rerun set — this test would never
/// return, and cargo has no per-test timeout to save it.
#[test]
fn attempts_are_capped_and_the_last_failure_is_reported() {
    let runner = FakeRunner::new().script(
        "gate",
        vec![Step::fail(NodeFailure::Exit { code: Some(2) })],
    );
    let (report, _dir) = run(GATED, &runner);
    match report.outcome {
        RunOutcome::Failed { node, failure } => {
            assert_eq!(node, "gate");
            assert_eq!(failure, NodeFailure::Exit { code: Some(2) });
        }
        other => panic!("expected a failure, got {other:?}"),
    }
    let gate_runs = runner.ids().iter().filter(|id| *id == "gate").count();
    assert_eq!(gate_runs, 2, "max_attempts is 2");
}

/// An agent node's retry re-runs the node itself with its hint, one
/// session per attempt — no separate fix session.
#[test]
fn an_agent_retry_reruns_the_node_without_a_fix_session() {
    let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
    on_fail:
      retry:
        max_attempts: 2
        wait: 0s
        hint: it failed with {{failure}}
";
    let runner = FakeRunner::new().script(
        "design",
        vec![
            Step::fail(NodeFailure::TurnFailed("boom".into())),
            Step::Ok {
                writes: Some("ok\n".into()),
            },
        ],
    );
    let (report, _dir) = run(flow, &runner);
    assert!(matches!(report.outcome, RunOutcome::Done));
    let seen: Vec<(String, u32)> = runner
        .calls()
        .iter()
        .map(|c| (c.node.clone(), c.attempt))
        .collect();
    assert_eq!(
        seen,
        vec![("design".to_string(), 1), ("design".to_string(), 2)],
        "the node's own counter increases; it is not reset by its own retry"
    );
    // The two-channel rule: the retry prompt is the node's own prompt,
    // then a separator, then the rendered hint. Nothing else observes
    // that the hint was rendered at all.
    assert_eq!(
        runner.calls()[1].text,
        "write\n\n---\nit failed with the turn failed: boom"
    );
    // …and the first attempt has no hint, because there is no failure
    // to answer yet. Asserting only `calls[1]` would let an
    // implementation append an empty hint to every attempt.
    assert_eq!(runner.calls()[0].text, "write");
}

/// The `∩ executed` half of invariant 3. `docs` sits downstream of the
/// rerun root but has not run yet — driving it during the repair would
/// produce its output before the gate's verdict, and again afterwards.
#[test]
fn a_rerun_set_never_drives_a_node_that_has_not_run_yet() {
    let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: implement
    kind: agent
    output: implement.md
    prompt: write
  - id: gate
    kind: command
    deps: [implement]
    run: \"true\"
    on_fail:
      repair:
        fix: fix it, see {{attempts}}
        rerun: [implement]
        max_attempts: 2
        wait: 0s
  - id: docs
    kind: agent
    deps: [gate]
    output: docs.md
    prompt: write
";
    let runner = FakeRunner::new().script(
        "gate",
        vec![
            Step::fail(NodeFailure::Exit { code: Some(1) }),
            Step::Ok { writes: None },
        ],
    );
    let (report, _dir) = run(flow, &runner);
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert_eq!(
        runner.ids(),
        vec!["implement", "gate", "__fix__", "implement", "gate", "docs"],
        "docs runs once, after the gate finally passes"
    );
}

/// A retry must not inherit the previous attempt's half-written file.
/// Attempt 1 writes a partial output and fails; attempt 2 writes
/// nothing. Without archiving, the stale file makes attempt 2 pass.
#[test]
fn a_retry_does_not_inherit_the_previous_attempts_partial_output() {
    let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
    on_fail:
      retry:
        max_attempts: 2
        wait: 0s
        hint: retry after {{failure}}
";
    let runner = FakeRunner::new().script(
        "design",
        vec![
            Step::Fail {
                writes: Some("half a thou".to_string()),
                failure: NodeFailure::ContextExhausted,
            },
            Step::Ok { writes: None },
        ],
    );
    let (report, _dir) = run(flow, &runner);
    assert!(
        matches!(
            report.outcome,
            RunOutcome::Failed {
                failure: NodeFailure::NoOutput { .. },
                ..
            }
        ),
        "attempt 2 wrote nothing, so the stale file must not carry it: {:?}",
        report.outcome
    );
}

/// `wait` is parsed, defaulted and clamped by `resolve`; if the
/// scheduler ignores it the field is dead config and no other test
/// notices, because every other flow here sets `0s`.
#[test]
fn a_non_zero_wait_elapses_between_attempts() {
    let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: gate
    kind: command
    run: \"true\"
    on_fail:
      repair:
        fix: see {{attempts}}
        max_attempts: 2
        wait: 120ms
";
    let runner = FakeRunner::new().script(
        "gate",
        vec![Step::fail(NodeFailure::Exit { code: Some(1) })],
    );
    let started = std::time::Instant::now();
    let (_report, _dir) = run(flow, &runner);
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(100),
        "one 120ms wait separates the two attempts, got {:?}",
        started.elapsed()
    );
}
