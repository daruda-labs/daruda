use super::*;

fn fold(events: &[FlowEvent]) -> NodeRunStates {
    let mut states = NodeRunStates::new();
    for event in events {
        apply_run_event(&mut states, event);
    }
    states
}

/// A node takes the colour of the last thing the run said about it; a
/// node the run has not spoken about is pending.
#[test]
fn each_node_wears_the_last_word_on_it() {
    let states = fold(&[
        FlowEvent::NodeStarted {
            node: "design".into(),
            attempt: 2,
        },
        FlowEvent::NodeStarted {
            node: "build".into(),
            attempt: 1,
        },
        FlowEvent::NodePassed {
            node: "build".into(),
            attempt: 1,
        },
    ]);
    assert_eq!(
        states.get(&NodeId::from("design")),
        Some(&NodeRunState::Running { attempt: 2 })
    );
    assert_eq!(
        states.get(&NodeId::from("build")),
        Some(&NodeRunState::Passed)
    );
    assert_eq!(
        states
            .get(&NodeId::from("review"))
            .copied()
            .unwrap_or_default(),
        NodeRunState::Pending
    );
}

/// A repair: the gate is fixing, then still failed until it re-runs, and
/// the members it re-derives lose the pass they had.
#[test]
fn a_repair_takes_back_what_it_re_derives() {
    let states = fold(&[
        FlowEvent::NodePassed {
            node: "design".into(),
            attempt: 1,
        },
        FlowEvent::NodeFailed {
            node: "gate".into(),
            attempt: 1,
            failure: daruda_flow::runner::NodeFailure::Exit { code: Some(1) },
        },
        FlowEvent::FixStarted {
            gate: "gate".into(),
        },
    ]);
    assert_eq!(
        states.get(&NodeId::from("gate")),
        Some(&NodeRunState::Fixing)
    );
    let states = {
        let mut states = states;
        apply_run_event(
            &mut states,
            &FlowEvent::FixEnded {
                gate: "gate".into(),
                failure: None,
            },
        );
        apply_run_event(
            &mut states,
            &FlowEvent::Rerunning {
                gate: "gate".into(),
                members: vec!["design".into()],
            },
        );
        states
    };
    assert_eq!(
        states.get(&NodeId::from("gate")),
        Some(&NodeRunState::Failed)
    );
    assert_eq!(
        states.get(&NodeId::from("design")),
        Some(&NodeRunState::Pending)
    );
}

/// A colouring is only about the graph it was made for: the same ids in
/// the same order.
#[test]
fn a_colouring_is_about_the_nodes_it_was_made_for() {
    let colouring = RunColouring {
        states: NodeRunStates::new(),
        of_nodes: vec!["design".into(), "build".into()],
    };
    assert!(colouring.is_about(["design", "build"].into_iter().map(NodeId::from)));
    assert!(!colouring.is_about(["build", "design"].into_iter().map(NodeId::from)));
    assert!(!colouring.is_about(["design"].into_iter().map(NodeId::from)));
}
