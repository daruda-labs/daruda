//! The opt-in question answer timeout seen from the host: an expired
//! question notifies once that the agent went on without an answer.

use std::time::{Duration, Instant};

use gpui::TestAppContext;

use super::{asked, question};
use crate::surface::strings as s;

/// §0.3: expiry tells the person once; a question that ends any other way
/// (here, skipped) does not.
#[gpui::test]
fn an_expired_question_notifies_once_and_a_skipped_one_does_not(cx: &mut TestAppContext) {
    let (_dir, ws, pane, mut answer) = asked(false, cx);
    let view = ws.read_with(cx, |ws, _| {
        ws.agent_chat_view(pane).expect("chat pane").clone()
    });
    ws.update(cx, |ws, _| {
        ws.mirrors.notifications.agent_waiting_enabled = true;
        ws.mirrors.notifications.skip_focused_pane = false;
    });
    let timeout = Duration::from_secs(60);
    view.update(cx, |v, cx| v.set_question_timeout(Some(timeout), cx));
    cx.run_until_parked();
    let _ = crate::platform::notifications::take_recorded_for_test();

    let later = Instant::now() + timeout * 2;
    view.update(cx, |v, cx| v.tick_activity(later, cx));
    cx.run_until_parked();
    view.update(cx, |v, cx| v.tick_activity(later + timeout, cx));
    cx.run_until_parked();
    assert_eq!(answer.answer(), Some(daruda_acp::QuestionAnswer::Decline));
    let shown = crate::platform::notifications::take_recorded_for_test();
    assert_eq!(
        shown
            .iter()
            .map(|(_, body)| body.clone())
            .collect::<Vec<_>>(),
        [s::notification::agent_question_expired(
            view.read_with(cx, |v, _| v.agent_name().to_owned()),
            "Which color?",
        )],
        "exactly one notification"
    );

    view.update(cx, |v, cx| {
        v.ask_question_for_test(2, question(), cx);
        v.skip_question(2, cx);
    });
    cx.run_until_parked();
    assert!(crate::platform::notifications::take_recorded_for_test().is_empty());
}
