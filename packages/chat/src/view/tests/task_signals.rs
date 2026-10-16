//! What a task-tagged prompt tells the host: that it reached the wire, or
//! that it left the queue without ever being sent. Untagged prompts are
//! silent on both counts.

use std::cell::RefCell;
use std::rc::Rc;

use daruda_acp::{AcpEvent, AcpFailure, AcpSessionHandle, HandleProbe};
use daruda_store::tasks::ExecutionRef;
use gpui::{Entity, TestAppContext};

use super::super::{
    AgentChatEvent, AgentChatView, AgentSessionStatus, PromptDispatch, PromptDropReason, PromptId,
    PromptOrigin, QueuedPrompt,
};
use super::make_test_view;

#[derive(Debug, PartialEq, Eq)]
enum Signal {
    Sent(ExecutionRef),
    Dropped(Vec<ExecutionRef>, PromptDropReason),
}

fn run(task: &str) -> ExecutionRef {
    ExecutionRef {
        task_id: task.to_string(),
        execution_id: format!("{task}-exec"),
    }
}

fn tagged(id: u64, task: &str) -> QueuedPrompt {
    QueuedPrompt {
        id: PromptId(id),
        text: format!("prompt {id}"),
        attachments: Vec::new(),
        origin: PromptOrigin::InApp,
        task: Some(run(task)),
    }
}

fn untagged(id: u64, origin: PromptOrigin) -> QueuedPrompt {
    QueuedPrompt {
        id: PromptId(id),
        text: format!("prompt {id}"),
        attachments: Vec::new(),
        origin,
        task: None,
    }
}

/// A pane as its window's root plus a log of the task signals it emits.
fn observed(cx: &mut TestAppContext) -> (Entity<AgentChatView>, Rc<RefCell<Vec<Signal>>>) {
    let handle = make_test_view(cx);
    cx.run_until_parked();
    let view = handle.root(cx).expect("the view's window is open");
    let log = Rc::new(RefCell::new(Vec::new()));
    let sink = log.clone();
    cx.update(|cx| {
        cx.subscribe(&view, move |_, event: &AgentChatEvent, _| match event {
            AgentChatEvent::PromptSent { task, .. } => {
                sink.borrow_mut().push(Signal::Sent(task.clone()))
            }
            AgentChatEvent::PromptsDropped { tasks, reason } => {
                sink.borrow_mut()
                    .push(Signal::Dropped(tasks.clone(), *reason));
            }
            _ => {}
        })
        .detach();
    });
    (view, log)
}

/// Connected with a detached handle, so a send really reaches "the wire".
fn connect(view: &Entity<AgentChatView>, cx: &mut TestAppContext) -> HandleProbe {
    let (handle, probe) = AcpSessionHandle::detached_for_test();
    view.update(cx, |v, _| {
        v.status = AgentSessionStatus::Connected;
        v.attach_handle(handle);
    });
    probe
}

#[gpui::test]
fn a_tagged_prompt_sent_now_reports_sent_once(cx: &mut TestAppContext) {
    let (view, log) = observed(cx);
    let mut probe = connect(&view, cx);

    let dispatch = view.update(cx, |v, cx| {
        v.send_task_prompt_text("go".into(), run("t1"), cx)
    });

    assert_eq!(dispatch, PromptDispatch::SentNow);
    assert_eq!(probe.drain(), (1, false), "the prompt went over the handle");
    assert_eq!(*log.borrow(), vec![Signal::Sent(run("t1"))]);
}

#[gpui::test]
fn a_closed_prompt_channel_never_reports_sent(cx: &mut TestAppContext) {
    let (view, log) = observed(cx);
    drop(connect(&view, cx));
    let dispatch = view.update(cx, |view, cx| {
        view.send_task_prompt_text("go".into(), run("t1"), cx)
    });
    assert_eq!(dispatch, PromptDispatch::Unavailable);
    assert_eq!(
        *log.borrow(),
        vec![Signal::Dropped(
            vec![run("t1")],
            PromptDropReason::StartFailed
        )]
    );
    assert!(view.read_with(cx, |view, _| view.turn_is_idle()));
}

#[gpui::test]
fn a_queued_prompt_losing_its_channel_is_dropped_without_a_phantom_turn(cx: &mut TestAppContext) {
    let (view, log) = observed(cx);
    let probe = connect(&view, cx);
    view.update(cx, |view, cx| {
        view.set_turn_in_flight();
        assert_eq!(
            view.send_task_prompt_text("go".into(), run("t1"), cx),
            PromptDispatch::Queued
        );
    });
    drop(probe);
    view.update(cx, |view, cx| {
        view.set_turn_idle();
        view.pump_pending_prompt(cx);
    });
    assert_eq!(
        *log.borrow(),
        vec![Signal::Dropped(
            vec![run("t1")],
            PromptDropReason::StartFailed
        )]
    );
    assert!(view.read_with(cx, |view, _| view.turn_is_idle()));
}

#[gpui::test]
fn a_tagged_prompt_sent_from_the_queue_reports_sent_once(cx: &mut TestAppContext) {
    let (view, log) = observed(cx);
    let mut probe = connect(&view, cx);

    view.update(cx, |v, cx| {
        v.set_turn_in_flight();
        assert_eq!(
            v.send_task_prompt_text("later".into(), run("t1"), cx),
            PromptDispatch::Queued
        );
    });
    assert!(log.borrow().is_empty(), "queueing is not sending");

    view.update(cx, |v, cx| {
        v.set_turn_idle();
        v.pump_pending_prompt(cx);
        // A second pump finds the queue empty and must not report again.
        v.set_turn_idle();
        v.pump_pending_prompt(cx);
    });

    assert_eq!(probe.drain(), (1, false));
    assert_eq!(*log.borrow(), vec![Signal::Sent(run("t1"))]);
}

#[gpui::test]
fn a_failed_connect_reports_every_tagged_prompt_it_drops(cx: &mut TestAppContext) {
    let (view, log) = observed(cx);

    view.update(cx, |v, cx| {
        v.queue.paused_prompts.push(tagged(1, "parked"));
        v.queue
            .paused_prompts
            .push(untagged(2, PromptOrigin::InApp));
        v.queue.pending_prompts.push(tagged(3, "live"));
        v.apply_event(
            AcpEvent::Error(AcpFailure::unclassified("adapter exited")),
            "",
            false,
            cx,
        );
        assert_eq!(v.queued_prompt_count(), 0);
    });

    assert_eq!(
        *log.borrow(),
        vec![Signal::Dropped(
            vec![run("parked"), run("live")],
            PromptDropReason::ConnectFailed
        )]
    );
}

/// A session that had connected and then failed is not a failed connect:
/// the prompts it drops say the session failed.
#[gpui::test]
fn a_session_failing_after_connect_reports_its_drops_as_a_session_failure(cx: &mut TestAppContext) {
    let (view, log) = observed(cx);
    let _probe = connect(&view, cx);

    view.update(cx, |v, cx| {
        v.queue.pending_prompts.push(tagged(1, "queued"));
        v.apply_event(
            AcpEvent::Error(AcpFailure::unclassified("adapter exited")),
            "",
            false,
            cx,
        );
    });

    assert_eq!(
        *log.borrow(),
        vec![Signal::Dropped(
            vec![run("queued")],
            PromptDropReason::SessionFailed
        )]
    );
}

/// A task's prompt arriving while the composer edits a queued prompt is
/// queued with its tag, and leaves the slot being edited and the editing
/// mode as they were.
#[gpui::test]
fn a_task_prompt_never_lands_in_the_slot_being_edited(cx: &mut TestAppContext) {
    let (view, _log) = observed(cx);

    view.update(cx, |v, cx| {
        v.queue
            .pending_prompts
            .push(untagged(1, PromptOrigin::InApp));
        v.begin_edit(PromptId(1), cx);
        let dispatch = v.send_task_prompt_text("task text".into(), run("t1"), cx);

        assert_eq!(dispatch, PromptDispatch::Queued);
        assert_eq!(v.queue.editing_prompt, Some(PromptId(1)), "the edit ended");
        let edited = v
            .queue
            .pending_prompts
            .iter()
            .find(|q| q.id == PromptId(1))
            .expect("the edited prompt is still queued");
        assert_eq!(edited.text, "prompt 1", "the task's text overwrote it");
        assert!(edited.task.is_none());
        let queued = v
            .queue
            .pending_prompts
            .iter()
            .find(|q| q.task == Some(run("t1")))
            .expect("the task's prompt was not queued");
        assert_eq!(queued.text, "task text");
    });
}

#[gpui::test]
fn a_session_teardown_reports_dropped_tagged_prompts(cx: &mut TestAppContext) {
    let (view, log) = observed(cx);

    view.update(cx, |v, cx| {
        v.queue.pending_prompts.push(tagged(1, "live"));
        v.queue.paused_prompts.push(tagged(2, "parked"));
        v.retry_for_reconnect(None, cx);
        assert_eq!(v.queued_prompt_count(), 0);
    });

    assert_eq!(
        *log.borrow(),
        vec![Signal::Dropped(
            vec![run("parked"), run("live")],
            PromptDropReason::SessionTornDown
        )]
    );
}

#[gpui::test]
fn removing_a_tagged_queued_prompt_reports_it_dropped_by_the_user(cx: &mut TestAppContext) {
    let (view, log) = observed(cx);

    view.update(cx, |v, cx| {
        v.queue.pending_prompts.push(tagged(1, "keep"));
        v.queue.paused_prompts.push(tagged(2, "gone"));
        v.remove_queued(PromptId(2), cx);
        // Removing an id that is no longer queued drops nothing.
        v.remove_queued(PromptId(2), cx);
    });

    assert_eq!(
        *log.borrow(),
        vec![Signal::Dropped(
            vec![run("gone")],
            PromptDropReason::RemovedByUser
        )]
    );
}

#[gpui::test]
fn clearing_the_queue_reports_tagged_prompts_dropped_by_the_user(cx: &mut TestAppContext) {
    let (view, log) = observed(cx);

    view.update(cx, |v, cx| {
        v.queue.paused_prompts.push(tagged(1, "parked"));
        v.queue
            .pending_prompts
            .push(untagged(2, PromptOrigin::InApp));
        v.queue.pending_prompts.push(tagged(3, "live"));
        v.clear_queue(cx);
    });

    assert_eq!(
        *log.borrow(),
        vec![Signal::Dropped(
            vec![run("parked"), run("live")],
            PromptDropReason::RemovedByUser
        )]
    );
}

#[gpui::test]
fn an_untagged_prompt_emits_nothing_new(cx: &mut TestAppContext) {
    let (view, log) = observed(cx);
    let mut probe = connect(&view, cx);

    view.update(cx, |v, cx| {
        assert_eq!(
            v.send_prompt_text("typed".into(), cx),
            PromptDispatch::SentNow
        );
        v.queue
            .pending_prompts
            .push(untagged(10, PromptOrigin::InApp));
        v.queue
            .pending_prompts
            .push(untagged(11, PromptOrigin::Telegram));
        v.set_turn_idle();
        v.pump_pending_prompt(cx);
        v.queue
            .pending_prompts
            .push(untagged(12, PromptOrigin::InApp));
        v.remove_queued(PromptId(12), cx);
        v.queue
            .paused_prompts
            .push(untagged(13, PromptOrigin::InApp));
        v.clear_queue(cx);
        v.queue
            .pending_prompts
            .push(untagged(14, PromptOrigin::Telegram));
        v.retry_for_reconnect(None, cx);
        v.queue
            .pending_prompts
            .push(untagged(15, PromptOrigin::InApp));
        v.apply_event(
            AcpEvent::Error(AcpFailure::unclassified("adapter exited")),
            "",
            false,
            cx,
        );
    });

    assert_eq!(probe.drain(), (2, true), "both untagged prompts were sent");
    assert!(log.borrow().is_empty(), "got {:?}", log.borrow());
}

#[gpui::test]
fn parking_on_stop_is_not_a_drop(cx: &mut TestAppContext) {
    let (view, log) = observed(cx);

    view.update(cx, |v, cx| {
        v.set_turn_in_flight();
        v.queue.pending_prompts.push(tagged(1, "t1"));
        v.cancel_turn(cx);
        assert_eq!(v.queue.paused_prompts.len(), 1, "the prompt is parked");
    });

    assert!(log.borrow().is_empty(), "got {:?}", log.borrow());
}
