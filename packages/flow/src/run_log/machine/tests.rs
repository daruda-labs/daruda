use super::*;
use crate::NodeId;
use crate::run_log::line::{AnswerLine, AskLine, AttemptLine, EndLine, FailureLine, SpentLine};
use crate::runner::AskOwner;
use daruda_runlog::Machine as _;
use daruda_runlog::testing::{assert_machine_laws, enumerate_crashes};

fn id(name: &str) -> NodeId {
    NodeId::from(name)
}

fn started() -> FlowRunEvent {
    FlowRunEvent::RunStarted {
        nodes: vec![
            NodeStart {
                id: id("design"),
                produces: Produces::File,
            },
            NodeStart {
                id: id("gate"),
                produces: Produces::Nothing,
            },
        ],
        until: None,
        pinned: Vec::new(),
        profile: None,
    }
}

fn line(attempt: u32, evidence: u32) -> Box<AttemptLine> {
    Box::new(AttemptLine {
        attempt,
        evidence,
        at_ms: 0,
        took_ms: 0,
        waited_ms: 0,
        answers: Vec::new(),
        turns: 1,
        git_status: None,
        invalidated: Vec::new(),
        archived: Vec::new(),
        spent: SpentLine::default(),
    })
}

fn ask(node: &str, generation: u32, attempt: u32) -> FlowRunEvent {
    FlowRunEvent::ask_raised(
        AskOwner::Node {
            node: id(node),
            generation,
            attempt,
        },
        AskLine {
            tool: "write".to_string(),
            detail: None,
        },
    )
}

fn spawned(node: &str, evidence: u32) -> FlowRunEvent {
    FlowRunEvent::ProcessSpawned {
        node: id(node),
        evidence,
        leader: 4242,
        identity: 7,
    }
}

/// Commit `events` in order through `check`, `assign` and `apply`, the way
/// the log does, and return the state and the events as assigned.
fn run(events: Vec<FlowRunEvent>) -> (FlowRunState, Vec<FlowRunEvent>) {
    let mut state = FlowRunState::default();
    let mut assigned = Vec::new();
    for mut event in events {
        FlowRunMachine::check(&state, &event)
            .unwrap_or_else(|e| panic!("{event:?} was refused: {e}"));
        FlowRunMachine::assign(&state, &mut event);
        FlowRunMachine::apply(&mut state, &event);
        assigned.push(event);
    }
    (state, assigned)
}

/// A run with `design` passed and `gate` mid-attempt, a fix and a question
/// open: every precondition has something to be checked against.
fn busy() -> FlowRunState {
    run(vec![
        started(),
        FlowRunEvent::attempt_started(id("design"), 0),
        FlowRunEvent::AttemptPassed {
            node: id("design"),
            record: line(1, 1),
        },
        FlowRunEvent::attempt_started(id("gate"), 0),
        ask("gate", 0, 1),
        FlowRunEvent::fix_started(id("gate")),
    ])
    .0
}

fn samples() -> Vec<FlowRunEvent> {
    vec![
        started(),
        FlowRunEvent::Resumed { carried: 1 },
        FlowRunEvent::attempt_started(id("design"), 0),
        FlowRunEvent::attempt_started(id("gate"), 0),
        FlowRunEvent::attempt_started(id("gate"), 7),
        FlowRunEvent::node_refused(id("design"), FailureLine::Refused),
        spawned("gate", 2),
        spawned("gate", 9),
        FlowRunEvent::AttemptPassed {
            node: id("gate"),
            record: line(1, 2),
        },
        FlowRunEvent::AttemptFailed {
            node: id("gate"),
            failure: FailureLine::Exit { code: Some(1) },
            record: line(1, 2),
        },
        FlowRunEvent::AttemptCanceled {
            node: id("gate"),
            record: line(1, 2),
        },
        FlowRunEvent::OutputLost { node: id("design") },
        FlowRunEvent::Invalidated {
            gate: id("gate"),
            members: vec![id("design")],
        },
        FlowRunEvent::RerunScheduled {
            gate: id("gate"),
            members: vec![id("design")],
        },
        FlowRunEvent::fix_started(id("gate")),
        FlowRunEvent::FixEnded {
            gate: id("gate"),
            outcome: FixOutcome::Passed,
            record: line(1, 3),
        },
        ask("gate", 0, 1),
        FlowRunEvent::ask_raised(
            AskOwner::Fix { gate: id("gate") },
            AskLine {
                tool: "edit".to_string(),
                detail: None,
            },
        ),
        FlowRunEvent::AskAnswered {
            id: 1,
            answer: AnswerLine::Allowed,
        },
        FlowRunEvent::AskAbandoned {
            id: 1,
            reason: AbandonReason::AppRestart,
        },
        FlowRunEvent::RunEnded { end: EndLine::Done },
        FlowRunEvent::RunEnded {
            end: EndLine::Stalled {
                nodes: vec![id("gate")],
            },
        },
    ]
}

#[test]
fn the_machine_obeys_the_run_log_laws() {
    let ended = run(vec![
        started(),
        FlowRunEvent::RunEnded {
            end: EndLine::Stalled {
                nodes: vec![id("gate")],
            },
        },
    ])
    .0;
    let states = vec![
        FlowRunState::default(),
        run(vec![started()]).0,
        busy(),
        ended,
    ];
    assert_machine_laws::<FlowRunMachine>(&states, &samples());
}

/// Every crash point of a run with a repair and a question reopens to a
/// committed prefix and goes on.
#[test]
fn every_crash_point_reopens_to_a_committed_prefix() {
    let scenario = vec![
        started(),
        FlowRunEvent::attempt_started(id("design"), 0),
        FlowRunEvent::AttemptPassed {
            node: id("design"),
            record: line(1, 1),
        },
        FlowRunEvent::attempt_started(id("gate"), 0),
        ask("gate", 0, 1),
        FlowRunEvent::AskAnswered {
            id: 1,
            answer: AnswerLine::Allowed,
        },
        FlowRunEvent::Invalidated {
            gate: id("gate"),
            members: vec![id("design")],
        },
        FlowRunEvent::AttemptFailed {
            node: id("gate"),
            failure: FailureLine::Exit { code: Some(1) },
            record: line(1, 2),
        },
        FlowRunEvent::fix_started(id("gate")),
        FlowRunEvent::FixEnded {
            gate: id("gate"),
            outcome: FixOutcome::Passed,
            record: line(1, 3),
        },
        FlowRunEvent::RerunScheduled {
            gate: id("gate"),
            members: vec![id("design")],
        },
        FlowRunEvent::attempt_started(id("design"), 1),
        FlowRunEvent::AttemptPassed {
            node: id("design"),
            record: line(1, 4),
        },
        FlowRunEvent::RunEnded { end: EndLine::Done },
    ];
    assert!(enumerate_crashes::<FlowRunMachine>(scenario).cases > 0);
}

/// **B2.** Two starts proposed together get distinct evidence numbers, and
/// the numbers come from the log, not the proposer.
#[test]
fn concurrent_starts_are_numbered_by_the_log() {
    let (state, events) = run(vec![
        started(),
        FlowRunEvent::attempt_started(id("design"), 0),
        FlowRunEvent::attempt_started(id("gate"), 0),
    ]);
    let numbers: Vec<(u32, u32)> = events
        .iter()
        .filter_map(|event| match event {
            FlowRunEvent::AttemptStarted {
                attempt, evidence, ..
            } => Some((*attempt, *evidence)),
            _ => None,
        })
        .collect();
    assert_eq!(numbers, vec![(1, 1), (1, 2)]);
    assert_eq!(state.next_evidence, 3);
}

/// **B1.** A rerun starts a new generation: its attempt counts from 1, and
/// a start proposed for the old generation is refused.
#[test]
fn a_rerun_starts_a_generation_at_attempt_one() {
    let (state, events) = run(vec![
        started(),
        FlowRunEvent::attempt_started(id("design"), 0),
        FlowRunEvent::AttemptPassed {
            node: id("design"),
            record: line(1, 1),
        },
        FlowRunEvent::Invalidated {
            gate: id("gate"),
            members: vec![id("design")],
        },
        FlowRunEvent::RerunScheduled {
            gate: id("gate"),
            members: vec![id("design")],
        },
        FlowRunEvent::attempt_started(id("design"), 1),
    ]);
    assert!(matches!(
        events.last(),
        Some(FlowRunEvent::AttemptStarted {
            generation: 1,
            attempt: 1,
            evidence: 2,
            ..
        })
    ));
    let design = state.node(&id("design")).expect("design");
    assert_eq!((design.generation, design.attempts), (1, 1));

    let mut stale = state.clone();
    FlowRunMachine::apply(
        &mut stale,
        &FlowRunEvent::AttemptPassed {
            node: id("design"),
            record: line(1, 2),
        },
    );
    assert!(matches!(
        FlowRunMachine::check(&stale, &FlowRunEvent::attempt_started(id("design"), 0)),
        Err(FlowRunRejected::WrongGeneration { .. })
    ));
}

/// **F10, D4-2.** A resume turns the attempt in flight into an interrupted
/// one that keeps its number but spends no try, closes every question, and
/// lets the next start continue the numbers.
#[test]
fn a_resume_interrupts_closes_questions_and_continues_numbers() {
    let (crashed, _) = run(vec![
        started(),
        FlowRunEvent::attempt_started(id("gate"), 0),
        ask("gate", 0, 1),
    ]);
    assert!(
        matches!(
            FlowRunMachine::check(&crashed, &FlowRunEvent::Resumed { carried: 0 }),
            Err(FlowRunRejected::OpenAsks(ids)) if ids == vec![1]
        ),
        "a resume must close the questions first"
    );

    let mut events = vec![
        started(),
        FlowRunEvent::attempt_started(id("gate"), 0),
        ask("gate", 0, 1),
        FlowRunEvent::AskAbandoned {
            id: 1,
            reason: AbandonReason::AppRestart,
        },
        FlowRunEvent::Resumed { carried: 0 },
    ];
    let (resumed, _) = run(events.clone());
    let gate = resumed.node(&id("gate")).expect("gate");
    assert_eq!(gate.last, AttemptState::Interrupted { attempt: 1 });
    assert_eq!(gate.attempts, 1, "the interrupted attempt keeps its number");
    assert_eq!(gate.spent(), 0, "a crash spends none of the node's tries");
    assert!(resumed.open_asks.is_empty());

    events.push(FlowRunEvent::attempt_started(id("gate"), 0));
    events.push(ask("gate", 0, 2));
    let (_, assigned) = run(events);
    assert!(matches!(
        &assigned[assigned.len() - 2],
        FlowRunEvent::AttemptStarted {
            attempt: 2,
            evidence: 2,
            ..
        }
    ));
    assert!(
        matches!(assigned.last(), Some(FlowRunEvent::AskRaised { id: 2, .. })),
        "a question after a resume takes the next id (B4)"
    );
}

/// **B5.** A run cannot record its end with a question still open.
#[test]
fn a_run_ends_only_with_no_open_question() {
    let (state, _) = run(vec![
        started(),
        FlowRunEvent::attempt_started(id("gate"), 0),
        ask("gate", 0, 1),
    ]);
    assert!(matches!(
        FlowRunMachine::check(&state, &FlowRunEvent::RunEnded { end: EndLine::Done }),
        Err(FlowRunRejected::OpenAsks(_))
    ));
}

/// **B3.** A repair's fix asks as its gate, and only while it is running.
#[test]
fn a_fix_asks_as_its_gate_only_while_it_runs() {
    let fix_ask = FlowRunEvent::ask_raised(
        AskOwner::Fix { gate: id("gate") },
        AskLine {
            tool: "edit".to_string(),
            detail: None,
        },
    );
    let (idle, _) = run(vec![started()]);
    assert!(matches!(
        FlowRunMachine::check(&idle, &fix_ask),
        Err(FlowRunRejected::NoFix(_))
    ));
    let (fixing, _) = run(vec![started(), FlowRunEvent::fix_started(id("gate"))]);
    assert!(FlowRunMachine::check(&fixing, &fix_ask).is_ok());
}

/// **M9.** A pin starts out passed, with no attempt, and is not an output a
/// resume can declare lost.
#[test]
fn a_pin_counts_as_passed_and_cannot_be_lost() {
    let (state, _) = run(vec![FlowRunEvent::RunStarted {
        nodes: vec![NodeStart {
            id: id("design"),
            produces: Produces::File,
        }],
        until: None,
        pinned: vec![id("design")],
        profile: None,
    }]);
    let design = state.node(&id("design")).expect("design");
    assert_eq!(design.last, AttemptState::Pinned);
    assert!(state.finished().contains(&id("design")));
    assert!(matches!(
        FlowRunMachine::check(&state, &FlowRunEvent::OutputLost { node: id("design") }),
        Err(FlowRunRejected::NotPassed(_))
    ));
}

/// An older build's pass has no attempt in this log, and says so rather
/// than claiming an attempt 0; it still counts and can still be lost.
#[test]
fn an_imported_pass_has_no_attempt() {
    let (state, _) = run(vec![FlowRunEvent::Imported(Box::new(Imported {
        nodes: vec![NodeStart {
            id: id("design"),
            produces: Produces::File,
        }],
        until: None,
        pinned: Vec::new(),
        profile: None,
        passed: vec![id("design")],
        next_evidence: 2,
        spent: SpentLine::default(),
        records: Vec::new(),
        carried: Vec::new(),
    }))]);
    let design = state.node(&id("design")).expect("design");
    assert_eq!(design.last, AttemptState::Passed { attempt: None });
    assert!(state.finished().contains(&id("design")));
    assert!(
        FlowRunMachine::check(&state, &FlowRunEvent::OutputLost { node: id("design") }).is_ok()
    );
}

/// **§5.5 step 6.** Only an agent node leaves an output a resume can find
/// gone; a passed command has none to lose.
#[test]
fn a_command_cannot_lose_an_output() {
    let (state, _) = run(vec![
        started(),
        FlowRunEvent::attempt_started(id("gate"), 0),
        FlowRunEvent::AttemptPassed {
            node: id("gate"),
            record: line(1, 1),
        },
    ]);
    assert!(matches!(
        FlowRunMachine::check(&state, &FlowRunEvent::OutputLost { node: id("gate") }),
        Err(FlowRunRejected::NoOutput(_))
    ));
}

/// **D4-2.** A rerun's generation spends its own tries: an attempt a crash
/// interrupted in an earlier generation does not discount the new one.
#[test]
fn a_rerun_resets_what_a_crash_did_not_spend() {
    let (state, _) = run(vec![
        started(),
        FlowRunEvent::attempt_started(id("design"), 0),
        FlowRunEvent::Resumed { carried: 0 },
        FlowRunEvent::attempt_started(id("design"), 0),
        FlowRunEvent::AttemptPassed {
            node: id("design"),
            record: line(2, 2),
        },
        FlowRunEvent::Invalidated {
            gate: id("gate"),
            members: vec![id("design")],
        },
        FlowRunEvent::RerunScheduled {
            gate: id("gate"),
            members: vec![id("design")],
        },
        FlowRunEvent::attempt_started(id("design"), 1),
        FlowRunEvent::Resumed { carried: 0 },
    ]);
    let design = state.node(&id("design")).expect("design");
    assert_eq!(design.generation, 1);
    assert_eq!(design.crashed, 1, "only this generation's crash is left");
    assert_eq!(design.spent(), 0);

    let (rerun, _) = run(vec![
        started(),
        FlowRunEvent::attempt_started(id("design"), 0),
        FlowRunEvent::Resumed { carried: 0 },
        FlowRunEvent::attempt_started(id("design"), 0),
        FlowRunEvent::AttemptPassed {
            node: id("design"),
            record: line(2, 2),
        },
        FlowRunEvent::Invalidated {
            gate: id("gate"),
            members: vec![id("design")],
        },
        FlowRunEvent::RerunScheduled {
            gate: id("gate"),
            members: vec![id("design")],
        },
        FlowRunEvent::attempt_started(id("design"), 1),
        FlowRunEvent::AttemptFailed {
            node: id("design"),
            failure: FailureLine::Exit { code: Some(1) },
            record: line(1, 3),
        },
    ]);
    let design = rerun.node(&id("design")).expect("design");
    assert_eq!(
        (design.crashed, design.spent()),
        (0, 1),
        "the earlier generation's crash was carried into the rerun"
    );
}

/// A member whose rerun is already in flight cannot be scheduled again: a
/// second start beside the live attempt is not a rerun.
#[test]
fn a_rerun_of_a_member_in_flight_is_refused() {
    let (state, _) = run(vec![
        started(),
        FlowRunEvent::attempt_started(id("design"), 0),
        FlowRunEvent::AttemptPassed {
            node: id("design"),
            record: line(1, 1),
        },
        FlowRunEvent::Invalidated {
            gate: id("gate"),
            members: vec![id("design")],
        },
        FlowRunEvent::RerunScheduled {
            gate: id("gate"),
            members: vec![id("design")],
        },
        FlowRunEvent::attempt_started(id("design"), 1),
    ]);
    let again = FlowRunEvent::RerunScheduled {
        gate: id("gate"),
        members: vec![id("design")],
    };
    assert_eq!(
        FlowRunMachine::check(&state, &again),
        Err(FlowRunRejected::InFlight(id("design")))
    );
}

/// A lost output takes the pass away and starts the node's next generation
/// with nothing spent, however many attempts the lost pass took.
#[test]
fn a_lost_output_starts_a_new_generation() {
    let (state, _) = run(vec![
        started(),
        FlowRunEvent::attempt_started(id("design"), 0),
        FlowRunEvent::AttemptFailed {
            node: id("design"),
            failure: FailureLine::Exit { code: Some(1) },
            record: line(1, 1),
        },
        FlowRunEvent::attempt_started(id("design"), 0),
        FlowRunEvent::AttemptPassed {
            node: id("design"),
            record: line(2, 2),
        },
        FlowRunEvent::OutputLost { node: id("design") },
    ]);
    let design = state.node(&id("design")).expect("design");
    assert_eq!(
        (design.generation, design.attempts, design.spent()),
        (1, 0, 0)
    );
    assert!(FlowRunMachine::check(&state, &FlowRunEvent::attempt_started(id("design"), 1)).is_ok());
}

/// Only a run that stopped without ending, or ended stalled, resumes.
#[test]
fn only_a_crashed_or_stalled_run_resumes() {
    let done = run(vec![
        started(),
        FlowRunEvent::RunEnded { end: EndLine::Done },
    ])
    .0;
    assert!(matches!(
        FlowRunMachine::check(&done, &FlowRunEvent::Resumed { carried: 0 }),
        Err(FlowRunRejected::NotResumable)
    ));
    let stalled = run(vec![
        started(),
        FlowRunEvent::RunEnded {
            end: EndLine::Stalled { nodes: Vec::new() },
        },
    ])
    .0;
    assert!(FlowRunMachine::check(&stalled, &FlowRunEvent::Resumed { carried: 0 }).is_ok());
}

/// **F7.** A command's group is kept with its attempt until it settles, once
/// per attempt, and only for the attempt that started it.
#[test]
fn a_spawned_group_belongs_to_its_attempt_until_it_settles() {
    let (state, _) = run(vec![
        started(),
        FlowRunEvent::attempt_started(id("gate"), 0),
        spawned("gate", 1),
    ]);
    assert_eq!(state.unsettled_processes().len(), 1);
    for refused in [spawned("gate", 1), spawned("gate", 2), spawned("design", 1)] {
        assert!(
            FlowRunMachine::check(&state, &refused).is_err(),
            "{refused:?} was accepted"
        );
    }
    let (settled, _) = run(vec![
        started(),
        FlowRunEvent::attempt_started(id("gate"), 0),
        spawned("gate", 1),
        FlowRunEvent::AttemptPassed {
            node: id("gate"),
            record: line(1, 1),
        },
    ]);
    assert!(settled.unsettled_processes().is_empty());
}
