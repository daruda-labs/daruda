use std::path::PathBuf;

use daruda_flow::event::FlowEvent;
use daruda_flow::runner::{AskOwner, AskRequest, PendingAsk};

use super::*;

fn view() -> RunView {
    let run_dir = PathBuf::from("/run");
    RunView::new(
        run_dir.clone(),
        FlowSource::Resumed { run_dir },
        vec!["design".into(), "build".into()],
    )
}

/// A question and the receiver an answer would arrive on.
fn ask(
    id: u64,
    node: &str,
) -> (
    ParkedAsk,
    smol::channel::Receiver<daruda_acp::PermissionDecision>,
) {
    let (reply, rx) = smol::channel::bounded(1);
    let pending = PendingAsk {
        owner: AskOwner::Node {
            node: node.into(),
            generation: 0,
            attempt: 1,
        },
        ask_id: id,
        request: AskRequest {
            tool: "write".into(),
            detail: None,
            options: Vec::new(),
        },
        reply,
    };
    (ParkedAsk::new(pending), rx)
}

fn open_ids(view: &RunView) -> Vec<u64> {
    view.questions().map(ParkedAsk::ask_id).collect()
}

fn started(node: &str) -> FlowEvent {
    FlowEvent::NodeStarted {
        node: node.into(),
        attempt: 1,
    }
}

/// Nodes running together ask independently: a second question sits beside
/// the first, and only the first is the one to bring forward.
#[test]
fn questions_are_held_as_a_set() {
    let mut view = view();
    let (first, _rx1) = ask(1, "design");
    let (second, _rx2) = ask(2, "build");
    assert_eq!(view.park(first), Parked::Alone);
    assert_eq!(view.park(second), Parked::Beside);
    assert_eq!(open_ids(&view), [1, 2]);
}

/// A sibling's progress leaves the question standing; taking it down would
/// leave its node parked on a reply nobody could send.
#[test]
fn a_sibling_event_never_drops_an_open_question() {
    let mut view = view();
    view.advance(&started("design"));
    let (question, rx) = ask(1, "design");
    view.park(question);
    view.advance(&started("build"));
    view.advance(&FlowEvent::NodePassed {
        node: "build".into(),
        attempt: 1,
    });
    assert_eq!(open_ids(&view), [1], "the sibling took the question down");
    assert_eq!(
        view.answer(1, daruda_acp::PermissionDecision::Cancelled),
        Ok(())
    );
    assert_eq!(rx.try_recv(), Ok(daruda_acp::PermissionDecision::Cancelled));
}

/// Answering one leaves the rest up; a stale id answers nothing.
#[test]
fn answering_takes_down_only_the_question_it_names() {
    let mut view = view();
    let (first, rx1) = ask(1, "design");
    let (second, rx2) = ask(2, "build");
    view.park(first);
    view.park(second);
    assert_eq!(
        view.answer(9, daruda_acp::PermissionDecision::Cancelled),
        Err(AnswerRefused::NotOpen)
    );
    assert_eq!(
        view.answer(2, daruda_acp::PermissionDecision::Cancelled),
        Ok(())
    );
    assert!(rx1.try_recv().is_err(), "the other question was answered");
    assert!(rx2.try_recv().is_ok());
    assert_eq!(open_ids(&view), [1]);
}

/// The engine abandoned the question, so the answer reaches nobody — and
/// says so — and the question does not stay up.
#[test]
fn an_answer_to_an_abandoned_question_is_refused() {
    let mut view = view();
    let (question, rx) = ask(1, "design");
    view.park(question);
    drop(rx);
    assert_eq!(
        view.answer(1, daruda_acp::PermissionDecision::Cancelled),
        Err(AnswerRefused::Closed)
    );
    assert!(open_ids(&view).is_empty());
}

/// The engine closing a question is the one event that takes it down.
#[test]
fn a_question_the_engine_closed_goes_on_the_next_event() {
    let mut view = view();
    let (closed, rx) = ask(1, "design");
    let (open, _rx) = ask(2, "build");
    view.park(closed);
    view.park(open);
    drop(rx);
    assert_eq!(
        view.advance(&started("review")),
        Advanced::Moved { left_setup: true }
    );
    assert_eq!(open_ids(&view), [2]);
}

#[test]
fn stopping_withdraws_every_question() {
    let mut view = view();
    let (first, rx1) = ask(1, "design");
    let (second, rx2) = ask(2, "build");
    view.park(first);
    view.park(second);
    view.withdraw_questions();
    assert!(open_ids(&view).is_empty());
    assert!(rx1.is_closed() && rx2.is_closed(), "the engine is released");
}

/// Only the first node reports leaving setup: that is the one moment
/// retention has run, so the history read from disk is stale exactly then.
#[test]
fn leaving_setup_is_reported_once() {
    let mut view = view();
    assert_eq!(
        view.advance(&started("design")),
        Advanced::Moved { left_setup: true }
    );
    assert_eq!(
        view.advance(&started("build")),
        Advanced::Moved { left_setup: false }
    );
    assert_eq!(view.advance(&started("build")), Advanced::Nothing);
}

/// Running nodes are a set, drawn in the flow's order.
#[test]
fn every_node_under_way_is_running() {
    let mut view = view();
    view.advance(&started("build"));
    view.advance(&started("design"));
    let running: Vec<String> = view
        .running()
        .into_iter()
        .map(|(id, _)| id.to_string())
        .collect();
    assert_eq!(running, ["design", "build"]);
    view.advance(&FlowEvent::NodePassed {
        node: "design".into(),
        attempt: 1,
    });
    assert_eq!(view.running().len(), 1);
}

/// The log's word replaces what the stream left behind — a stopped
/// node that was still drawn running, a carried node never re-announced.
#[test]
fn the_log_settles_every_node() {
    let mut view = view();
    view.advance(&started("build"));
    let mut from_log = NodeRunStates::new();
    from_log.insert("design".into(), NodeRunState::Passed);
    from_log.insert("build".into(), NodeRunState::Pending);
    assert!(view.reconcile(from_log.clone()));
    assert_eq!(view.node_state("design"), NodeRunState::Passed);
    assert_eq!(view.node_state("build"), NodeRunState::Pending);
    assert!(
        !view.reconcile(from_log),
        "the same word twice changes nothing"
    );
}

/// A resume's wait for the dead run is still setup: the run says it is
/// waiting, and leaves setup only when its first node starts.
#[test]
fn a_resume_wait_is_still_setup() {
    let mut view = view();
    let settling = FlowEvent::Settling {
        wait: std::time::Duration::from_secs(4),
    };
    assert_eq!(
        view.advance(&settling),
        Advanced::Moved { left_setup: false }
    );
    assert_eq!(view.lull(), &Lull::Settling);
    assert_eq!(
        view.advance(&started("design")),
        Advanced::Moved { left_setup: true }
    );
}

#[test]
fn waiting_level_lasts_until_all_questions_close() {
    use crate::runs::result::WaitingOn;
    let mut view = view();
    let (first, rx1) = ask(1, "design");
    let (second, _rx2) = ask(2, "build");
    view.park(first);
    view.park(second);
    assert_eq!(
        view.waiting_on(),
        Some(WaitingOn::Question {
            node: "design".into()
        })
    );
    drop(rx1);
    assert_eq!(
        view.waiting_on(),
        Some(WaitingOn::Question {
            node: "build".into()
        })
    );
    view.advance(&started("build"));
    view.answer(2, daruda_acp::PermissionDecision::Cancelled)
        .unwrap();
    assert_eq!(view.waiting_on(), None);
    let (third, _rx3) = ask(3, "design");
    view.park(third);
    assert!(view.waiting_on().is_some());
    view.withdraw_questions();
    assert_eq!(view.waiting_on(), None);
}
