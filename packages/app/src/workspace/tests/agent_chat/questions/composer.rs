//! The composer answering a pending question: Enter while bound, Esc, the
//! prompt a released question blocks, drafts across panes and fields.

use std::collections::BTreeMap;

use daruda_acp::{
    AcpEvent, AcpSessionHandle, QuestionAnswer, QuestionField, QuestionFieldKind, QuestionOption,
    QuestionProbe, QuestionRequest, QuestionValue,
};
use daruda_chat::view::{AgentSessionStatus, QuestionComposer};
use gpui::{Entity, TestAppContext, VisualTestContext};

use super::super::super::build_workspace_with;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::TabEntry;
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};

pub(super) fn field(
    key: &str,
    title: &str,
    kind: QuestionFieldKind,
    of: Option<&str>,
) -> QuestionField {
    QuestionField {
        key: key.into(),
        title: Some(title.into()),
        description: None,
        required: false,
        kind,
        companion_of: of.map(str::to_owned),
        default: None,
    }
}

/// One free-text question.
fn path_question() -> QuestionRequest {
    QuestionRequest {
        message: "Where?".into(),
        tool_call_id: None,
        delivery: daruda_acp::QuestionDelivery::Blocking,
        fields: vec![field("path", "Path", QuestionFieldKind::Text, None)],
    }
}

/// A select with its "Other" box, then a free-text question.
pub(super) fn color_then_reason() -> QuestionRequest {
    let option = |value: &str| QuestionOption {
        value: value.into(),
        label: value.into(),
        description: None,
    };
    QuestionRequest {
        message: "Two things".into(),
        tool_call_id: None,
        delivery: daruda_acp::QuestionDelivery::Blocking,
        fields: vec![
            field(
                "color",
                "Color",
                QuestionFieldKind::SingleSelect(vec![option("red"), option("blue")]),
                None,
            ),
            field(
                "color_other",
                "Other",
                QuestionFieldKind::Text,
                Some("color"),
            ),
            field("why", "Why", QuestionFieldKind::Text, None),
        ],
    }
}

pub(super) fn workspace(
    cx: &mut TestAppContext,
) -> (tempfile::TempDir, Entity<Workspace>, VisualTestContext) {
    let (dir, ws, mut vcx) = workspace_with_dock_shut(cx);
    // The composer must be on screen for keys to reach it.
    vcx.update(|_, cx| {
        let dock = ws.read(cx).docks.bottom.clone();
        dock.update(cx, |dock, _| dock.is_open = true);
    });
    (dir, ws, vcx)
}

fn workspace_with_dock_shut(
    cx: &mut TestAppContext,
) -> (tempfile::TempDir, Entity<Workspace>, VisualTestContext) {
    let dir = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(dir.path());
    let (wh, ws) = build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    cx.run_until_parked();
    let vcx = VisualTestContext::from_window(wh.into(), cx);
    (dir, ws, vcx)
}

/// A yes/no question, which takes no text.
fn yes_no_question() -> QuestionRequest {
    QuestionRequest {
        message: "Ready?".into(),
        tool_call_id: None,
        delivery: daruda_acp::QuestionDelivery::Blocking,
        fields: vec![field("ok", "Ready", QuestionFieldKind::Boolean, None)],
    }
}

/// A yes/no, then a free-text question.
fn yes_no_then_reason() -> QuestionRequest {
    QuestionRequest {
        message: "Two things".into(),
        tool_call_id: None,
        delivery: daruda_acp::QuestionDelivery::Blocking,
        fields: vec![
            field("ok", "Ready", QuestionFieldKind::Boolean, None),
            field("why", "Why", QuestionFieldKind::Text, None),
        ],
    }
}

/// The real Enter key: the composer's own `PressEnter`, not a direct call.
pub(super) fn press_enter_key(vcx: &mut VisualTestContext) {
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
}

/// A chat pane in its own tab, focused the way a click focuses it.
pub(super) fn add_pane(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> PaneId {
    let pane = vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            let pane = ws.create_agent_chat_pane(
                None,
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            let pane_id = pane.id;
            ws.active_runtime_mut().panes.push(pane);
            let tab_id = ws.alloc_id();
            ws.active_runtime_mut().tabs.push(TabEntry {
                id: tab_id,
                layout: PaneLayout::Pane(pane_id),
                last_focused_pane: pane_id,
                user_label: None,
            });
            pane_id
        })
    });
    focus(ws, vcx, pane);
    pane
}

fn focus(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, pane: PaneId) {
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.set_focused_pane(pane, window, cx);
            ws.focus_pane(pane, window, cx);
        })
    });
    vcx.run_until_parked();
}

/// Connect `pane` with a turn in flight and questions `ids` parked.
pub(super) fn connect(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    pane: PaneId,
    ids: &[u64],
) -> Vec<QuestionProbe> {
    let (handle, _probe) = AcpSessionHandle::detached_for_test();
    let probes = ids
        .iter()
        .map(|id| handle.park_question_for_test(*id))
        .collect();
    vcx.update(|_, cx| {
        let view = ws
            .read(cx)
            .agent_chat_view(pane)
            .expect("chat pane")
            .clone();
        view.update(cx, |v, _| {
            v.set_status_for_test(AgentSessionStatus::Connected);
            v.attach_handle(handle);
            v.set_turn_in_flight();
        });
    });
    probes
}

pub(super) fn ask(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    pane: PaneId,
    id: u64,
    question: QuestionRequest,
) {
    vcx.update(|_, cx| {
        ws.update(cx, |ws, cx| {
            ws.fold_agent_chat_event(pane, AcpEvent::QuestionAsked { id, question }, cx);
        })
    });
    vcx.run_until_parked();
}

pub(super) fn type_text(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, text: &str) {
    vcx.update(|window, cx| {
        let input = ws.read(cx).input_dock.input.clone();
        input.update(cx, |state, cx| state.set_value(text, window, cx));
    });
    vcx.run_until_parked();
}

pub(super) fn composer_text(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> String {
    vcx.update(|_, cx| ws.read(cx).input_dock.input.read(cx).value().to_string())
}

fn press_enter(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) {
    vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.send_terminal_input(window, cx)));
    vcx.run_until_parked();
}

fn press_escape(vcx: &mut VisualTestContext) {
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
}

fn summary(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    pane: PaneId,
) -> Option<QuestionComposer> {
    vcx.update(|_, cx| {
        ws.read(cx)
            .agent_chat_view(pane)
            .expect("chat pane")
            .read(cx)
            .question_composer()
    })
}

fn field_text(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    pane: PaneId,
    id: u64,
    field: usize,
) -> Option<String> {
    vcx.update(|_, cx| {
        ws.read(cx)
            .agent_chat_view(pane)
            .expect("chat pane")
            .read(cx)
            .question_field_text(id, field)
    })
}

pub(super) fn queued(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    pane: PaneId,
) -> Vec<String> {
    vcx.update(|_, cx| {
        let view = ws
            .read(cx)
            .agent_chat_view(pane)
            .expect("chat pane")
            .clone();
        super::super::queue_texts(view.read(cx))
    })
}

pub(super) fn strip(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
) -> Option<crate::workspace::layout::QuestionStripSnapshot> {
    vcx.update(|_, cx| ws.update(cx, |ws, cx| ws.prepare_bottom_dock_snapshot(cx).question))
}

fn text_answer(key: &str, text: &str) -> Option<QuestionAnswer> {
    Some(QuestionAnswer::Submit(BTreeMap::from([(
        key.to_owned(),
        QuestionValue::Text(text.into()),
    )])))
}

#[gpui::test]
fn enter_while_bound_answers_and_does_not_queue(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, path_question());
    let shown = strip(&ws, &mut vcx).expect("the strip shows the question");
    assert!(shown.composer.bound, "an empty composer takes the question");
    assert_eq!(shown.pending_prompt, None);

    type_text(&ws, &mut vcx, "src/lib.rs");
    press_enter(&ws, &mut vcx);

    assert_eq!(probes[0].answer(), text_answer("path", "src/lib.rs"));
    assert!(
        queued(&ws, &mut vcx, pane).is_empty(),
        "an answer is no prompt"
    );
    assert_eq!(composer_text(&ws, &mut vcx), "");
    let history = vcx.update(|_, cx| {
        let ws = ws.read(cx);
        ws.lane_scoped
            .get(&ws.active_ref())
            .is_some_and(|lane| lane.input_history.has_entries())
    });
    assert!(!history, "an answer stays out of prompt history");
    assert!(
        strip(&ws, &mut vcx).is_none(),
        "the strip leaves with the question"
    );
}

/// Once released, a typed prompt would wait behind the question for
/// good, so Enter warns first; "Skip and send this" declines, then sends.
#[gpui::test]
fn enter_after_release_arms_the_prompt_warning_and_skip_and_send_declines_then_sends(
    cx: &mut TestAppContext,
) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, path_question());
    vcx.update(|_, cx| ws.update(cx, |ws, cx| ws.release_question_for_prompt(pane, cx)));
    vcx.run_until_parked();

    type_text(&ws, &mut vcx, "do the other thing");
    press_enter(&ws, &mut vcx);
    assert_eq!(probes[0].answer(), None, "nothing is sent yet");
    assert!(queued(&ws, &mut vcx, pane).is_empty());
    assert_eq!(composer_text(&ws, &mut vcx), "do the other thing");
    let warned = strip(&ws, &mut vcx).expect("strip");
    assert_eq!(warned.pending_prompt.as_deref(), Some("do the other thing"));

    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.skip_question_and_send_prompt(pane, window, cx)
        })
    });
    vcx.run_until_parked();
    assert_eq!(probes[0].answer(), Some(QuestionAnswer::Decline));
    assert_eq!(
        queued(&ws, &mut vcx, pane),
        vec!["do the other thing".to_owned()],
        "the prompt follows the turn the decline lets finish"
    );
    assert_eq!(composer_text(&ws, &mut vcx), "");
    assert!(strip(&ws, &mut vcx).is_none());
}

/// "Keep queued", and editing the text after the warning, both leave the
/// question waiting.
#[gpui::test]
fn keep_queued_sends_behind_the_question_and_an_edit_disarms_the_warning(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, path_question());
    vcx.update(|_, cx| ws.update(cx, |ws, cx| ws.release_question_for_prompt(pane, cx)));
    type_text(&ws, &mut vcx, "first");
    press_enter(&ws, &mut vcx);
    type_text(&ws, &mut vcx, "first, edited");
    assert_eq!(strip(&ws, &mut vcx).expect("strip").pending_prompt, None);

    press_enter(&ws, &mut vcx);
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.keep_prompt_queued_behind_question(pane, window, cx)
        })
    });
    vcx.run_until_parked();
    assert_eq!(
        queued(&ws, &mut vcx, pane),
        vec!["first, edited".to_owned()]
    );
    assert_eq!(probes[0].answer(), None, "the question still waits");
    assert_eq!(composer_text(&ws, &mut vcx), "");
}

/// Review Focus 2: a question arriving over a typed draft leaves it alone.
#[gpui::test]
fn a_question_over_a_typed_draft_stays_released(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let _probes = connect(&ws, &mut vcx, pane, &[1]);
    type_text(&ws, &mut vcx, "a follow-up I was writing");
    ask(&ws, &mut vcx, pane, 1, path_question());

    let shown = strip(&ws, &mut vcx).expect("the strip offers the question");
    assert!(!shown.composer.bound);
    assert_eq!(composer_text(&ws, &mut vcx), "a follow-up I was writing");
}

/// The first Esc leaves answer mode, the next one stops the agent.
#[gpui::test]
fn first_escape_releases_and_second_escape_stops(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, path_question());

    press_escape(&mut vcx);
    let released = strip(&ws, &mut vcx).expect("still waiting");
    assert!(!released.composer.bound);
    assert!(released.esc_again, "the strip says what the next Esc does");
    assert_eq!(probes[0].answer(), None);

    press_escape(&mut vcx);
    assert_eq!(probes[0].answer(), Some(QuestionAnswer::Cancel));
    assert!(strip(&ws, &mut vcx).is_none());
}

/// Writing from the card over a prompt draft keeps the draft as the
/// start of the answer; over an empty composer it loads the field's answer.
#[gpui::test]
fn writing_over_a_draft_keeps_the_draft_as_the_answer_start(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let _probes = connect(&ws, &mut vcx, pane, &[1]);
    type_text(&ws, &mut vcx, "use the staging path");
    ask(&ws, &mut vcx, pane, 1, path_question());
    assert!(!summary(&ws, &mut vcx, pane).expect("open").bound);

    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.focus_composer_for_question(pane, 1, Some("old".into()), window, cx)
        })
    });
    vcx.run_until_parked();
    let bound = summary(&ws, &mut vcx, pane).expect("open");
    assert!(bound.bound);
    assert!(bound.draft_became_answer);
    assert_eq!(composer_text(&ws, &mut vcx), "use the staging path");

    vcx.update(|_, cx| ws.update(cx, |ws, cx| ws.release_question_for_prompt(pane, cx)));
    type_text(&ws, &mut vcx, "");
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.focus_composer_for_question(pane, 1, Some("old".into()), window, cx)
        })
    });
    vcx.run_until_parked();
    assert_eq!(
        composer_text(&ws, &mut vcx),
        "old",
        "an empty composer edits"
    );
    assert!(
        !summary(&ws, &mut vcx, pane)
            .expect("open")
            .draft_became_answer
    );
}

#[gpui::test]
fn release_leaves_uncommitted_text_in_the_composer(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let _probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, path_question());
    type_text(&ws, &mut vcx, "half an answer");

    vcx.update(|_, cx| ws.update(cx, |ws, cx| ws.release_question_for_prompt(pane, cx)));
    vcx.run_until_parked();

    assert!(!summary(&ws, &mut vcx, pane).expect("open").bound);
    assert_eq!(composer_text(&ws, &mut vcx), "half an answer");
    assert_eq!(
        field_text(&ws, &mut vcx, pane, 1, 0),
        None,
        "nothing committed"
    );
}

/// Review Focus 5: each pane keeps its own uncommitted text and binding.
#[gpui::test]
fn switching_panes_keeps_each_panes_text_and_binding(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let first = add_pane(&ws, &mut vcx);
    let mut first_probes = connect(&ws, &mut vcx, first, &[1]);
    ask(&ws, &mut vcx, first, 1, path_question());
    type_text(&ws, &mut vcx, "for the first");

    let second = add_pane(&ws, &mut vcx);
    assert_eq!(composer_text(&ws, &mut vcx), "", "no text leaks across");
    let mut second_probes = connect(&ws, &mut vcx, second, &[1]);
    ask(&ws, &mut vcx, second, 1, path_question());
    assert!(summary(&ws, &mut vcx, second).expect("open").bound);
    type_text(&ws, &mut vcx, "for the second");

    focus(&ws, &mut vcx, first);
    assert_eq!(composer_text(&ws, &mut vcx), "for the first");
    assert!(summary(&ws, &mut vcx, first).expect("open").bound);
    assert_eq!(strip(&ws, &mut vcx).expect("strip").pane_id, first);
    press_enter(&ws, &mut vcx);
    assert_eq!(
        first_probes[0].answer(),
        text_answer("path", "for the first")
    );
    assert_eq!(second_probes[0].answer(), None);

    focus(&ws, &mut vcx, second);
    assert_eq!(composer_text(&ws, &mut vcx), "for the second");
    press_enter(&ws, &mut vcx);
    assert_eq!(
        second_probes[0].answer(),
        text_answer("path", "for the second")
    );
}

#[gpui::test]
async fn slash_completion_is_off_while_bound(cx: &mut TestAppContext) {
    use crate::ui::{CompletionProvider as _, Rope};
    use crate::workspace::main_area::bottom_dock::slash_command::SlashCommandProvider;

    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let _probes = connect(&ws, &mut vcx, pane, &[1]);
    let provider = SlashCommandProvider {
        workspace: ws.downgrade(),
    };
    let completes = |vcx: &mut VisualTestContext| {
        vcx.update(|window, cx| {
            let input = ws.read(cx).input_dock.input.clone();
            input.update(cx, |_, cx| {
                let trigger = provider.is_completion_trigger(1, "/", cx);
                let task = provider.completions(
                    &Rope::from("/c"),
                    2,
                    lsp_types::CompletionContext {
                        trigger_kind: lsp_types::CompletionTriggerKind::INVOKED,
                        trigger_character: None,
                    },
                    window,
                    cx,
                );
                (trigger, task)
            })
        })
    };
    let count = |response: lsp_types::CompletionResponse| match response {
        lsp_types::CompletionResponse::Array(items) => items.len(),
        lsp_types::CompletionResponse::List(list) => list.items.len(),
    };

    let (trigger, task) = completes(&mut vcx);
    assert!(trigger);
    assert_eq!(count(task.await.unwrap()), 1, "/clear before any question");

    ask(&ws, &mut vcx, pane, 1, path_question());
    let (trigger, task) = completes(&mut vcx);
    assert!(!trigger, "a bound composer opens no menu");
    assert_eq!(count(task.await.unwrap()), 0);
}

/// A bound composer's text is the answer however it starts: `/usr/bin` is
/// a path, not a slash command.
#[gpui::test]
fn a_slash_leading_answer_is_sent_as_the_answer(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, path_question());

    type_text(&ws, &mut vcx, "/usr/bin");
    press_enter(&ws, &mut vcx);

    assert_eq!(probes[0].answer(), text_answer("path", "/usr/bin"));
    assert!(queued(&ws, &mut vcx, pane).is_empty());
}

/// A card pick that moves focus on commits what the composer held to
/// the field it was typed for, then loads the new field's own text.
#[gpui::test]
fn a_card_pick_that_moves_focus_keeps_typed_text_on_its_field(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let _probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, color_then_reason());
    assert_eq!(summary(&ws, &mut vcx, pane).expect("open").field, 0);
    type_text(&ws, &mut vcx, "teal");

    let view = vcx.update(|_, cx| ws.read(cx).agent_chat_view(pane).expect("chat").clone());
    vcx.update(|_, cx| view.update(cx, |v, cx| v.pick_question_option(1, 0, 0, cx)));
    vcx.run_until_parked();

    assert_eq!(summary(&ws, &mut vcx, pane).expect("open").field, 1);
    assert_eq!(
        field_text(&ws, &mut vcx, pane, 1, 0).as_deref(),
        Some("teal")
    );
    assert_eq!(field_text(&ws, &mut vcx, pane, 1, 1), None);
    assert_eq!(composer_text(&ws, &mut vcx), "", "the text stayed behind");

    // Back to the first field: the reason goes to its own field, and the
    // composer picks up the note already committed there.
    type_text(&ws, &mut vcx, "blue is taken");
    vcx.update(|_, cx| view.update(cx, |v, cx| v.bind_question_composer(1, Some(0), false, cx)));
    vcx.run_until_parked();
    assert_eq!(
        field_text(&ws, &mut vcx, pane, 1, 1).as_deref(),
        Some("blue is taken")
    );
    assert_eq!(composer_text(&ws, &mut vcx), "teal");
}

/// Once the warning is up, more Enters change
/// nothing; only the strip's buttons send the prompt on.
#[gpui::test]
fn a_second_enter_after_the_warning_does_not_queue(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, path_question());
    vcx.update(|_, cx| ws.update(cx, |ws, cx| ws.release_question_for_prompt(pane, cx)));
    type_text(&ws, &mut vcx, "later");

    press_enter_key(&mut vcx);
    press_enter_key(&mut vcx);
    press_enter_key(&mut vcx);

    assert!(queued(&ws, &mut vcx, pane).is_empty());
    assert_eq!(probes[0].answer(), None);
    assert_eq!(
        composer_text(&ws, &mut vcx),
        "later",
        "no line breaks pile up"
    );
    assert_eq!(
        strip(&ws, &mut vcx)
            .expect("strip")
            .pending_prompt
            .as_deref(),
        Some("later")
    );
}

/// An Enter the question refuses (a yes/no takes no text) leaves the
/// composer exactly as it was, through the real key path.
#[gpui::test]
fn a_refused_enter_keeps_the_text_without_a_line_break(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, yes_no_question());
    type_text(&ws, &mut vcx, "maybe");

    press_enter_key(&mut vcx);
    press_enter_key(&mut vcx);

    assert_eq!(composer_text(&ws, &mut vcx), "maybe");
    assert_eq!(probes[0].answer(), None);
    assert!(queued(&ws, &mut vcx, pane).is_empty());
}

/// Writing from the card opens a shut bottom dock on the composer.
#[gpui::test]
fn writing_from_the_card_opens_a_shut_dock(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace_with_dock_shut(cx);
    let pane = add_pane(&ws, &mut vcx);
    let _probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, path_question());
    assert!(!vcx.update(|_, cx| ws.read(cx).docks.bottom.read(cx).is_open));

    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.focus_composer_for_question(pane, 1, None, window, cx)
        })
    });
    vcx.run_until_parked();

    let (open, input) = vcx.update(|_, cx| {
        let ws = ws.read(cx);
        (ws.docks.bottom.read(cx).is_open, ws.input_dock.visible)
    });
    assert!(open, "the dock opens");
    assert!(input, "on the composer");
    assert!(summary(&ws, &mut vcx, pane).expect("open").bound);
}

/// Text typed on a yes/no has no field to go to when a pick moves
/// focus on; it must not become the next field's answer.
#[gpui::test]
fn text_a_field_refuses_does_not_follow_focus(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, yes_no_then_reason());
    type_text(&ws, &mut vcx, "hmm");
    let view = vcx.update(|_, cx| ws.read(cx).agent_chat_view(pane).expect("chat").clone());
    vcx.update(|_, cx| view.update(cx, |v, cx| v.set_question_flag(1, 0, true, cx)));
    vcx.run_until_parked();
    assert_eq!(summary(&ws, &mut vcx, pane).expect("open").field, 1);

    press_enter(&ws, &mut vcx);
    assert_eq!(field_text(&ws, &mut vcx, pane, 1, 1), None);
    assert_eq!(probes[0].answer(), None);
    assert_eq!(composer_text(&ws, &mut vcx), "hmm");

    // Emptied, the composer belongs to the new field.
    type_text(&ws, &mut vcx, "");
    type_text(&ws, &mut vcx, "because");
    press_enter(&ws, &mut vcx);
    assert_eq!(
        probes[0].answer(),
        Some(QuestionAnswer::Submit(BTreeMap::from([
            ("ok".to_owned(), QuestionValue::Bool(true)),
            ("why".to_owned(), QuestionValue::Text("because".into())),
        ])))
    );
}

/// Released answer text taken up again is still the answer, not a
/// prompt draft that "became the answer".
#[gpui::test]
fn rebinding_released_answer_text_is_no_draft_turned_answer(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let _probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, path_question());
    type_text(&ws, &mut vcx, "src/");
    vcx.update(|_, cx| ws.update(cx, |ws, cx| ws.release_question_for_prompt(pane, cx)));
    vcx.run_until_parked();

    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.answer_question_from_strip(pane, window, cx))
    });
    vcx.run_until_parked();

    let bound = summary(&ws, &mut vcx, pane).expect("open");
    assert!(bound.bound);
    assert!(!bound.draft_became_answer);
    assert_eq!(composer_text(&ws, &mut vcx), "src/");
}

/// Question ids restart with every connection: text typed for question 1
/// of an earlier session is a prompt draft to question 1 of the next.
#[gpui::test]
fn answer_text_from_an_earlier_session_is_a_draft(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let _old = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, path_question());
    type_text(&ws, &mut vcx, "src/");
    vcx.update(|_, cx| ws.update(cx, |ws, cx| ws.release_question_for_prompt(pane, cx)));
    vcx.run_until_parked();

    let view = vcx.update(|_, cx| ws.read(cx).agent_chat_view(pane).expect("chat").clone());
    vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.reset_for_new_session(cx);
            v.begin_connect(None, cx);
        })
    });
    let _new = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, path_question());
    vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| ws.answer_question_from_strip(pane, window, cx))
    });
    vcx.run_until_parked();

    let bound = summary(&ws, &mut vcx, pane).expect("open");
    assert!(bound.bound);
    assert!(
        bound.draft_became_answer,
        "the old session's text is a draft"
    );
}

/// The empty-draft hint follows a pane's saved draft after focus leaves:
/// a question arriving there leaves the draft alone.
#[gpui::test]
fn a_question_at_a_pane_left_with_a_draft_stays_released(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let first = add_pane(&ws, &mut vcx);
    let _probes = connect(&ws, &mut vcx, first, &[1]);
    type_text(&ws, &mut vcx, "a draft for the first");
    let _second = add_pane(&ws, &mut vcx);

    ask(&ws, &mut vcx, first, 1, path_question());

    assert!(!summary(&ws, &mut vcx, first).expect("open").bound);
}

fn says_text_not_taken(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> bool {
    strip(ws, vcx).is_some_and(|s| s.text_not_taken)
}

/// A refused Enter on a yes/no says why on the strip and keeps the text;
/// the reason goes once the composer empties or the question settles.
#[gpui::test]
fn a_refused_enter_says_why_until_the_composer_empties(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, yes_no_question());
    type_text(&ws, &mut vcx, "maybe");
    assert!(!says_text_not_taken(&ws, &mut vcx), "not before an Enter");

    press_enter_key(&mut vcx);
    assert!(says_text_not_taken(&ws, &mut vcx));
    assert_eq!(composer_text(&ws, &mut vcx), "maybe");
    type_text(&ws, &mut vcx, "maybe not");
    assert!(
        says_text_not_taken(&ws, &mut vcx),
        "an edit keeps the reason"
    );

    type_text(&ws, &mut vcx, "");
    assert!(
        !says_text_not_taken(&ws, &mut vcx),
        "an empty composer clears it"
    );

    // Refused again, then the question settles by a pick.
    type_text(&ws, &mut vcx, "maybe");
    press_enter_key(&mut vcx);
    assert!(says_text_not_taken(&ws, &mut vcx));
    let view = vcx.update(|_, cx| ws.read(cx).agent_chat_view(pane).expect("chat").clone());
    vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.set_question_flag(1, 0, true, cx);
            v.submit_question(1, cx);
        })
    });
    vcx.run_until_parked();
    assert!(probes[0].answer().is_some());
    assert!(
        !says_text_not_taken(&ws, &mut vcx),
        "a settled question clears it"
    );
    assert_eq!(composer_text(&ws, &mut vcx), "maybe", "the text is kept");
}

/// While a field swap is pending, a refused Enter says why too; moving
/// the field or emptying the composer clears the reason.
#[gpui::test]
fn a_pending_swap_enter_says_why_until_cleared(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let _probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, yes_no_then_reason());
    type_text(&ws, &mut vcx, "hmm");
    let view = vcx.update(|_, cx| ws.read(cx).agent_chat_view(pane).expect("chat").clone());
    let bind = |vcx: &mut VisualTestContext, field: usize| {
        vcx.update(|_, cx| {
            view.update(cx, |v, cx| {
                v.bind_question_composer(1, Some(field), false, cx)
            })
        });
        vcx.run_until_parked();
    };
    vcx.update(|_, cx| view.update(cx, |v, cx| v.set_question_flag(1, 0, true, cx)));
    vcx.run_until_parked();
    assert_eq!(summary(&ws, &mut vcx, pane).expect("open").field, 1);
    assert!(!says_text_not_taken(&ws, &mut vcx), "not before an Enter");

    press_enter_key(&mut vcx);
    assert!(says_text_not_taken(&ws, &mut vcx));
    assert_eq!(composer_text(&ws, &mut vcx), "hmm");

    bind(&mut vcx, 0);
    assert_eq!(summary(&ws, &mut vcx, pane).expect("open").field, 0);
    assert!(
        !says_text_not_taken(&ws, &mut vcx),
        "a field move clears it"
    );

    bind(&mut vcx, 1);
    press_enter_key(&mut vcx);
    assert!(says_text_not_taken(&ws, &mut vcx));
    type_text(&ws, &mut vcx, "");
    assert!(
        !says_text_not_taken(&ws, &mut vcx),
        "an empty composer clears it"
    );
    assert_eq!(field_text(&ws, &mut vcx, pane, 1, 1), None);
}

/// A field swap that empties a pane's saved draft while another pane holds
/// the composer leaves that pane's composer empty: its next question binds.
#[gpui::test]
fn a_draft_emptied_in_the_background_lets_the_next_question_bind(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let first = add_pane(&ws, &mut vcx);
    let _probes = connect(&ws, &mut vcx, first, &[1, 2]);
    ask(&ws, &mut vcx, first, 1, color_then_reason());
    ask(&ws, &mut vcx, first, 2, path_question());
    type_text(&ws, &mut vcx, "teal");
    let _second = add_pane(&ws, &mut vcx);

    let view = vcx.update(|_, cx| ws.read(cx).agent_chat_view(first).expect("chat").clone());
    // The pick moves focus on; "teal" goes to the first field and the
    // second field's empty answer replaces the saved draft.
    vcx.update(|_, cx| view.update(cx, |v, cx| v.pick_question_option(1, 0, 0, cx)));
    vcx.run_until_parked();
    assert_eq!(
        field_text(&ws, &mut vcx, first, 1, 0).as_deref(),
        Some("teal")
    );

    vcx.update(|_, cx| view.update(cx, |v, cx| v.skip_question(1, cx)));
    vcx.run_until_parked();
    let next = summary(&ws, &mut vcx, first).expect("open");
    assert_eq!(next.question_id, 2);
    assert!(next.bound, "the emptied composer is free for it");
}

/// The card's send takes the text typed in the bound composer with it:
/// a pick moved focus to the reason, typed but not yet entered.
#[gpui::test]
fn the_card_send_commits_the_bound_composer_text_first(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, color_then_reason());
    let view = vcx.update(|_, cx| ws.read(cx).agent_chat_view(pane).expect("chat").clone());
    vcx.update(|_, cx| view.update(cx, |v, cx| v.pick_question_option(1, 0, 0, cx)));
    vcx.run_until_parked();
    type_text(&ws, &mut vcx, "because X");

    vcx.update(|_, cx| view.update(cx, |v, cx| v.submit_question_from_card(1, cx)));
    vcx.run_until_parked();

    assert_eq!(
        probes[0].answer(),
        Some(QuestionAnswer::Submit(BTreeMap::from([
            ("color".to_owned(), QuestionValue::Text("red".into())),
            ("why".to_owned(), QuestionValue::Text("because X".into())),
        ])))
    );
    assert_eq!(composer_text(&ws, &mut vcx), "", "the text went with it");
}

/// Text the bound field cannot take keeps the card's send from going: the
/// text stays and the strip says why.
#[gpui::test]
fn the_card_send_over_text_a_field_refuses_sends_nothing(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, yes_no_then_reason());
    type_text(&ws, &mut vcx, "maybe");
    let view = vcx.update(|_, cx| ws.read(cx).agent_chat_view(pane).expect("chat").clone());

    vcx.update(|_, cx| view.update(cx, |v, cx| v.submit_question_from_card(1, cx)));
    vcx.run_until_parked();

    assert_eq!(probes[0].answer(), None);
    assert_eq!(composer_text(&ws, &mut vcx), "maybe");
    assert!(says_text_not_taken(&ws, &mut vcx));
}

/// Released, an Enter carrying only attachments is a prompt too: it is held
/// back with the warning rather than queued silently behind the question.
#[gpui::test]
fn an_attachment_only_enter_is_held_back_behind_a_released_question(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, path_question());
    vcx.update(|_, cx| ws.update(cx, |ws, cx| ws.release_question_for_prompt(pane, cx)));
    let view = vcx.update(|_, cx| ws.read(cx).agent_chat_view(pane).expect("chat").clone());
    vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.add_attachments(
                vec![daruda_acp::PromptAttachment {
                    name: "notes".into(),
                    content: daruda_acp::AttachmentContent::ResourceLink {
                        uri: "urn:test:notes".into(),
                    },
                }],
                cx,
            )
            .expect("attached")
        })
    });

    press_enter_key(&mut vcx);

    assert!(queued(&ws, &mut vcx, pane).is_empty(), "nothing queued yet");
    assert_eq!(probes[0].answer(), None);
    assert_eq!(
        strip(&ws, &mut vcx)
            .expect("strip")
            .pending_prompt
            .as_deref(),
        Some("")
    );
}
