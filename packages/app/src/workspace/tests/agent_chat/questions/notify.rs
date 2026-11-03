//! A question arriving fires the desktop "waiting" notification, under the
//! same setting and on-screen rule as a permission request.

use daruda_acp::AcpEvent;
use gpui::TestAppContext;

use super::{asked, question};
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

fn ask(ws: &gpui::Entity<Workspace>, pane: PaneId, id: u64, cx: &mut TestAppContext) {
    let _ = crate::platform::notifications::take_recorded_for_test();
    ws.update(cx, |ws, cx| {
        ws.fold_agent_chat_event(
            pane,
            AcpEvent::QuestionAsked {
                id,
                question: question(),
            },
            cx,
        );
    });
    cx.run_until_parked();
}

fn shown_bodies() -> Vec<String> {
    crate::platform::notifications::take_recorded_for_test()
        .into_iter()
        .map(|(_, body)| body)
        .collect()
}

/// A question in a pane off screen notifies once, naming the agent
/// and the question; one in the pane being looked at is skipped exactly when
/// a permission request there would be.
#[gpui::test]
fn a_question_notifies_like_a_permission_request(cx: &mut TestAppContext) {
    let (_dir, ws, pane, _answer) = asked(false, cx);
    ws.update(cx, |ws, _| {
        ws.mirrors.notifications.agent_waiting_enabled = true;
        ws.mirrors.notifications.skip_focused_pane = true;
    });
    assert!(
        !ws.read_with(cx, |ws, _| ws.pane_on_screen(pane)),
        "the pane sits in a background tab"
    );
    let agent = ws.read_with(cx, |ws, cx| {
        ws.agent_chat_view(pane)
            .expect("chat pane")
            .read(cx)
            .agent_name()
            .to_owned()
    });

    ask(&ws, pane, 2, cx);
    assert_eq!(
        shown_bodies(),
        [s::notification::agent_question_waiting(
            agent,
            "Which color?"
        )]
    );

    ws.update(cx, |ws, _| {
        let runtime = ws.active_runtime_mut();
        runtime.active_tab_index = runtime
            .tabs
            .iter()
            .position(|tab| tab.layout.contains(pane))
            .expect("the pane's tab");
        runtime.focused_pane_id = pane;
    });
    assert!(ws.read_with(cx, |ws, _| ws.pane_on_screen(pane)));
    ask(&ws, pane, 3, cx);
    // The app's activation is the real OS state, as in the approval test.
    let expected = !crate::platform::attention::is_app_active();
    assert_eq!(shown_bodies().len(), usize::from(expected));

    ws.update(cx, |ws, _| {
        ws.mirrors.notifications.agent_waiting_enabled = false;
    });
    ask(&ws, pane, 4, cx);
    assert!(
        shown_bodies().is_empty(),
        "the waiting setting turns it off"
    );
}
