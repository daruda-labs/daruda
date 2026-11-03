//! Answering a pending question from the keyboard through real key
//! dispatch: digits, ↑↓, Tab and Shift+Tab, and the empty Enter.

use std::collections::BTreeMap;

use daruda_acp::{
    AcpEvent, ModeStateView, QuestionAnswer, QuestionFieldKind, QuestionOption, QuestionRequest,
    QuestionValue, SessionModeView,
};
use gpui::{Entity, TestAppContext, VisualTestContext};

use super::composer::{
    add_pane, ask, color_then_reason, composer_text, connect, field, press_enter_key, type_text,
    workspace,
};
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

/// One single-select question with its "Other" box: a pick sends it.
fn color_question() -> QuestionRequest {
    let option = |value: &str| QuestionOption {
        value: value.into(),
        label: value.into(),
        description: None,
    };
    QuestionRequest {
        message: "Which color?".into(),
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
        ],
    }
}

/// The workspace with the app's key bindings, which the question keys need.
fn keyed_workspace(
    cx: &mut TestAppContext,
) -> (tempfile::TempDir, Entity<Workspace>, VisualTestContext) {
    let (dir, ws, mut vcx) = workspace(cx);
    vcx.update(|_, cx| crate::bind_keys::register_static_bindings(cx));
    (dir, ws, vcx)
}

fn keys(vcx: &mut VisualTestContext, keys: &str) {
    vcx.simulate_keystrokes(keys);
    vcx.run_until_parked();
}

/// The oldest question's focused field and highlighted row.
fn focus(
    ws: &Entity<Workspace>,
    vcx: &mut VisualTestContext,
    pane: PaneId,
) -> Option<(usize, Option<usize>)> {
    vcx.update(|_, cx| {
        ws.read(cx)
            .agent_chat_view(pane)
            .expect("chat pane")
            .read(cx)
            .question_focus_for_test()
    })
}

fn color_answer(color: &str) -> Option<QuestionAnswer> {
    Some(QuestionAnswer::Submit(BTreeMap::from([(
        "color".to_owned(),
        QuestionValue::Text(color.into()),
    )])))
}

/// A digit picks but never sends: it may be the start of a prompt. The
/// empty Enter then sends the answered form.
#[gpui::test]
fn digit_in_an_empty_bound_composer_picks_without_sending(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, color_question());

    keys(&mut vcx, "2");
    assert_eq!(focus(&ws, &mut vcx, pane), Some((0, Some(1))));

    assert_eq!(probes[0].answer(), None, "a digit alone sends nothing");
    assert_eq!(composer_text(&ws, &mut vcx), "", "the digit is not typed");

    keys(&mut vcx, "enter");

    assert_eq!(probes[0].answer(), color_answer("blue"));
}

/// A digit is a choice only in an empty bound composer; with text, or
/// released, it is a character like any other.
#[gpui::test]
fn digit_in_a_non_empty_composer_types(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, color_question());

    type_text(&ws, &mut vcx, "day ");
    keys(&mut vcx, "2");
    let typed = composer_text(&ws, &mut vcx);
    assert!(typed.contains('2') && typed.contains("day "), "{typed:?}");
    assert_eq!(probes[0].answer(), None);

    type_text(&ws, &mut vcx, "");
    keys(&mut vcx, "escape");
    keys(&mut vcx, "1");
    assert_eq!(composer_text(&ws, &mut vcx), "1", "released, a digit types");
    assert_eq!(probes[0].answer(), None);
}

/// Review Focus 1: IME marked text sits in the composer's value before it is
/// committed, so the composer is not empty and a digit cannot choose.
#[gpui::test]
fn a_digit_during_ime_composition_does_not_pick(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler as _;

    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, color_question());
    vcx.update(|window, cx| {
        let input = ws.read(cx).input_dock.input.clone();
        input.update(cx, |state, cx| {
            state.replace_and_mark_text_in_range(None, "ㄱ", None, window, cx)
        });
    });
    vcx.run_until_parked();

    keys(&mut vcx, "1");

    assert_eq!(probes[0].answer(), None);
    assert_ne!(
        composer_text(&ws, &mut vcx),
        "",
        "the digit went to the text"
    );
}

#[gpui::test]
fn arrow_up_in_an_empty_bound_composer_moves_the_highlight_not_history(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let _probes = connect(&ws, &mut vcx, pane, &[1]);
    vcx.update(|_, cx| {
        ws.update(cx, |ws, _| {
            let lane = ws.active_ref();
            ws.lane_scoped_mut(lane).input_history.push("an old prompt");
        })
    });
    ask(&ws, &mut vcx, pane, 1, color_question());

    keys(&mut vcx, "up");
    assert_eq!(
        focus(&ws, &mut vcx, pane),
        Some((0, Some(2))),
        "the write row"
    );
    assert_eq!(composer_text(&ws, &mut vcx), "", "no history recall");
    keys(&mut vcx, "up");
    assert_eq!(focus(&ws, &mut vcx, pane), Some((0, Some(1))));
    keys(&mut vcx, "down");
    assert_eq!(focus(&ws, &mut vcx, pane), Some((0, Some(2))));

    type_text(&ws, &mut vcx, "x");
    keys(&mut vcx, "up");
    assert_eq!(
        focus(&ws, &mut vcx, pane),
        Some((0, None)),
        "typing put the highlight out, and ↑ is the composer's own again"
    );
    assert_eq!(composer_text(&ws, &mut vcx), "an old prompt");
}

/// Ruling 4: while bound, Tab and Shift+Tab move between fields (the text
/// typed for one stays with it); released, Shift+Tab cycles the mode.
#[gpui::test]
fn shift_tab_moves_fields_while_bound_and_cycles_mode_otherwise(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let _probes = connect(&ws, &mut vcx, pane, &[1]);
    let mode = |id: &str| SessionModeView {
        id: id.into(),
        name: id.into(),
        description: None,
    };
    vcx.update(|_, cx| {
        ws.update(cx, |ws, cx| {
            ws.fold_agent_chat_event(
                pane,
                AcpEvent::ModeChanged {
                    state: ModeStateView {
                        available: vec![mode("default"), mode("plan")],
                        current: "default".into(),
                    },
                },
                cx,
            )
        })
    });
    let current_mode = |vcx: &mut VisualTestContext| {
        vcx.update(|_, cx| {
            ws.read(cx)
                .agent_chat_view(pane)
                .expect("chat pane")
                .read(cx)
                .session_config()
                .current_mode_id()
                .map(str::to_owned)
        })
    };
    ask(&ws, &mut vcx, pane, 1, color_then_reason());

    type_text(&ws, &mut vcx, "teal");
    keys(&mut vcx, "tab");
    assert_eq!(focus(&ws, &mut vcx, pane), Some((1, None)));
    assert_eq!(composer_text(&ws, &mut vcx), "", "the next field's text");
    type_text(&ws, &mut vcx, "because");
    keys(&mut vcx, "shift-tab");
    assert_eq!(focus(&ws, &mut vcx, pane), Some((0, None)));
    assert_eq!(composer_text(&ws, &mut vcx), "teal", "its own text returns");
    assert_eq!(current_mode(&mut vcx).as_deref(), Some("default"));
    keys(&mut vcx, "shift-tab");
    assert_eq!(
        focus(&ws, &mut vcx, pane),
        Some((0, None)),
        "no field before"
    );
    assert_eq!(current_mode(&mut vcx).as_deref(), Some("default"));

    keys(&mut vcx, "escape");
    keys(&mut vcx, "shift-tab");
    assert_eq!(current_mode(&mut vcx).as_deref(), Some("plan"));
    assert_eq!(focus(&ws, &mut vcx, pane), Some((0, None)));
}

/// §2.4: while bound, an empty Enter chooses the highlighted row; the
/// parked queue's resume gesture waits until the question is done.
#[gpui::test]
fn empty_enter_picks_the_highlighted_row_before_resuming_the_queue(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    let view = vcx.update(|_, cx| ws.read(cx).agent_chat_view(pane).expect("chat").clone());
    vcx.update(|_, cx| {
        ws.update(cx, |ws, cx| {
            ws.send_agent_prompt_text(pane, "parked".to_string(), cx)
        });
        view.update(cx, |v, cx| {
            v.cancel_turn(cx);
            v.set_turn_in_flight();
        });
    });
    ask(&ws, &mut vcx, pane, 1, color_question());

    keys(&mut vcx, "down");
    press_enter_key(&mut vcx);

    assert_eq!(probes[0].answer(), color_answer("red"));
    vcx.update(|_, cx| {
        assert!(
            !view.read(cx).resume_armed(),
            "the Enter answered; it did not arm the resume"
        );
    });
}

/// Typing ends keyboard choosing, so text typed and deleted again
/// leaves no row for an empty Enter to pick.
#[gpui::test]
fn typing_clears_the_highlight(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, color_question());

    keys(&mut vcx, "down");
    assert_eq!(focus(&ws, &mut vcx, pane), Some((0, Some(0))));
    keys(&mut vcx, "x");
    assert_eq!(focus(&ws, &mut vcx, pane), Some((0, None)));
    keys(&mut vcx, "backspace");
    assert_eq!(composer_text(&ws, &mut vcx), "");
    press_enter_key(&mut vcx);

    assert_eq!(probes[0].answer(), None, "nothing was lit to pick");
}

fn send_lit(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, pane: PaneId) -> bool {
    vcx.update(|_, cx| {
        ws.read(cx)
            .agent_chat_view(pane)
            .expect("chat pane")
            .read(cx)
            .question_composer()
            .is_some_and(|q| q.send_highlighted)
    })
}

/// Tab on the last field commits its text and lights send, which
/// holds no text; Shift+Tab brings the field and its text back, and Enter
/// on send sends what the button would.
#[gpui::test]
fn tab_past_the_last_field_lands_on_send(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, color_then_reason());

    keys(&mut vcx, "tab");
    type_text(&ws, &mut vcx, "because");
    keys(&mut vcx, "tab");
    assert!(send_lit(&ws, &mut vcx, pane));
    assert_eq!(composer_text(&ws, &mut vcx), "", "send takes no text");
    keys(&mut vcx, "shift-tab");
    assert!(!send_lit(&ws, &mut vcx, pane));
    assert_eq!(focus(&ws, &mut vcx, pane), Some((1, None)));
    assert_eq!(composer_text(&ws, &mut vcx), "because");

    keys(&mut vcx, "tab");
    press_enter_key(&mut vcx);
    assert_eq!(
        probes[0].answer(),
        Some(QuestionAnswer::Submit(BTreeMap::from([(
            "why".to_owned(),
            QuestionValue::Text("because".into()),
        )]))),
        "the color left blank is optional"
    );
}

/// A text-only field has no rows to number, so its digits are typed.
#[gpui::test]
fn a_digit_on_a_text_field_is_typed(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    let days = QuestionRequest {
        message: "How long?".into(),
        tool_call_id: None,
        delivery: daruda_acp::QuestionDelivery::Blocking,
        fields: vec![field("days", "Days", QuestionFieldKind::Text, None)],
    };
    ask(&ws, &mut vcx, pane, 1, days);

    keys(&mut vcx, "5");

    assert_eq!(composer_text(&ws, &mut vcx), "5");
    assert_eq!(probes[0].answer(), None);
}

/// Space toggles a multi-select option only; on a single select it is a
/// typed space, and it picks nothing.
#[gpui::test]
fn space_on_a_single_select_is_typed(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, color_question());

    keys(&mut vcx, "down");
    keys(&mut vcx, "space");

    assert_eq!(composer_text(&ws, &mut vcx), " ");
    assert_eq!(probes[0].answer(), None);
}

/// With modifier-to-send on, plain Enter is a line break, so the answer
/// placeholder names the send chord instead.
#[gpui::test]
fn the_answer_placeholder_names_the_modifier_send(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let _probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, color_question());

    let placeholder = |vcx: &mut VisualTestContext, modifier: bool| {
        vcx.update(|_, cx| {
            ws.update(cx, |ws, cx| {
                ws.mirrors.agent.use_modifier_to_send = modifier;
                ws.compute_input_placeholder(cx)
            })
        })
    };
    use crate::surface::strings::bottom_dock as s;
    assert_eq!(placeholder(&mut vcx, false), s::question_placeholder());
    assert_eq!(
        placeholder(&mut vcx, true),
        s::question_placeholder_modifier()
    );
}

/// macOS IMEs can deliver digits directly as text, bypassing keybindings.
#[gpui::test]
fn ime_first_digit_picks_without_inserting_or_sending(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, color_question());
    commit_text(&ws, &mut vcx, "2");
    assert_eq!(composer_text(&ws, &mut vcx), "");
    assert_eq!(probes[0].answer(), None);
    keys(&mut vcx, "enter");
    assert_eq!(probes[0].answer(), color_answer("blue"));
}

fn commit_text(ws: &Entity<Workspace>, vcx: &mut VisualTestContext, text: &str) {
    use gpui::EntityInputHandler as _;
    vcx.update(|window, cx| {
        let input = ws.read(cx).input_dock.input.clone();
        input.update(cx, |state, cx| {
            state.replace_text_in_range(None, text, window, cx)
        });
    });
    vcx.run_until_parked();
}

/// IME-first Space toggles a multi-select, while longer commits stay text.
#[gpui::test]
fn ime_first_space_toggles_and_multi_character_commits_stay_text(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    let mut request = color_question();
    let QuestionFieldKind::SingleSelect(options) = request.fields[0].kind.clone() else {
        panic!("select")
    };
    request.fields[0].kind = QuestionFieldKind::MultiSelect(options);
    ask(&ws, &mut vcx, pane, 1, request);
    keys(&mut vcx, "down");
    commit_text(&ws, &mut vcx, " ");
    assert_eq!(composer_text(&ws, &mut vcx), "");
    assert_eq!(probes[0].answer(), None);
    commit_text(&ws, &mut vcx, "12");
    assert_eq!(composer_text(&ws, &mut vcx), "12");
    type_text(&ws, &mut vcx, "");
    keys(&mut vcx, "enter");
    vcx.update(|_, cx| {
        let view = ws.read(cx).agent_chat_view(pane).expect("chat").clone();
        view.update(cx, |v, cx| v.submit_question_from_card(1, cx));
    });
    vcx.run_until_parked();
    assert_eq!(
        probes[0].answer(),
        Some(QuestionAnswer::Submit(BTreeMap::from([(
            "color".to_owned(),
            QuestionValue::Choices(vec!["red".into()])
        )])))
    );
}

/// A composition must not be replaced with history or interpreted as a pick.
#[gpui::test]
fn marked_text_bypasses_question_shortcuts_and_history(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler as _;
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, color_question());
    vcx.update(|window, cx| {
        ws.update(cx, |ws, _| {
            let lane = ws.active_ref();
            ws.lane_scoped_mut(lane).input_history.push("old prompt");
        });
        let input = ws.read(cx).input_dock.input.clone();
        input.update(cx, |state, cx| {
            state.replace_and_mark_text_in_range(None, "ㄱ", None, window, cx)
        });
    });
    keys(&mut vcx, "up");
    assert_eq!(composer_text(&ws, &mut vcx), "ㄱ");
    assert_eq!(focus(&ws, &mut vcx, pane), Some((0, None)));
    commit_text(&ws, &mut vcx, "1");
    assert_eq!(composer_text(&ws, &mut vcx), "1");
    assert_eq!(probes[0].answer(), None);
}

/// Native text commits retain normal typing on released and text-only fields.
#[gpui::test]
fn ime_first_digits_fall_through_outside_choice_fields(cx: &mut TestAppContext) {
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, color_then_reason());
    keys(&mut vcx, "tab");
    commit_text(&ws, &mut vcx, "2");
    assert_eq!(composer_text(&ws, &mut vcx), "2");
    type_text(&ws, &mut vcx, "");
    keys(&mut vcx, "escape");
    commit_text(&ws, &mut vcx, "1");
    assert_eq!(composer_text(&ws, &mut vcx), "1");
    assert_eq!(probes[0].answer(), None);
}

/// Programmatic draft restoration and explicit replacements are always text.
#[gpui::test]
fn restored_digits_and_explicit_replacements_do_not_pick(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler as _;
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, color_question());
    type_text(&ws, &mut vcx, "2");
    assert_eq!(composer_text(&ws, &mut vcx), "2");
    type_text(&ws, &mut vcx, "");
    vcx.update(|window, cx| {
        let input = ws.read(cx).input_dock.input.clone();
        input.update(cx, |state, cx| {
            state.replace_text_in_range(Some(0..0), "1", window, cx)
        });
    });
    vcx.run_until_parked();
    assert_eq!(composer_text(&ws, &mut vcx), "1");
    assert_eq!(probes[0].answer(), None);
}

/// IMEs can finalize marked text with unmarkText rather than a replacement.
#[gpui::test]
fn unmarking_composition_updates_the_question_composer_state(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler as _;
    use std::{cell::Cell, rc::Rc};
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, color_question());
    keys(&mut vcx, "down");
    let changes = Rc::new(Cell::new(0));
    let observed = changes.clone();
    vcx.update(|window, cx| {
        let input = ws.read(cx).input_dock.input.clone();
        cx.subscribe(&input, move |_, event: &crate::ui::InputEvent, _| {
            if matches!(event, crate::ui::InputEvent::Change) {
                observed.set(observed.get() + 1);
            }
        })
        .detach();
        input.update(cx, |state, cx| {
            state.replace_and_mark_text_in_range(None, "가나", None, window, cx);
            state.unmark_text(window, cx);
            state.unmark_text(window, cx);
        });
    });
    vcx.run_until_parked();
    assert_eq!(changes.get(), 1, "unmarking twice commits only once");
    assert_eq!(composer_text(&ws, &mut vcx), "가나");
    vcx.update(|_, cx| assert!(!ws.read(cx).question_arrows_take(cx)));
    assert_eq!(focus(&ws, &mut vcx, pane), Some((0, None)));
    assert_eq!(probes[0].answer(), None);
}

#[gpui::test]
fn composition_enter_does_not_submit_and_escape_keeps_answer_mode(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler as _;
    let (_dir, ws, mut vcx) = keyed_workspace(cx);
    let pane = add_pane(&ws, &mut vcx);
    let mut probes = connect(&ws, &mut vcx, pane, &[1]);
    ask(&ws, &mut vcx, pane, 1, color_question());
    vcx.update(|window, cx| {
        let input = ws.read(cx).input_dock.input.clone();
        input.update(cx, |state, cx| {
            state.replace_and_mark_text_in_range(None, "가", None, window, cx)
        });
        ws.update(cx, |ws, cx| {
            assert!(!ws.composer_enter(
                &crate::ui::InputEnter {
                    secondary: false,
                    shift: false
                },
                window,
                cx
            ));
        });
    });
    keys(&mut vcx, "escape");
    vcx.update(|_, cx| assert!(ws.read(cx).focused_question_bound(cx)));
    assert_eq!(probes[0].answer(), None);
}
