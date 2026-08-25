//! Running part of a flow: how far, and what not to pay for twice.
//!
//! `flow_graph_pane/pins.rs` decides when a pin stops holding and
//! `workspace/flow_pins.rs` decides where a reused output comes from, both
//! without a window. These check that the two reach the screen — a badge that
//! says which cards are pinned, and a glyph that is only live when there is one
//! node to stop at.

use super::*;

use crate::workspace::main_area::flow_graph_pane::{
    FlowGraphView, TOOLBAR_PIN_SELECTOR, TOOLBAR_RUN_UNTIL_SELECTOR,
};

/// The graph pane the active lane has open.
fn graph_view(
    ws: &gpui::Entity<crate::workspace::Workspace>,
    vcx: &gpui::VisualTestContext,
) -> gpui::Entity<FlowGraphView> {
    ws.read_with(vcx, |ws, _| {
        ws.active_runtime()
            .panes
            .iter()
            .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
    })
    .expect("the graph pane opened")
}

/// Which nodes are drawing as reused, read back through the canvas.
fn pinned_cards(
    view: &gpui::Entity<FlowGraphView>,
    vcx: &gpui::VisualTestContext,
) -> Vec<daruda_flow::NodeId> {
    let mut pinned: Vec<daruda_flow::NodeId> = view
        .read_with(vcx, |v, cx| v.cards_for_test(cx))
        .into_iter()
        .filter(|(_, (_, accent))| accent == "Pinned")
        .map(|(id, _)| id)
        .collect();
    pinned.sort();
    pinned
}

fn press(vcx: &mut gpui::VisualTestContext, selector: &'static str) {
    let at = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("the toolbar paints {selector}"));
    vcx.simulate_click(at.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
}

/// A pin is dropped when *its* node changes and kept when a neighbour's does.
///
/// The rule the whole feature rests on: clearing everything on any edit would
/// mean re-pinning before every iteration, which is the cost this is meant to
/// remove. Driven through the toolbar and a real reload, so the button's wiring
/// and the card's badge are both part of what is under test.
#[gpui::test]
async fn editing_one_node_leaves_another_nodes_pin_alone(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = graph_view(&ws, &vcx);

    for node in ["design", "build"] {
        view.update_in(&mut vcx, |v, window, cx| {
            v.select_node_for_test(&node.into(), window, cx)
        });
        vcx.run_until_parked();
        press(&mut vcx, TOOLBAR_PIN_SELECTOR);
    }
    assert_eq!(
        pinned_cards(&view, &vcx),
        vec![
            daruda_flow::NodeId::from("build"),
            daruda_flow::NodeId::from("design")
        ],
        "the toolbar's pin did not reach the cards"
    );

    std::fs::write(
        &flow_path,
        TWO_NODE_CHAIN.replace("build it", "build it twice"),
    )
    .expect("write");
    view.update_in(&mut vcx, |v, window, cx| v.reload(window, cx));
    vcx.run_until_parked();
    assert_eq!(
        pinned_cards(&view, &vcx),
        vec![daruda_flow::NodeId::from("design")],
        "the edited node kept its pin, or the untouched one lost it"
    );

    // Pressing again takes the remaining one back — the one button says both
    // directions, following what is selected.
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test(&"design".into(), window, cx)
    });
    vcx.run_until_parked();
    press(&mut vcx, TOOLBAR_PIN_SELECTOR);
    assert!(
        pinned_cards(&view, &vcx).is_empty(),
        "unpinning did nothing"
    );
}

/// A file that stopped loading takes every pin with it: there is no node-by-node
/// comparison left to make, and the safe answer is to pay for the node.
#[gpui::test]
async fn a_flow_that_stops_loading_clears_every_pin(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = graph_view(&ws, &vcx);
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test(&"design".into(), window, cx)
    });
    vcx.run_until_parked();
    press(&mut vcx, TOOLBAR_PIN_SELECTOR);
    assert_eq!(pinned_cards(&view, &vcx).len(), 1, "nothing was pinned");

    std::fs::write(&flow_path, "version: 1\nnodes: [\n").expect("write");
    view.update_in(&mut vcx, |v, window, cx| v.reload(window, cx));
    vcx.run_until_parked();
    // The graph is gone, so the cards cannot answer — the file coming back is
    // what shows whether the pin did.
    std::fs::write(&flow_path, TWO_NODE_CHAIN).expect("write");
    view.update_in(&mut vcx, |v, window, cx| v.reload(window, cx));
    vcx.run_until_parked();
    assert!(
        pinned_cards(&view, &vcx).is_empty(),
        "a pin outlived a file that would not parse"
    );
}

/// A flow that declares a profile, so pressing ▶ or ▶| stops at the profile
/// question instead of starting a real run — which is what makes the selection
/// it was going to run with readable.
const PROFILED_CHAIN: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
profiles:
  cheap:
    agent:
      model: haiku
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write a line
  - id: build
    kind: agent
    deps: [design]
    output: build.md
    prompt: build it
";

/// A finished run in `lane`'s runs directory: `design` passed and its output is
/// there. Written as the lines the engine writes, because reading those back is
/// how a pin finds its source.
fn finished_run_in(lane: &std::path::Path, spec: &str) -> std::path::PathBuf {
    let dir = crate::workspace::flow_paths::runs_dir(lane).join("0000000000000009-00000001-0001");
    std::fs::create_dir_all(&dir).expect("mkdir");
    // The spec is what says which flow the run was, and a pin resolves only
    // against a run whose node matches. The engine writes the *resolved* form;
    // the source resolves the same here because no profile is named.
    std::fs::write(dir.join("run.yaml"), spec).expect("the run's spec");
    std::fs::write(
        dir.join(daruda_flow::journal::JOURNAL_FILE),
        "{\"kind\":\"started\",\"v\":1,\"profile\":null}\n\
         {\"kind\":\"attempt\",\"v\":1,\"node\":\"design\",\"attempt\":1,\"evidence_seq\":1,\
         \"outcome\":{\"result\":\"passed\"},\"spent\":{\"node_runs\":1}}\n",
    )
    .expect("journal");
    std::fs::write(dir.join("design.md"), "already written\n").expect("output");
    std::fs::write(dir.join("DONE"), "").expect("marker");
    dir
}

/// A pin becomes a file the engine will copy, and the whole way through: the
/// card the person pinned, the newest run's journal, the path, and the request.
///
/// Every hop here has been a `Vec::new()` until now, so what this holds is that
/// none of them dropped it.
#[gpui::test]
async fn a_pinned_node_reaches_the_run_as_a_file_to_copy(cx: &mut TestAppContext) {
    let (lane, ws, flow_path, wh) = workspace_with_a_flow(cx, PROFILED_CHAIN);
    let run_dir = finished_run_in(lane.path(), PROFILED_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = graph_view(&ws, &vcx);
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test(&"design".into(), window, cx)
    });
    vcx.run_until_parked();
    press(&mut vcx, TOOLBAR_PIN_SELECTOR);

    press(
        &mut vcx,
        crate::workspace::main_area::flow_graph_pane::TOOLBAR_RUN_SELECTOR,
    );
    // The selection carries the id; which run holds the file is only knowable
    // once the profile is settled, so the request is where that shows up.
    let expected = crate::workspace::flow_request::FlowSelection {
        until: None,
        pinned: vec!["design".into()],
    };
    assert_eq!(
        ws.read_with(&vcx, |ws, _| ws.flow_picker.focused_pick()),
        Some(crate::workspace::command::flow_picker::FlowPick::Profile(
            crate::workspace::command::flow_picker::FlowPurpose::Run,
            flow_path.clone(),
            expected.clone(),
            None,
        )),
        "the pin did not survive the way to the run"
    );

    // And the request the engine is handed carries both axes. Assembled rather
    // than submitted: submitting would start a real run.
    let request = ws
        .update(&mut vcx, |ws, cx| {
            ws.build_flow_request(
                &flow_path,
                None,
                &crate::workspace::flow_request::FlowSelection {
                    until: Some("design".into()),
                    ..expected
                },
                cx,
            )
            .map(|submission| submission.request)
        })
        .expect("the request assembles");
    assert_eq!(request.until, Some("design".into()));
    assert_eq!(
        request.pinned,
        vec![daruda_flow::request::PinnedOutput {
            node: "design".into(),
            from: run_dir.join("design.md"),
        }]
    );
}

/// A pin nothing can satisfy is left out of the request and said out loud. Sent
/// anyway it would refuse the whole run; dropped quietly it would charge for a
/// node the person had told it not to.
#[gpui::test]
async fn a_pin_with_no_finished_output_is_reported_and_not_sent(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, PROFILED_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = graph_view(&ws, &vcx);
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test(&"build".into(), window, cx)
    });
    vcx.run_until_parked();
    press(&mut vcx, TOOLBAR_PIN_SELECTOR);
    press(
        &mut vcx,
        crate::workspace::main_area::flow_graph_pane::TOOLBAR_RUN_SELECTOR,
    );

    // The selection still carries what the person picked — dropping it there
    // would make the pin button look broken. Where it has to be gone is the
    // request, which is also the only place that knows the profile.
    let request = ws
        .update(&mut vcx, |ws, cx| {
            ws.build_flow_request(
                &flow_path,
                None,
                &crate::workspace::flow_request::FlowSelection {
                    until: None,
                    pinned: vec!["build".into()],
                },
                cx,
            )
            .map(|submission| submission.request)
        })
        .expect("the request assembles");
    assert!(
        request.pinned.is_empty(),
        "a pin with nothing behind it was sent to the engine: {:?}",
        request.pinned
    );
    assert!(
        ws.read_with(&vcx, |ws, _| ws
            .error_history()
            .iter()
            .any(|r| r.dedup_key.as_deref() == Some("flow.pin_unavailable"))),
        "the pin was dropped without saying so"
    );
}

/// ▶| is live only with exactly one node selected: a run cannot stop at two
/// places, and it must not quietly become "run everything" at none.
///
/// The flow declares a profile so an *enabled* press is observable without
/// starting anything — it stops at the profile question. Nothing spawns either
/// way, and the picker is the discriminator.
#[gpui::test]
async fn running_as_far_as_a_node_needs_exactly_one_selected(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, PROFILED_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = graph_view(&ws, &vcx);

    // The toolbar has to have been painted before a press can find it, and the
    // open alone does not draw one.
    view.update(&mut vcx, |_, cx| cx.notify());
    vcx.run_until_parked();

    // Nothing selected: no stopping point, so nothing happens.
    press(&mut vcx, TOOLBAR_RUN_UNTIL_SELECTOR);
    assert!(
        !ws.read_with(&vcx, |ws, _| ws.flow_picker.is_open()),
        "▶| ran with nothing selected"
    );

    // One node: the run is asked about, and about that flow.
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test(&"design".into(), window, cx)
    });
    vcx.run_until_parked();
    press(&mut vcx, TOOLBAR_RUN_UNTIL_SELECTOR);
    assert_eq!(
        ws.read_with(&vcx, |ws, _| ws.flow_picker.focused_pick()),
        Some(crate::workspace::command::flow_picker::FlowPick::Profile(
            crate::workspace::command::flow_picker::FlowPurpose::Run,
            flow_path.clone(),
            crate::workspace::flow_request::FlowSelection {
                until: Some("design".into()),
                pinned: Vec::new(),
            },
            None,
        )),
        "the node it was pressed on did not reach the run"
    );
    ws.update(&mut vcx, |ws, cx| ws.close_flow_picker(cx));
    vcx.run_until_parked();

    // Two: shift-click adds the second card, and there is no one node to stop
    // at any more.
    let canvas = view
        .read_with(&vcx, |v, _| v.canvas_for_test().cloned())
        .expect("it drew");
    let card_at = |vcx: &gpui::VisualTestContext, ix: usize| {
        canvas
            .read_with(vcx, |canvas, _| {
                let graph = canvas.graph();
                let node = graph.paint_order().get(ix).copied()?;
                let bounds = graph.node_world_bounds(node)?;
                let origin = canvas.viewport().window_bounds()?.origin;
                let local = canvas.viewport().world_to_screen(bounds.center());
                Some(gpui::point(origin.x + local.x, origin.y + local.y))
            })
            .expect("the card has a place on screen")
    };
    let (first, second) = (card_at(&vcx, 0), card_at(&vcx, 1));
    vcx.simulate_click(first, gpui::Modifiers::none());
    vcx.simulate_click(second, gpui::Modifiers::shift());
    vcx.run_until_parked();
    press(&mut vcx, TOOLBAR_RUN_UNTIL_SELECTOR);
    assert!(
        !ws.read_with(&vcx, |ws, _| ws.flow_picker.is_open()),
        "▶| ran with two nodes selected"
    );
}

/// A pin that went away has to say why on the card it went away from. Editing
/// an upstream node drops a downstream pin — correct, and invisible: the badge
/// went from "reused" to whatever the node's failure policy is, which reads
/// like nothing happened.
#[gpui::test]
async fn a_dropped_pin_names_the_upstream_node_that_did_it(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let view = graph_view(&ws, &vcx);

    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test(&"build".into(), window, cx)
    });
    vcx.run_until_parked();
    press(&mut vcx, TOOLBAR_PIN_SELECTOR);
    assert_eq!(
        pinned_cards(&view, &vcx),
        vec![daruda_flow::NodeId::from("build")]
    );

    // `design` is what `build` reads, so rewriting it is rewriting what build
    // would produce — without touching a character of build.
    std::fs::write(
        &flow_path,
        TWO_NODE_CHAIN.replace("write a line", "write three lines"),
    )
    .expect("write");
    view.update_in(&mut vcx, |v, window, cx| v.reload(window, cx));
    vcx.run_until_parked();

    assert!(
        pinned_cards(&view, &vcx).is_empty(),
        "the pin is gone, which was already true"
    );
    let badge = view
        .read_with(&vcx, |v, cx| v.cards_for_test(cx))
        .get(&daruda_flow::NodeId::from("build"))
        .map(|(badge, _)| badge.clone())
        .expect("build has a card");
    assert_eq!(
        badge,
        crate::surface::strings::flow_graph_unpinned_upstream("design"),
        "and the card says which node took it away"
    );
}

/// The reason has to outlive a reload of a flow that has run before.
///
/// Restoring a finished run's colours rewrites every card, including ones the
/// run said nothing about — so it erased the reason one frame after an edit
/// produced it, for any flow that had ever been run. Which is every flow a
/// person is iterating on, and iterating is when the reason is worth having.
#[gpui::test]
async fn a_previous_runs_colours_do_not_erase_why_a_pin_went(cx: &mut TestAppContext) {
    let (lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let here = ws.update(&mut vcx, |ws, _| ws.active);
    let view = graph_view(&ws, &vcx);

    ws.update(&mut vcx, |ws, _| {
        ws.seed_flow_run_of_for_test(
            here,
            lane.path().join("run"),
            crate::workspace::flow_request::FlowSource::File(flow_path.clone()),
        )
    });
    ws.update(&mut vcx, |ws, cx| {
        ws.apply_flow_event_for_test(
            here,
            &daruda_flow::event::FlowEvent::NodePassed {
                node: "design".into(),
                attempt: 1,
            },
            cx,
        )
    });

    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test(&"build".into(), window, cx)
    });
    vcx.run_until_parked();
    press(&mut vcx, TOOLBAR_PIN_SELECTOR);

    std::fs::write(
        &flow_path,
        TWO_NODE_CHAIN.replace("write a line", "write three lines"),
    )
    .expect("write");
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.reload_flow_graphs(None, window, cx)
    });
    vcx.run_until_parked();

    let badge = |vcx: &gpui::VisualTestContext| {
        view.read_with(vcx, |v, cx| v.cards_for_test(cx))
            .get(&daruda_flow::NodeId::from("build"))
            .map(|(badge, _)| badge.clone())
            .expect("build has a card")
    };
    assert_eq!(
        badge(&vcx),
        crate::surface::strings::flow_graph_unpinned_upstream("design"),
        "the colours came back and the reason survived them"
    );

    // And the other half of the rule: pressing Run is having acted on it.
    view.update_in(&mut vcx, |v, _, cx| v.forget_unpinned(cx));
    vcx.run_until_parked();
    assert_ne!(
        badge(&vcx),
        crate::surface::strings::flow_graph_unpinned_upstream("design"),
        "a run clears it"
    );
}
