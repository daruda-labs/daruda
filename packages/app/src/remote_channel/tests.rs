use super::*;
use futures::{FutureExt as _, StreamExt as _};

fn flow_question(ask_id: u64) -> bridge::FlowAskPrompt {
    bridge::FlowAskPrompt {
        worktree: "/worktree".into(),
        run_dir: "/worktree/.daruda/flow-runs/run".into(),
        ask_id,
        summary: "Choose an option".into(),
        buttons: vec![(
            "Keep once".into(),
            bridge::PermissionDecision::Allow("original-once".into()),
        )],
    }
}

#[gpui::test]
fn flow_question_fanout_attempts_every_eligible_channel(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let mut remote = global::tests::setup(cx);
        let mut telegram = crate::telegram::global::install_for_test(true, Some(42), cx);
        for (index, (delivery, expected)) in [
            (global::Delivery::Explicit, 2),
            (global::Delivery::Presence { away: false }, 1),
            (global::Delivery::Presence { away: true }, 2),
        ]
        .into_iter()
        .enumerate()
        {
            let prompt = flow_question(index as u64);
            assert!(send_flow_ask(prompt.clone(), delivery, cx));
            let mut channels = Vec::new();
            while let Some(Some(pending)) = remote.next().now_or_never() {
                let bridge::Outbound::FlowAsk(queued) = pending.outbound else {
                    panic!("flow question");
                };
                assert_eq!(queued.ask_id, prompt.ask_id);
                assert_eq!(queued.buttons, prompt.buttons);
                channels.push(pending.id);
            }
            assert_eq!(channels.len(), expected);
            assert!(channels.contains(&"personal".to_string()));
            if expected == 2 {
                let crate::telegram::bridge::Outbound::FlowAsk(queued) =
                    telegram.try_recv().unwrap()
                else {
                    panic!("flow question");
                };
                assert_eq!(queued.buttons, prompt.buttons);
            } else {
                assert!(telegram.try_recv().is_err());
            }
        }
        drop(remote);
        assert!(send_flow_ask(
            flow_question(3),
            global::Delivery::Explicit,
            cx
        ));
        assert!(telegram.try_recv().is_ok());
        drop(telegram);
        assert!(!send_flow_ask(
            flow_question(4),
            global::Delivery::Explicit,
            cx
        ));
    });
}

#[gpui::test]
fn card_completion_only_edits_a_settled_receipt(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let mut edits = Vec::new();
        let card = cards::CardId::FlowAsk {
            run_dir: "/worktree/.daruda/flow-runs/run".into(),
            ask_id: 1,
        };
        for completion in [
            cards::CardCompletion::Ignored,
            cards::CardCompletion::Unconfirmed,
            cards::CardCompletion::Delivered,
            cards::CardCompletion::Edit {
                receipt: "message",
                label: "Allowed".into(),
            },
        ] {
            apply_card_completion(
                &card,
                completion,
                |receipt, label, _| edits.push((receipt, label)),
                cx,
            );
        }
        assert_eq!(edits, vec![("message", "Allowed".to_string())]);
    });
}
