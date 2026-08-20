//! The node inspector: reading a node into boxes and writing it back.
//!
//! Every one of these needs a window, because a box is an entity. What a
//! field's contents *mean* is `form/fields.rs`'s, and tested without one.

use super::*;

/// A change written through the typed shape, and the two gates that stop one.
///
/// Each refusal asserts the *file*, not the toast: the whole point of a gate is
/// that a flow on disk is never left in a state nobody wrote.
#[gpui::test]
async fn an_edit_passes_both_gates_or_the_file_is_untouched(cx: &mut TestAppContext) {
    use crate::workspace::flow_file_ops::EditRefusal;

    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");
    let base = view
        .read_with(&vcx, |v, _| v.text().map(str::to_string))
        .expect("the pane holds what it read");

    // Gate 2 — the result would not load. `deps` naming a node that is not
    // there is a validation failure, so the file must not change.
    let base_for_bad = base.clone();
    let refused = ws.update_in(&mut vcx, |ws, window, cx| {
        ws.edit_flow(
            &flow_path,
            &base_for_bad,
            |file| file.nodes[0].deps = vec!["nobody".into()],
            window,
            cx,
        )
    });
    assert!(
        matches!(&refused, Err(EditRefusal::WouldNotLoad { detail, .. }) if detail.contains("nobody")),
        "refused in the engine's own words: {refused:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&flow_path).unwrap(),
        base,
        "and the file is byte-identical"
    );

    // Gate 1 — somebody else changed the file after we read it.
    let outside = format!("{base}# somebody else was here\n");
    std::fs::write(&flow_path, &outside).expect("write");
    let base_for_stale = base.clone();
    let refused = ws.update_in(&mut vcx, |ws, window, cx| {
        ws.edit_flow(
            &flow_path,
            &base_for_stale,
            |file| file.nodes[0].timeout = Some(std::time::Duration::from_secs(5)),
            window,
            cx,
        )
    });
    assert_eq!(
        refused,
        Err(EditRefusal::Stale),
        "an edit against text that has moved on is refused"
    );
    assert_eq!(
        std::fs::read_to_string(&flow_path).unwrap(),
        outside,
        "and the other change is left alone"
    );

    // And the same edit against the text that is actually there goes through,
    // keeping the line somebody else added.
    let wrote = ws.update_in(&mut vcx, |ws, window, cx| {
        ws.edit_flow(
            &flow_path,
            &outside,
            |file| file.nodes[0].timeout = Some(std::time::Duration::from_secs(5)),
            window,
            cx,
        )
    });
    assert_eq!(
        wrote,
        Ok(()),
        "the same edit against the current text is written"
    );
    let after = std::fs::read_to_string(&flow_path).unwrap();
    assert!(after.contains("    timeout: 5s\n"), "{after}");
    assert!(after.contains("# somebody else was here"), "{after}");
    daruda_flow::load(&after, None).expect("and what was written loads");

    // The pane followed its own write without waiting for a watcher event.
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(&vcx, |v, _| v.text().map(str::to_string)),
        Some(after),
        "the pane is drawing what is on disk"
    );
}

/// Clicking a card selects it, and nothing else happens.
///
/// The vendored plugin that normally does this starts a drag, which is why
/// daruda has its own — so the test asserts both halves: the selection lands,
/// and the node has not moved. Positions are never persisted, so a node that
/// moved would be a lie until the next reload.
#[gpui::test]
async fn clicking_a_card_selects_it_and_does_not_move_it(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::Selection;

    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");
    let canvas = view
        .read_with(&vcx, |v, _| v.canvas_for_test().cloned())
        .expect("it drew");

    // Where the first card sits on screen, asked of the canvas rather than
    // guessed: the graph is auto-placed and then framed into the pane.
    let (node, centre, before) = canvas
        .read_with(&vcx, |canvas, _| {
            let graph = canvas.graph();
            let node = graph.paint_order().first().copied()?;
            let bounds = graph.node_world_bounds(node)?;
            let origin = canvas.viewport().window_bounds()?.origin;
            let local = canvas.viewport().world_to_screen(bounds.center());
            Some((
                node,
                gpui::point(origin.x + local.x, origin.y + local.y),
                bounds.origin,
            ))
        })
        .expect("the card has a place on screen");

    vcx.simulate_click(centre, gpui::Modifiers::none());
    vcx.run_until_parked();

    assert!(
        canvas.read_with(&vcx, |c, _| c.graph().selected_node().contains(&node)),
        "the card under the pointer is selected"
    );
    assert_eq!(
        canvas.read_with(&vcx, |c, _| c
            .graph()
            .node_world_bounds(node)
            .map(|b| b.origin)),
        Some(before),
        "and it has not moved — this is a selection, not a drag"
    );
    assert!(
        matches!(view.read_with(&vcx, |v, cx| v.selection(cx)), Selection::One(id) if id == "design"),
        "the pane reports it in the flow's own terms"
    );

    // A click on empty space clears it. The pane is wider than the graph, so
    // the far corner of the canvas is background.
    let corner = canvas.read_with(&vcx, |c, _| {
        let b = c.viewport().window_bounds().expect("measured");
        gpui::point(
            b.origin.x + b.size.width - gpui::px(4.0),
            b.origin.y + gpui::px(4.0),
        )
    });
    vcx.simulate_click(corner, gpui::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.selection(cx)),
        Selection::None,
        "clicking the background clears the selection"
    );

    // Shift adds to the selection rather than replacing it — and the pane says
    // "several", because it cannot show one node's fields and stay honest.
    let second = canvas
        .read_with(&vcx, |canvas, _| {
            let graph = canvas.graph();
            let node = graph.paint_order().get(1).copied()?;
            let bounds = graph.node_world_bounds(node)?;
            let origin = canvas.viewport().window_bounds()?.origin;
            let local = canvas.viewport().world_to_screen(bounds.center());
            Some(gpui::point(origin.x + local.x, origin.y + local.y))
        })
        .expect("the second card has a place too");
    vcx.simulate_click(centre, gpui::Modifiers::none());
    vcx.simulate_click(second, gpui::Modifiers::shift());
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.selection(cx)),
        Selection::Many(vec!["build".to_string(), "design".to_string()]),
        "shift-click adds the second card, and says which two"
    );
}

/// The inspector shows what the file says, and Save writes one field back.
///
/// Driven through the event the buttons emit rather than by calling the op, so
/// the wiring the pane installs is part of what is under test.
#[gpui::test]
async fn the_inspector_saves_one_field_and_keeps_its_place(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::FlowGraphEvent;

    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");
    let before = std::fs::read_to_string(&flow_path).expect("readable");

    // Selecting the first card fills the form from the file.
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("design", window, cx)
    });
    vcx.run_until_parked();
    let output = view
        .read_with(&vcx, |v, cx| {
            let form = v.form().expect("a form for the selected node");
            Some(form.body_states(cx).output.clone())
        })
        .expect("an agent node has an output");
    assert_eq!(
        output.read_with(&vcx, |state, _| state.value().to_string()),
        "design.md",
        "the box holds what the file says"
    );

    // Type a new one and save.
    output.update_in(&mut vcx, |state, window, cx| {
        state.set_value("spec.md".to_string(), window, cx)
    });
    assert!(
        view.read_with(&vcx, |v, cx| v.form().expect("form").is_dirty(cx)),
        "an edited box is dirty"
    );
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();

    let after = std::fs::read_to_string(&flow_path).expect("readable");
    assert_eq!(
        after,
        before.replace("output: design.md", "output: spec.md"),
        "one field, one line"
    );
    // And the pane is still on that node, with a clean form read back from the
    // file — the save does not close the inspector it was typed into.
    assert!(
        view.read_with(&vcx, |v, cx| {
            v.form()
                .is_some_and(|form| form.node == "design" && !form.is_dirty(cx))
        }),
        "the inspector stayed put and reads the file again"
    );
}

/// Switching cards switches the form, and does not carry the old node's text
/// across.
#[gpui::test]
async fn selecting_another_node_rebuilds_the_form(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");

    let read_output = |vcx: &mut gpui::VisualTestContext| {
        view.read_with(vcx, |v, cx| {
            let form = v.form()?;
            Some(form.body_states(cx).output.read(cx).value().to_string())
        })
    };

    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("design", window, cx)
    });
    vcx.run_until_parked();
    assert_eq!(read_output(&mut vcx).as_deref(), Some("design.md"));

    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("build", window, cx)
    });
    vcx.run_until_parked();
    assert_eq!(
        read_output(&mut vcx).as_deref(),
        Some("build.md"),
        "the second node's own values, not the first's"
    );
}

/// Renaming a node moves every mention of it.
///
/// `deps: [design]` is flow style, so this is also where the two halves meet:
/// the rename walks into a value that cannot be spliced, and the editor replaces
/// that whole value in the style it was written in.
#[gpui::test]
async fn renaming_a_node_takes_the_mentions_of_it_along(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::FlowGraphEvent;

    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");

    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("design", window, cx)
    });
    vcx.run_until_parked();
    let id = view
        .read_with(&vcx, |v, _| v.form().map(|form| form.id_state().clone()))
        .expect("a form for the selected node");
    id.update_in(&mut vcx, |state, window, cx| {
        state.set_value("spec".to_string(), window, cx)
    });
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();

    let after = std::fs::read_to_string(&flow_path).expect("readable");
    assert!(
        after.contains("  - id: spec\n"),
        "the node is renamed:\n{after}"
    );
    assert!(
        after.contains("    deps: [spec]\n"),
        "and the node that depended on it says so, still on one line:\n{after}"
    );
    daruda_flow::load(&after, None).expect("and the flow still loads");
}

/// The safety net under the rename, pinned: a rename that forgets a mention is
/// refused by the second gate, and the file does not change.
#[gpui::test]
async fn a_rename_that_forgets_a_mention_is_refused(cx: &mut TestAppContext) {
    use crate::workspace::flow_file_ops::EditRefusal;

    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let before = std::fs::read_to_string(&flow_path).expect("readable");

    let refused = ws.update_in(&mut vcx, |ws, window, cx| {
        ws.edit_flow(
            &flow_path,
            &before,
            // Only the id, deliberately leaving `deps: [design]` behind.
            |file| file.nodes[0].id = "spec".into(),
            window,
            cx,
        )
    });
    assert!(
        matches!(refused, Err(EditRefusal::WouldNotLoad { .. })),
        "an unknown dependency is not written: {refused:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&flow_path).unwrap(),
        before,
        "and the file is byte-identical"
    );
}

/// A refused save says why *beside the fields*, in the words of whoever refused
/// it — the app does not re-write the engine's reason, it only chooses where to
/// put it.
#[gpui::test]
async fn a_refused_save_says_why_beside_the_fields(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::FlowGraphEvent;

    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");
    let before = std::fs::read_to_string(&flow_path).expect("readable");

    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("design", window, cx)
    });
    vcx.run_until_parked();
    let (deps, output) = view
        .read_with(&vcx, |v, cx| {
            let form = v.form()?;
            Some((
                form.deps_state().clone(),
                form.body_states(cx).output.clone(),
            ))
        })
        .expect("an agent node's form");

    // A dependency on a node that is not there — the engine's `UnknownDep`.
    deps.update_in(&mut vcx, |state, window, cx| {
        state.set_value("nobody".to_string(), window, cx)
    });
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();

    let banner = view
        .read_with(&vcx, |v, _| {
            v.form().and_then(|f| f.banner().map(str::to_string))
        })
        .expect("the form says why");
    assert!(
        banner.contains("nobody"),
        "the engine's own words reached the inspector: {banner}"
    );
    assert_eq!(
        std::fs::read_to_string(&flow_path).unwrap(),
        before,
        "and the file is untouched"
    );

    // Put it back and change something real: the save goes through and the
    // banner goes with it.
    deps.update_in(&mut vcx, |state, window, cx| {
        state.set_value(String::new(), window, cx)
    });
    output.update_in(&mut vcx, |state, window, cx| {
        state.set_value("spec.md".to_string(), window, cx)
    });
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(&vcx, |v, _| v
            .form()
            .and_then(|f| f.banner().map(str::to_string))),
        None,
        "a save that lands leaves nothing to explain"
    );
    assert!(
        std::fs::read_to_string(&flow_path)
            .unwrap()
            .contains("output: spec.md"),
        "and it landed"
    );
}

/// The node's own agent, and where it runs, are the file's words — read into the
/// boxes and written back one line at a time.
#[gpui::test]
async fn the_agent_override_is_read_and_written_one_field_at_a_time(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::FlowGraphEvent;

    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, OVERRIDDEN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");
    let before = std::fs::read_to_string(&flow_path).expect("readable");

    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("design", window, cx)
    });
    vcx.run_until_parked();

    // The block is open because the node already overrides something, and the
    // boxes hold what the file says.
    let (open, agent_id, model, cwd) = view.read_with(&vcx, |v, cx| {
        let form = v.form().expect("a form");
        let states = form.agent_states();
        (
            form.agent_open(),
            states.id.read(cx).value().to_string(),
            states.model.clone(),
            form.cwd_state().clone(),
        )
    });
    assert!(open, "a node that overrides something opens the block");
    assert_eq!(agent_id, "codex");

    model.update_in(&mut vcx, |state, window, cx| {
        state.set_value("opus".to_string(), window, cx)
    });
    cwd.update_in(&mut vcx, |state, window, cx| {
        state.set_value("sub".to_string(), window, cx)
    });
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();

    let after = std::fs::read_to_string(&flow_path).unwrap();
    assert_eq!(
        after.replace("    cwd: sub\n", ""),
        before.replace("model: gpt", "model: opus"),
        "the model line was replaced and nothing else moved"
    );
    assert!(
        after.contains("    cwd: sub\n"),
        "and `cwd` was added:\n{after}"
    );
    daruda_flow::load(&after, None).expect("still loads");
}

/// Emptying every axis takes the whole `agent:` block out — an empty key would
/// be a node overriding nothing while saying it overrides.
#[gpui::test]
async fn emptying_every_axis_removes_the_agent_block(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::FlowGraphEvent;

    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, OVERRIDDEN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");

    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("design", window, cx)
    });
    vcx.run_until_parked();
    let states = view.read_with(&vcx, |v, _| {
        let s = v.form().expect("a form").agent_states();
        (s.id.clone(), s.mode.clone(), s.model.clone())
    });
    for state in [states.0, states.1, states.2] {
        state.update_in(&mut vcx, |state, window, cx| {
            state.set_value(String::new(), window, cx)
        });
    }
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();

    let after = std::fs::read_to_string(&flow_path).unwrap();
    assert!(
        !after.contains("      id: codex") && !after.contains("    agent:"),
        "the block is gone:\n{after}"
    );
    assert!(
        after.contains("  agent:\n    id: claude"),
        "and `defaults.agent` is not what went away:\n{after}"
    );
    daruda_flow::load(&after, None).expect("still loads");
}

/// The engine's pairing rule reaches the person: naming an agent without a mode
/// is refused, in the engine's words, beside the fields.
#[gpui::test]
async fn naming_an_agent_without_a_mode_is_refused_in_the_engines_words(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::FlowGraphEvent;

    let plain = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write a line
";
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, plain);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");
    let before = std::fs::read_to_string(&flow_path).expect("readable");

    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("design", window, cx)
    });
    vcx.run_until_parked();
    let agent_id = view.read_with(&vcx, |v, _| {
        v.form().expect("a form").agent_states().id.clone()
    });
    agent_id.update_in(&mut vcx, |state, window, cx| {
        state.set_value("codex".to_string(), window, cx)
    });
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();

    let banner = view
        .read_with(&vcx, |v, _| {
            v.form().and_then(|f| f.banner().map(str::to_string))
        })
        .expect("the form says why");
    assert_eq!(
        banner,
        crate::surface::strings::flow_edit_would_not_load(
            &crate::surface::strings::flow_issue_line(
                Some("design"),
                &crate::surface::strings::flow_issue(
                    &daruda_flow::error::ValidationKind::AgentIdWithoutMode
                )
            )
        ),
        "the engine's rule, in the app's one wording site"
    );
    assert_eq!(
        std::fs::read_to_string(&flow_path).unwrap(),
        before,
        "and the file is untouched"
    );
}

/// Turning `halt` into a retry writes the block the file did not have, and
/// turning it back takes the block out again.
#[gpui::test]
async fn a_fail_policy_grows_a_block_and_gives_it_back(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::FlowGraphEvent;

    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");
    let before = std::fs::read_to_string(&flow_path).expect("readable");

    let fail_states = |vcx: &mut gpui::VisualTestContext| {
        view.read_with(vcx, |v, cx| {
            let form = v.form().expect("a form");
            let on_fail = form.body_states(cx).on_fail;
            Some((
                on_fail.policy.clone(),
                on_fail.hint.inline.clone(),
                on_fail.max_attempts.clone(),
            ))
        })
        .expect("an agent node")
    };

    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("design", window, cx)
    });
    vcx.run_until_parked();
    let (policy, hint, attempts) = fail_states(&mut vcx);

    // `halt` → try again, twice, with a hint.
    policy.update_in(&mut vcx, |state, window, cx| {
        state.set_selected_value(&gpui::SharedString::from("act"), window, cx)
    });
    hint.update_in(&mut vcx, |state, window, cx| {
        state.set_value("read the error first".to_string(), window, cx)
    });
    attempts.update_in(&mut vcx, |state, window, cx| {
        state.set_value("2".to_string(), window, cx)
    });
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();

    let after = std::fs::read_to_string(&flow_path).unwrap();
    assert!(
        after.contains(
            "    on_fail:\n      retry:\n        hint: read the error first\n        max_attempts: 2\n"
        ),
        "the block the file did not have:\n{after}"
    );
    daruda_flow::load(&after, None).expect("and it loads");

    // And back: the block collapses to the one line that means the same thing.
    let (policy, _, _) = fail_states(&mut vcx);
    policy.update_in(&mut vcx, |state, window, cx| {
        state.set_selected_value(&gpui::SharedString::from("halt"), window, cx)
    });
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();

    let after = std::fs::read_to_string(&flow_path).unwrap();
    assert!(
        after.contains("    on_fail: halt\n") && !after.contains("retry:"),
        "the block is one line again:\n{after}"
    );
    assert_eq!(
        after.replace("    on_fail: halt\n", ""),
        before,
        "and nothing else in the file moved"
    );
}

/// A gate's repair must say what to feed the fixer — the engine's rule, and the
/// inspector is where the person reads that they did not.
#[gpui::test]
async fn a_repair_without_failure_context_is_refused(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::FlowGraphEvent;

    let with_gate = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: build
    kind: agent
    output: build.md
    prompt: build it
  - id: check
    kind: command
    deps: [build]
    run: cargo test
";
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, with_gate);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");
    let before = std::fs::read_to_string(&flow_path).expect("readable");

    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("check", window, cx)
    });
    vcx.run_until_parked();
    let (policy, fix, attempts) = view.read_with(&vcx, |v, cx| {
        let on_fail = v.form().expect("a form").body_states(cx).on_fail;
        (
            on_fail.policy.clone(),
            on_fail.fix.clone(),
            on_fail.max_attempts.clone(),
        )
    });

    policy.update_in(&mut vcx, |state, window, cx| {
        state.set_selected_value(&gpui::SharedString::from("act"), window, cx)
    });
    // No `{{failure}}` and no `{{attempts}}` — the engine refuses this.
    fix.update_in(&mut vcx, |state, window, cx| {
        state.set_value("just fix it".to_string(), window, cx)
    });
    attempts.update_in(&mut vcx, |state, window, cx| {
        state.set_value("2".to_string(), window, cx)
    });
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();

    let banner = view
        .read_with(&vcx, |v, _| {
            v.form().and_then(|f| f.banner().map(str::to_string))
        })
        .expect("the form says why");
    assert!(
        banner.contains(&crate::surface::strings::flow_issue(
            &daruda_flow::error::ValidationKind::RepairWithoutFailureContext
        )),
        "the engine's rule reached the inspector: {banner}"
    );
    assert_eq!(
        std::fs::read_to_string(&flow_path).unwrap(),
        before,
        "and the file is untouched"
    );
}

/// What the form blocks itself: text that cannot become the number or the
/// duration the file needs. The engine never sees these, so nobody else can
/// refuse them.
#[gpui::test]
async fn the_form_blocks_what_cannot_become_a_number_or_a_duration(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::form::Refusal;

    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("design", window, cx)
    });
    vcx.run_until_parked();

    let (policy, attempts, timeout) = view
        .read_with(&vcx, |v, cx| {
            let form = v.form().expect("a form");
            let on_fail = form.body_states(cx).on_fail;
            Some((
                on_fail.policy.clone(),
                on_fail.max_attempts.clone(),
                form.timeout_state().clone(),
            ))
        })
        .expect("an agent node");

    // A retry has to say how many times, and "twice" is not a count.
    policy.update_in(&mut vcx, |state, window, cx| {
        state.set_selected_value(&gpui::SharedString::from("act"), window, cx)
    });
    attempts.update_in(&mut vcx, |state, window, cx| {
        state.set_value("twice".to_string(), window, cx)
    });
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.form().expect("a form").refusal(cx)),
        Some(Refusal::Attempts("twice".to_string())),
        "the form refuses a count it cannot make"
    );

    attempts.update_in(&mut vcx, |state, window, cx| {
        state.set_value("2".to_string(), window, cx)
    });
    timeout.update_in(&mut vcx, |state, window, cx| {
        state.set_value("soon".to_string(), window, cx)
    });
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.form().expect("a form").refusal(cx)),
        Some(Refusal::Timeout("soon".to_string())),
        "and a duration it cannot make"
    );
}

/// Adding a node writes it, chains it after what was selected, and leaves the
/// inspector on it — the person's next move is to type a prompt.
#[gpui::test]
async fn adding_a_node_chains_it_and_selects_it(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");

    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("build", window, cx)
    });
    vcx.run_until_parked();
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.add_node(&flow_path, view.clone(), window, cx)
    });
    vcx.run_until_parked();

    let after = std::fs::read_to_string(&flow_path).unwrap();
    assert!(
        after.contains("  - id: node-1\n") && after.contains("    deps:\n      - build\n"),
        "the new node is chained after the selected one:\n{after}"
    );
    assert!(
        after.contains("    output: node-1.md\n"),
        "and it writes somewhere:\n{after}"
    );
    daruda_flow::load(&after, None).expect("a node that loads, not one to fix first");
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.selected_node(cx)),
        Some("node-1".to_string()),
        "and the inspector is already on it"
    );
}

/// Deleting a node takes it out of what pointed at it, and the last one is
/// refused — the file would be left with `nodes:` and nothing under it.
#[gpui::test]
async fn deleting_a_node_takes_the_dependencies_with_it(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");

    // `build` depends on `design`; deleting `design` has to take that with it.
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.delete_nodes(
            &flow_path,
            view.clone(),
            vec!["design".to_string()],
            window,
            cx,
        )
    });
    vcx.run_until_parked();
    let after = std::fs::read_to_string(&flow_path).unwrap();
    assert!(
        !after.contains("id: design") && !after.contains("deps:"),
        "the node and the dependency on it are both gone:\n{after}"
    );
    daruda_flow::load(&after, None).expect("and it loads");

    // The one node left cannot go: the file would stop being a flow file.
    let before = after;
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.delete_nodes(
            &flow_path,
            view.clone(),
            vec!["build".to_string()],
            window,
            cx,
        )
    });
    vcx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(&flow_path).unwrap(),
        before,
        "the last node is refused and the file is untouched"
    );
}

/// A prompt can move from prose to a file and back, and the file never names
/// both — which is the one thing the engine refuses about this pair.
#[gpui::test]
async fn a_prompt_can_come_from_a_file_instead(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::FlowGraphEvent;

    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    // Next to the flow, not next to the lane: `MissingPromptFile` resolves a
    // file-backed prompt against the directory the flow file is in.
    std::fs::write(
        flow_path.parent().expect("a flows dir").join("brief.md"),
        "the brief",
    )
    .expect("write");
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");

    let prompt_states = |vcx: &mut gpui::VisualTestContext| {
        view.read_with(vcx, |v, cx| {
            let prompt = v.form().expect("a form").body_states(cx).prompt;
            (
                prompt.choice.clone(),
                prompt.file.clone(),
                prompt.inline.clone(),
            )
        })
    };

    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("design", window, cx)
    });
    vcx.run_until_parked();
    let (choice, file, inline) = prompt_states(&mut vcx);
    assert_eq!(
        inline.read_with(&vcx, |s, _| s.value().to_string()),
        "write a line",
        "the prose box holds what the file says"
    );

    choice.update_in(&mut vcx, |state, window, cx| {
        state.set_selected_value(&gpui::SharedString::from("file"), window, cx)
    });
    file.update_in(&mut vcx, |state, window, cx| {
        state.set_value("brief.md".to_string(), window, cx)
    });
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();

    let after = std::fs::read_to_string(&flow_path).unwrap();
    assert!(
        after.contains("    prompt_file: brief.md\n")
            && !after.contains("    prompt: write a line"),
        "the prompt moved into the file reference, and did not stay as well:\n{after}\nbanner: {:?}",
        view.read_with(&vcx, |v, _| v
            .form()
            .and_then(|f| f.banner().map(str::to_string)))
    );
    daruda_flow::load(&after, None).expect("and it loads — naming both would not");

    // And back: the prose that was typed is still in its box behind the select.
    let (choice, _, inline) = prompt_states(&mut vcx);
    assert_eq!(
        inline.read_with(&vcx, |s, _| s.value().to_string()),
        "",
        "a file-backed node starts with an empty prose box"
    );
    inline.update_in(&mut vcx, |state, window, cx| {
        state.set_value("write it here after all".to_string(), window, cx)
    });
    choice.update_in(&mut vcx, |state, window, cx| {
        state.set_selected_value(&gpui::SharedString::from("inline"), window, cx)
    });
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();

    let after = std::fs::read_to_string(&flow_path).unwrap();
    assert!(
        after.contains("    prompt: write it here after all\n") && !after.contains("prompt_file"),
        "and back again, with the reference gone:\n{after}"
    );
    daruda_flow::load(&after, None).expect("still loads");
}

/// A node can become the other kind. The body is replaced wholesale — an agent's
/// prompt and output for a command's line — and the boxes for the kind that is
/// no longer chosen keep what was typed in them, so switching back loses nothing.
#[gpui::test]
async fn a_node_can_become_a_command_and_back(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::{
        FlowGraphEvent,
        form::{KindChoice, Refusal},
    };

    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");

    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("build", window, cx)
    });
    vcx.run_until_parked();
    let (kind, run) = view.read_with(&vcx, |v, cx| {
        let body = v.form().expect("a form").body_states(cx);
        (body.kind_select.clone(), body.run.clone())
    });

    kind.update_in(&mut vcx, |state, window, cx| {
        state.set_selected_value(&gpui::SharedString::from("command"), window, cx)
    });
    // A command with nothing to run is not a node, and the engine does not check
    // it — the form does.
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.form().expect("a form").refusal(cx)),
        Some(Refusal::RunRequired),
        "the form refuses a command that runs nothing"
    );
    run.update_in(&mut vcx, |state, window, cx| {
        state.set_value("cargo test".to_string(), window, cx)
    });
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();

    let after = std::fs::read_to_string(&flow_path).unwrap();
    assert!(
        after.contains("    kind: command\n") && after.contains("    run: cargo test\n"),
        "the node is a command now:\n{after}"
    );
    assert!(
        !after.contains("    output: build.md") && !after.contains("    prompt: build it"),
        "and the agent's body went with the kind:\n{after}"
    );
    daruda_flow::load(&after, None).expect("and it loads");

    // Back again: the agent boxes were rebuilt from a file that no longer has
    // them, so they are empty and the form says what it needs.
    let (kind, output) = view.read_with(&vcx, |v, cx| {
        let body = v.form().expect("a form").body_states(cx);
        (body.kind_select.clone(), body.output.clone())
    });
    kind.update_in(&mut vcx, |state, window, cx| {
        state.set_selected_value(&gpui::SharedString::from("agent"), window, cx)
    });
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.form().expect("a form").refusal(cx)),
        Some(Refusal::OutputRequired),
        "an agent that writes nowhere is refused too"
    );
    output.update_in(&mut vcx, |state, window, cx| {
        state.set_value("build.md".to_string(), window, cx)
    });
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();

    let after = std::fs::read_to_string(&flow_path).unwrap();
    assert!(
        after.contains("    kind: agent\n") && after.contains("    output: build.md\n"),
        "and an agent again:\n{after}"
    );
    daruda_flow::load(&after, None).expect("still loads");
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.form().expect("a form").body_states(cx).kind),
        KindChoice::Agent
    );
}

/// A refusal lands on the box it is about, not only in the banner.
#[gpui::test]
async fn a_refusal_points_at_the_field_it_is_about(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::{FlowGraphEvent, form::notes::FormField};

    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");

    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("design", window, cx)
    });
    vcx.run_until_parked();
    let deps = view.read_with(&vcx, |v, _| v.form().expect("a form").deps_state().clone());
    deps.update_in(&mut vcx, |state, window, cx| {
        state.set_value("nobody".to_string(), window, cx)
    });
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();

    let (deps_note, output_note) = view.read_with(&vcx, |v, _| {
        let form = v.form().expect("a form");
        (
            form.note_for_test(FormField::Deps).map(str::to_string),
            form.note_for_test(FormField::Output).map(str::to_string),
        )
    });
    assert_eq!(
        deps_note.as_deref(),
        Some(
            crate::surface::strings::flow_issue(&daruda_flow::error::ValidationKind::UnknownDep {
                dep: "nobody".into()
            })
            .as_str()
        ),
        "the dependency box says what the engine said"
    );
    assert_eq!(output_note, None, "and no other box claims it");
    assert!(
        view.read_with(&vcx, |v, _| v
            .form()
            .and_then(|f| f.banner().map(str::to_string)))
            .is_some(),
        "the banner still carries the whole sentence"
    );

    // A save that lands clears both.
    deps.update_in(&mut vcx, |state, window, cx| {
        state.set_value(String::new(), window, cx)
    });
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(&vcx, |v, _| {
            v.form()
                .and_then(|f| f.note_for_test(FormField::Deps).map(str::to_string))
        }),
        None,
        "nothing left to point at"
    );
}

/// A reload replaces what the inspector shows. When that throws away something
/// the person typed and had not saved, it says so — and only then: the pane
/// that pressed Save is mid-edit too when its own write comes back, and a
/// banner on every successful save would be noise.
#[gpui::test]
async fn typing_lost_to_a_reload_is_reported(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("design", window, cx)
    });
    vcx.run_until_parked();

    // Typed, not saved.
    let output = view.read_with(&vcx, |v, cx| {
        v.form().expect("a form").body_states(cx).output.clone()
    });
    output.update_in(&mut vcx, |state, window, cx| {
        state.set_value("mine.md".to_string(), window, cx)
    });
    assert!(
        view.read_with(&vcx, |v, _| v.form().expect("a form").banner().is_none()),
        "nothing to say yet"
    );

    // Somebody else writes the file.
    let text = std::fs::read_to_string(&flow_path).expect("on disk");
    std::fs::write(&flow_path, text.replace("design.md", "theirs.md")).expect("write");
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.reload_flow_graphs(None, window, cx)
    });
    vcx.run_until_parked();

    assert_eq!(
        view.read_with(&vcx, |v, cx| v
            .form()
            .expect("a form")
            .body_states(cx)
            .output
            .read(cx)
            .value()
            .to_string()),
        "theirs.md",
        "the file wins"
    );
    assert!(
        told_about_dropped_typing(&ws, &vcx),
        "and the typing that went with it is reported"
    );
}

/// Adding a node throws the same typing away, and rebuilds the inspector for
/// the node it just made — so a banner would be gone before it was read.
#[gpui::test]
async fn typing_lost_to_adding_a_node_is_reported(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("design", window, cx)
    });
    vcx.run_until_parked();
    let output = view.read_with(&vcx, |v, cx| {
        v.form().expect("a form").body_states(cx).output.clone()
    });
    output.update_in(&mut vcx, |state, window, cx| {
        state.set_value("mine.md".to_string(), window, cx)
    });

    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.add_node(&flow_path, view.clone(), window, cx)
    });
    vcx.run_until_parked();

    assert!(
        told_about_dropped_typing(&ws, &vcx),
        "the typed output went nowhere and nothing said so"
    );
}

/// The node being edited is renamed out from under the inspector. Nothing comes
/// back to compare against, and that is the case most worth saying.
#[gpui::test]
async fn typing_lost_with_the_node_itself_is_reported(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test("design", window, cx)
    });
    vcx.run_until_parked();
    let output = view.read_with(&vcx, |v, cx| {
        v.form().expect("a form").body_states(cx).output.clone()
    });
    output.update_in(&mut vcx, |state, window, cx| {
        state.set_value("mine.md".to_string(), window, cx)
    });

    // Somebody else renames the node this form is about.
    let text = std::fs::read_to_string(&flow_path).expect("on disk");
    std::fs::write(&flow_path, text.replace("design", "drawing")).expect("write");
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.reload_flow_graphs(None, window, cx)
    });
    vcx.run_until_parked();

    assert!(
        view.read_with(&vcx, |v, _| v.form().is_none()),
        "the node it was about is gone, so the inspector has nothing to show"
    );
    assert!(
        told_about_dropped_typing(&ws, &vcx),
        "and that is exactly when it has to be said"
    );
}

/// A marquee catches several nodes, and one Delete takes all of them — with the
/// references to them swept out of the nodes that stay.
#[gpui::test]
async fn deleting_a_multi_selection_takes_every_node_in_it(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, LONG_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");

    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.delete_nodes(
            &flow_path,
            view.clone(),
            vec!["n2".to_string(), "n3".to_string()],
            window,
            cx,
        )
    });
    vcx.run_until_parked();

    let after = std::fs::read_to_string(&flow_path).expect("on disk");
    let file = daruda_flow::parse::parse_flow_file(&after).expect("still a flow");
    let ids: Vec<&str> = file.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(
        !ids.contains(&"n2") && !ids.contains(&"n3"),
        "both went:\n{after}"
    );
    assert!(
        !after.contains("deps: [n2]") && !after.contains("deps: [n3]"),
        "and nothing still runs after them:\n{after}"
    );
}

/// The last node cannot go, and neither can *every* node — a marquee over the
/// whole graph is the way to ask for that. The differ would leave `nodes:` with
/// nothing under it, which does not parse.
#[gpui::test]
async fn a_selection_of_every_node_is_refused(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");
    let before = std::fs::read_to_string(&flow_path).expect("on disk");

    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.delete_nodes(
            &flow_path,
            view.clone(),
            vec!["design".to_string(), "build".to_string()],
            window,
            cx,
        )
    });
    vcx.run_until_parked();

    assert_eq!(
        std::fs::read_to_string(&flow_path).expect("on disk"),
        before,
        "the file is untouched"
    );
    assert!(
        ws.read_with(&vcx, |ws, _| ws
            .error_history()
            .iter()
            .any(|r| r.dedup_key.as_deref() == Some("flow.delete_node_last"))),
        "and it says why"
    );
}
