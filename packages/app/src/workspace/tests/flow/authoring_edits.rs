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

pub(super) fn set_output(
    v: &mut crate::workspace::pages::flows::graph::FlowGraphView,
    node: &str,
    value: &str,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<crate::workspace::pages::flows::graph::FlowGraphView>,
) {
    v.select_node_for_test(&node.into(), window, cx);
    v.form()
        .unwrap()
        .body_states(cx)
        .output
        .clone()
        .update(cx, |input, cx| {
            input.set_value(value.to_string(), window, cx)
        });
}

#[gpui::test]
async fn node_revert_keeps_other_drafts_and_the_title(cx: &mut TestAppContext) {
    let (_lane, ws, path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(ws.active, &path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws.read_with(&vcx, |ws, _| ws.open_graph().unwrap().2);
    view.update_in(&mut vcx, |v, window, cx| {
        set_output(v, "design", "design-draft.md", window, cx);
        set_output(v, "build", "build-draft.md", window, cx);
        v.name_input_for_test()
            .update(cx, |input, cx| input.set_value("Renamed", window, cx));
        v.revert_selected_node(window, cx);
    });
    view.read_with(&vcx, |v, cx| {
        assert_eq!(v.form().unwrap().fields(cx).body.output, "build.md");
        assert_eq!(v.edited_name(cx), "Renamed");
    });
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test(&"design".into(), window, cx)
    });
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.form().unwrap().fields(cx).body.output),
        "design-draft.md"
    );
}

#[gpui::test]
async fn disconnecting_a_drafted_node_rebases_its_dependencies(cx: &mut TestAppContext) {
    let (_lane, ws, path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(ws.active, &path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws.read_with(&vcx, |ws, _| ws.open_graph().unwrap().2);
    view.update_in(&mut vcx, |v, window, cx| {
        set_output(v, "build", "build-draft.md", window, cx)
    });
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.disconnect_nodes(
            &path,
            view.clone(),
            &"design".into(),
            &"build".into(),
            window,
            cx,
        )
    });
    view.read_with(&vcx, |v, cx| {
        assert!(!v.draft_conflict());
        assert!(v.save_error_for_test().is_none());
        let fields = v.form().unwrap().fields(cx);
        assert!(fields.deps.is_empty());
        assert_eq!(fields.body.output, "build-draft.md");
    });
    assert!(ws.update_in(&mut vcx, |ws, window, cx| ws.save_flow_editor(
        &path,
        view.clone(),
        window,
        cx
    )));
    let file =
        daruda_flow::parse::parse_flow_file(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let build = file
        .nodes
        .iter()
        .find(|n| n.id.as_str() == "build")
        .unwrap();
    assert!(build.deps.is_empty());
}

#[gpui::test]
async fn restored_drafts_show_their_suggested_values(cx: &mut TestAppContext) {
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
            .timeout_state()
            .clone()
            .update(cx, |input, cx| input.set_value("30m", window, cx));
        v.select_node_for_test(&"build".into(), window, cx);
        v.select_node_for_test(&"design".into(), window, cx);
    });
    let picker = view.read_with(&vcx, |v, cx| v.form().unwrap().timeout_picker(cx));
    assert_eq!(
        picker
            .read_with(&vcx, |state, _| state.selected_value().cloned())
            .as_deref(),
        Some("30m")
    );
}

#[gpui::test]
async fn resolving_a_conflict_retracts_its_banner(cx: &mut TestAppContext) {
    let (_lane, ws, path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(ws.active, &path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws.read_with(&vcx, |ws, _| ws.open_graph().unwrap().2);
    view.update_in(&mut vcx, |v, window, cx| {
        set_output(v, "design", "mine.md", window, cx)
    });
    let original = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, original.replace("write a line", "theirs")).unwrap();
    view.update_in(&mut vcx, |v, window, cx| v.reload(window, cx));
    assert!(view.read_with(&vcx, |v, _| v.save_error_for_test().is_some()));
    std::fs::write(&path, &original).unwrap();
    view.update_in(&mut vcx, |v, window, cx| v.reload(window, cx));
    view.read_with(&vcx, |v, _| {
        assert!(!v.draft_conflict());
        assert!(v.save_error_for_test().is_none());
    });
}

#[gpui::test]
async fn a_conflicted_node_stays_revertable_after_typing_back(cx: &mut TestAppContext) {
    let (_lane, ws, path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(ws.active, &path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws.read_with(&vcx, |ws, _| ws.open_graph().unwrap().2);
    view.update_in(&mut vcx, |v, window, cx| {
        set_output(v, "design", "mine.md", window, cx)
    });
    let original = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, original.replace("write a line", "theirs")).unwrap();
    view.update_in(&mut vcx, |v, window, cx| {
        v.reload(window, cx);
        set_output(v, "design", "design.md", window, cx);
    });
    view.read_with(&vcx, |v, cx| {
        let status = v.form_status(cx);
        assert!(status.conflict && status.node_revertable);
    });
    view.update_in(&mut vcx, |v, window, cx| v.revert_selected_node(window, cx));
    view.read_with(&vcx, |v, cx| {
        assert!(!v.draft_conflict());
        assert!(!v.form_status(cx).node_revertable);
    });
}

#[gpui::test]
async fn an_unrelated_reload_keeps_the_live_form(cx: &mut TestAppContext) {
    let (_lane, ws, path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(ws.active, &path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws.read_with(&vcx, |ws, _| ws.open_graph().unwrap().2);
    view.update_in(&mut vcx, |v, window, cx| {
        set_output(v, "design", "mine.md", window, cx)
    });
    let before = view.read_with(&vcx, |v, cx| {
        v.form().unwrap().body_states(cx).output.entity_id()
    });
    let original = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, original.replace("build it", "build it again")).unwrap();
    view.update_in(&mut vcx, |v, window, cx| v.reload(window, cx));
    view.read_with(&vcx, |v, cx| {
        let form = v.form().unwrap();
        assert_eq!(form.body_states(cx).output.entity_id(), before);
        assert_eq!(form.fields(cx).body.output, "mine.md");
        assert!(!v.draft_conflict());
    });
}

#[gpui::test]
async fn a_refused_save_selects_the_drafted_node_it_names(cx: &mut TestAppContext) {
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
            .deps_state()
            .clone()
            .update(cx, |input, cx| input.set_value("ghost", window, cx));
        set_output(v, "build", "build-draft.md", window, cx);
    });
    assert!(
        !ws.update_in(&mut vcx, |ws, window, cx| ws.save_flow_editor(
            &path,
            view.clone(),
            window,
            cx
        ))
    );
    view.read_with(&vcx, |v, _| {
        let form = v.form().unwrap();
        assert_eq!(form.node, daruda_flow::NodeId::from("design"));
        assert!(
            form.note_for_test(
                crate::workspace::pages::flows::graph::inspector::notes::FormField::Deps
            )
            .is_some()
        );
    });
}
