//! Delayed prompt results and draft reconciliation regression coverage.
use super::*;

#[gpui::test]
async fn prompt_results_follow_reload_and_hidden_nodes_but_refuse_conflicts(
    cx: &mut TestAppContext,
) {
    use crate::workspace::pages::flows::graph::form::prompt_edit::PromptField;
    let (_lane, ws, path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(ws.active, &path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws.read_with(&vcx, |ws, _| ws.open_graph().unwrap().2);
    for field in [PromptField::Inline, PromptField::File] {
        let edit = view.update_in(&mut vcx, |v, window, cx| {
            v.select_node_for_test(&"design".into(), window, cx);
            v.begin_prompt_edit(field, cx).unwrap()
        });
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}\n# reload\n")).unwrap();
        view.update_in(&mut vcx, |v, window, cx| {
            v.reload(window, cx);
            assert!(v.apply_prompt_edit(&edit, "first draft".into(), window, cx));
            assert!(!v.apply_prompt_edit(&edit, "stale draft".into(), window, cx));
            let next = v.begin_prompt_edit(field, cx).unwrap();
            v.select_node_for_test(&"build".into(), window, cx);
            assert!(v.apply_prompt_edit(&next, "hidden draft".into(), window, cx));
            v.select_node_for_test(&"design".into(), window, cx);
            let states = v.form().unwrap().body_states(cx);
            let input = match field {
                PromptField::Inline => &states.prompt.inline,
                PromptField::File => &states.prompt.file,
            };
            assert_eq!(input.read(cx).value().as_ref(), "hidden draft");
        });
    }
    let edit = view.read_with(&vcx, |v, cx| {
        v.begin_prompt_edit(PromptField::Inline, cx).unwrap()
    });
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("design.md", "external.md")).unwrap();
    view.update_in(&mut vcx, |v, window, cx| {
        v.reload(window, cx);
        assert!(!v.apply_prompt_edit(&edit, "conflicting draft".into(), window, cx));
    });
}

#[gpui::test]
async fn deleting_a_dirty_node_rebases_surviving_drafts(cx: &mut TestAppContext) {
    let (_lane, ws, path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(ws.active, &path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws.read_with(&vcx, |ws, _| ws.open_graph().unwrap().2);
    view.update_in(&mut vcx, |v, window, cx| {
        for node in ["design", "build"] {
            v.select_node_for_test(&node.into(), window, cx);
            v.form()
                .unwrap()
                .body_states(cx)
                .output
                .clone()
                .update(cx, |input, cx| {
                    input.set_value(format!("{node}-draft.md"), window, cx)
                });
        }
    });
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.delete_nodes(&path, view.clone(), vec!["design".into()], window, cx)
    });
    assert!(!view.read_with(&vcx, |v, _| v.draft_conflict()));
    assert!(ws.update_in(&mut vcx, |ws, window, cx| ws.save_flow_editor(
        &path,
        view.clone(),
        window,
        cx
    )));
    let file =
        daruda_flow::parse::parse_flow_file(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(file.nodes.len(), 1);
    assert!(file.nodes[0].deps.is_empty());
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("build-draft.md")
    );
}

#[gpui::test]
async fn expanded_prompt_keyboard_submit_uses_the_reloaded_form(cx: &mut TestAppContext) {
    use crate::ui::WindowExt as _;
    let (_lane, ws, path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(ws.active, &path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws.read_with(&vcx, |ws, _| ws.open_graph().unwrap().2);
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test(&"design".into(), window, cx);
        v.expand_prompt(window, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("end enter");
    assert!(vcx.update(|window, cx| window.has_active_dialog(cx)));
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{text}\n# editor reload\n")).unwrap();
    view.update_in(&mut vcx, |v, window, cx| v.reload(window, cx));
    vcx.simulate_keystrokes("secondary-enter");
    vcx.run_until_parked();
    assert!(!vcx.update(|window, cx| window.has_active_dialog(cx)));
    assert!(view.read_with(&vcx, |v, cx| {
        v.form()
            .unwrap()
            .body_states(cx)
            .prompt
            .inline
            .read(cx)
            .value()
            .contains('\n')
    }));
}

#[gpui::test]
async fn refused_deletion_keeps_all_drafts(cx: &mut TestAppContext) {
    let (_lane, ws, path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(ws.active, &path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws.read_with(&vcx, |ws, _| ws.open_graph().unwrap().2);
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test(&"design".into(), window, cx);
        v.form()
            .unwrap()
            .body_states(cx)
            .output
            .clone()
            .update(cx, |input, cx| input.set_value("kept.md", window, cx));
    });
    let before = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{before}\n# concurrent writer\n")).unwrap();
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.delete_nodes(&path, view.clone(), vec!["design".into()], window, cx)
    });
    assert!(view.read_with(&vcx, |v, cx| v.has_unsaved_form(cx)));
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.form().unwrap().fields(cx).body.output),
        "kept.md"
    );
    assert_eq!(
        daruda_flow::parse::parse_flow_file(&std::fs::read_to_string(&path).unwrap())
            .unwrap()
            .nodes
            .len(),
        2
    );
}

#[gpui::test]
async fn expanded_prompt_keeps_text_when_apply_conflicts(cx: &mut TestAppContext) {
    use crate::ui::WindowExt as _;
    let (_lane, ws, path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(ws.active, &path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws.read_with(&vcx, |ws, _| ws.open_graph().unwrap().2);
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test(&"design".into(), window, cx);
        v.expand_prompt(window, cx);
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("end enter");
    view.update_in(&mut vcx, |v, window, cx| {
        v.form()
            .unwrap()
            .body_states(cx)
            .prompt
            .inline
            .clone()
            .update(cx, |input, cx| {
                input.set_value("concurrent edit", window, cx)
            });
    });
    vcx.simulate_keystrokes("secondary-enter");
    assert!(vcx.update(|window, cx| window.has_active_dialog(cx)));
    assert_eq!(
        view.read_with(&vcx, |v, cx| v
            .form()
            .unwrap()
            .body_states(cx)
            .prompt
            .inline
            .read(cx)
            .value()
            .to_string()),
        "concurrent edit"
    );
    view.update_in(&mut vcx, |v, window, cx| {
        v.form()
            .unwrap()
            .body_states(cx)
            .prompt
            .inline
            .clone()
            .update(cx, |input, cx| input.set_value("write a line", window, cx));
    });
    vcx.simulate_keystrokes("secondary-enter");
    assert!(!vcx.update(|window, cx| window.has_active_dialog(cx)));
    assert!(view.read_with(&vcx, |v, cx| {
        v.form()
            .unwrap()
            .body_states(cx)
            .prompt
            .inline
            .read(cx)
            .value()
            .contains('\n')
    }));
}

#[gpui::test]
async fn expanded_prompt_buttons_are_keyboard_reachable(cx: &mut TestAppContext) {
    use crate::ui::WindowExt as _;
    let (_lane, ws, path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(ws.active, &path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws.read_with(&vcx, |ws, _| ws.open_graph().unwrap().2);
    for (keys, saved) in [("tab enter", false), ("tab tab enter", true)] {
        view.update_in(&mut vcx, |v, window, cx| {
            v.select_node_for_test(&"design".into(), window, cx);
            v.expand_prompt(window, cx);
        });
        vcx.run_until_parked();
        vcx.simulate_keystrokes("end enter");
        vcx.simulate_keystrokes(keys);
        // GPUI buttons activate on key release; simulate_keystrokes sends only down.
        vcx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
        });
        vcx.run_until_parked();
        assert!(!vcx.update(|window, cx| window.has_active_dialog(cx)));
        assert_eq!(
            view.read_with(&vcx, |v, cx| v
                .form()
                .unwrap()
                .body_states(cx)
                .prompt
                .inline
                .read(cx)
                .value()
                .contains('\n')),
            saved
        );
    }
}
