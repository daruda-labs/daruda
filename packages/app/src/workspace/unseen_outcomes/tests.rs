use super::*;

const A: PaneId = 1;
const B: PaneId = 2;

#[test]
fn a_later_outcome_replaces_an_earlier_one() {
    let mut unseen = UnseenOutcomes::default();
    unseen.record(A, AgentOutcome::Errored);
    unseen.record(A, AgentOutcome::Completed);
    assert_eq!(
        unseen.for_panes(&[A]).collect::<Vec<_>>(),
        [AgentOutcome::Completed]
    );
}

#[test]
fn only_the_asked_panes_are_reported() {
    let mut unseen = UnseenOutcomes::default();
    unseen.record(A, AgentOutcome::Completed);
    unseen.record(B, AgentOutcome::Errored);
    assert_eq!(
        unseen.for_panes(&[B]).collect::<Vec<_>>(),
        [AgentOutcome::Errored]
    );
    assert_eq!(unseen.for_panes(&[]).count(), 0);
}

#[test]
fn forgetting_reports_whether_anything_went() {
    let mut unseen = UnseenOutcomes::default();
    unseen.record(A, AgentOutcome::Completed);
    assert!(!unseen.forget(&[B]));
    assert!(unseen.forget(&[A, B]));
    assert_eq!(unseen.panes().count(), 0);
}

#[test]
fn lane_outcomes_keep_projects_separate_and_follow_acknowledgements() {
    let first = LaneRef {
        project: 1,
        lane: 0,
    };
    let second = LaneRef {
        project: 2,
        lane: 0,
    };
    let panes = [(A, first), (B, second)];
    let mut unseen = UnseenOutcomes::default();
    unseen.record(A, AgentOutcome::Completed);
    unseen.record(B, AgentOutcome::Errored);
    unseen.record(3, AgentOutcome::Completed);
    assert_eq!(
        unseen.by_lane(&panes),
        HashMap::from([
            (first, AgentOutcome::Completed),
            (second, AgentOutcome::Errored),
        ])
    );
    unseen.forget(&[A]);
    assert_eq!(
        unseen.by_lane(&panes),
        HashMap::from([(second, AgentOutcome::Errored)])
    );
}

#[test]
fn an_unseen_error_outranks_completion_regardless_of_pane_order() {
    let lane = LaneRef::default();
    let mut unseen = UnseenOutcomes::default();
    unseen.record(A, AgentOutcome::Completed);
    unseen.record(B, AgentOutcome::Errored);
    for panes in [[(A, lane), (B, lane)], [(B, lane), (A, lane)]] {
        assert_eq!(
            unseen.by_lane(&panes),
            HashMap::from([(lane, AgentOutcome::Errored)])
        );
    }
}
