//! Authoring choices and draft persistence through real workspace operations.
use super::*;
use daruda_flow::NodeId;

#[gpui::test]
async fn drafts_survive_switching_and_save_together(cx: &mut TestAppContext) {
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
            .update(cx, |input, cx| input.set_value("draft.md", window, cx));
        v.select_node_for_test(&"build".into(), window, cx);
        v.form()
            .unwrap()
            .body_states(cx)
            .output
            .clone()
            .update(cx, |input, cx| input.set_value("result.md", window, cx));
        v.select_node_for_test(&"design".into(), window, cx);
    });
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.form().unwrap().fields(cx).body.output),
        "draft.md"
    );
    assert!(ws.update_in(&mut vcx, |ws, window, cx| ws.save_flow_editor(
        &path,
        view.clone(),
        window,
        cx
    )));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("draft.md") && text.contains("result.md"));
    assert!(!view.read_with(&vcx, |v, cx| v.has_unsaved_form(cx)));
}

#[gpui::test]
async fn invalid_hidden_drafts_cannot_be_silently_saved(cx: &mut TestAppContext) {
    let (_lane, ws, path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let before = std::fs::read_to_string(&path).unwrap();
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
            .update(cx, |input, cx| {
                input.set_value("not a duration", window, cx)
            });
        v.select_node_for_test(&"build".into(), window, cx);
    });
    assert!(
        !ws.update_in(&mut vcx, |ws, window, cx| ws.save_flow_editor(
            &path,
            view.clone(),
            window,
            cx
        ))
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), before);
    assert_eq!(
        view.read_with(&vcx, |v, _| v.form().unwrap().node.clone()),
        NodeId::from("design")
    );
}

#[gpui::test]
async fn pickers_preserve_unknown_values_and_restore_inheritance(cx: &mut TestAppContext) {
    use crate::ui::select::SelectEvent;
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
            .agent_states()
            .model
            .clone()
            .update(cx, |input, cx| input.set_value("retired-model", window, cx));
    });
    vcx.run_until_parked();
    let picker = view.read_with(&vcx, |v, cx| v.form().unwrap().agent_picker(1, cx));
    assert_eq!(
        picker
            .read_with(&vcx, |state, _| state.selected_value().cloned())
            .as_deref(),
        Some("retired-model")
    );
    picker.update_in(&mut vcx, |state, window, cx| {
        state.set_selected_value(&"".into(), window, cx);
        cx.emit(SelectEvent::Confirm(Some("".into())));
    });
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.form().unwrap().fields(cx).agent.model),
        None
    );
    assert!(!view.read_with(&vcx, |v, cx| v.has_unsaved_form(cx)));
}

#[gpui::test]
async fn selecting_a_result_inserts_a_reference_without_changing_dependencies(
    cx: &mut TestAppContext,
) {
    use crate::ui::select::SelectEvent;
    let (_lane, ws, path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(ws.active, &path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws.read_with(&vcx, |ws, _| ws.open_graph().unwrap().2);
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test(&"build".into(), window, cx);
        v.choose_form_link(
            true,
            &SelectEvent::Confirm(Some("design".into())),
            window,
            cx,
        );
    });
    let fields = view.read_with(&vcx, |v, cx| v.form().unwrap().fields(cx));
    assert_eq!(fields.deps, ["design"]);
    let crate::workspace::pages::flows::graph::form::SourceField::Inline(prompt) =
        fields.body.prompt
    else {
        panic!("inline prompt");
    };
    // The prompt was never focused, so the reference follows its text rather
    // than landing at the caret `set_value` left at the start.
    assert_eq!(prompt, "build it{{node.design.output}}");
}

#[cfg(feature = "screenshot")]
#[gpui::test]
async fn expanded_prompt_opens_and_dismisses(cx: &mut TestAppContext) {
    use crate::ui::WindowExt as _;
    use crate::workspace::pages::flows::graph::form::screenshot::FlowFormShot;
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_form_for_shot(FlowFormShot::Expanded, window, cx)
    });
    vcx.run_until_parked();
    assert!(vcx.update(|window, cx| window.has_active_dialog(cx)));
    vcx.update(|window, cx| window.close_dialog(cx));
    assert!(!vcx.update(|window, cx| window.has_active_dialog(cx)));
}

#[gpui::test]
async fn inactive_prompt_sources_survive_selection_and_reload(cx: &mut TestAppContext) {
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
            .prompt
            .file
            .clone()
            .update(cx, |input, cx| {
                input.set_value("prompts/draft.md", window, cx)
            });
        v.select_node_for_test(&"build".into(), window, cx);
    });
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{text}\n# external comment\n")).unwrap();
    view.update_in(&mut vcx, |v, window, cx| {
        v.reload(window, cx);
        v.select_node_for_test(&"design".into(), window, cx);
    });
    assert_eq!(
        view.read_with(&vcx, |v, cx| v
            .form()
            .unwrap()
            .body_states(cx)
            .prompt
            .file
            .read(cx)
            .value()
            .to_string()),
        "prompts/draft.md"
    );
    assert!(view.read_with(&vcx, |v, cx| v.has_unsaved_form(cx)));
}
