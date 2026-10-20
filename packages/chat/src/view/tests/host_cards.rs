use std::cell::RefCell;
use std::rc::Rc;

use gpui::{Modifiers, TestAppContext, VisualTestContext};

use super::super::{AgentChatEvent, HostCard, HostCardAction};
use super::open_view;

fn card(id: u64) -> HostCard {
    HostCard {
        id,
        summary: format!("Start task {id}?"),
        actions: vec![
            HostCardAction {
                id: "approve".into(),
                label: "Approve".into(),
            },
            HostCardAction {
                id: "refuse".into(),
                label: "Refuse".into(),
            },
        ],
    }
}

#[gpui::test]
fn host_card_commands_notify_without_changing_the_transcript(cx: &mut TestAppContext) {
    let (_, view) = open_view(cx);
    let initial = view.read_with(cx, |view, _| view.render_count.get());

    view.update(cx, |view, cx| view.push_host_card(card(7), cx));
    cx.run_until_parked();
    let pushed = view.read_with(cx, |view, _| {
        assert!(view.items.is_empty());
        assert_eq!(view.host_cards.entries().len(), 1);
        view.render_count.get()
    });
    assert!(pushed > initial);

    view.update(cx, |view, cx| {
        view.settle_host_card(7, "Handled on phone".into(), cx)
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(view.items.is_empty());
        assert!(view.render_count.get() > pushed);
        assert_eq!(
            view.host_cards.entries()[0].status.label(),
            Some("Handled on phone")
        );
    });
}

#[gpui::test]
fn an_actual_card_click_emits_the_host_choice_once(cx: &mut TestAppContext) {
    let (window, view) = open_view(cx);
    let choices = Rc::new(RefCell::new(Vec::new()));
    let recorded = choices.clone();
    cx.update(|cx| {
        cx.subscribe(&view, move |_, event: &AgentChatEvent, _| {
            if let AgentChatEvent::HostCardChosen { card_id, action_id } = event {
                recorded.borrow_mut().push((*card_id, action_id.clone()));
            }
        })
        .detach();
    });
    view.update(cx, |view, cx| view.push_host_card(card(7), cx));
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window, cx);
    let button = visual
        .debug_bounds("agent-chat-host-card-action-7-0")
        .expect("the action button paints");
    visual.simulate_click(button.center(), Modifiers::default());
    visual.run_until_parked();
    assert_eq!(*choices.borrow(), vec![(7, "approve".to_owned())]);

    view.update(&mut visual, |view, cx| {
        view.choose_host_card(7, "refuse".into(), cx);
        view.settle_host_card(7, "Approved".into(), cx);
        view.choose_host_card(7, "approve".into(), cx);
    });
    assert_eq!(choices.borrow().len(), 1);
}

#[gpui::test]
fn invalid_and_settled_cards_never_emit_choices(cx: &mut TestAppContext) {
    let (_, view) = open_view(cx);
    let choices = Rc::new(RefCell::new(0));
    let recorded = choices.clone();
    cx.update(|cx| {
        cx.subscribe(&view, move |_, event: &AgentChatEvent, _| {
            if matches!(event, AgentChatEvent::HostCardChosen { .. }) {
                *recorded.borrow_mut() += 1;
            }
        })
        .detach();
    });
    view.update(cx, |view, cx| {
        view.push_host_card(card(7), cx);
        view.choose_host_card(8, "approve".into(), cx);
        view.choose_host_card(7, "unknown".into(), cx);
        view.settle_host_card(7, "Handled on phone".into(), cx);
        view.choose_host_card(7, "approve".into(), cx);
    });
    assert_eq!(*choices.borrow(), 0);
}
