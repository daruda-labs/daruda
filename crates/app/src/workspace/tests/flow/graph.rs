//! The graph pane: opening one, framing it, colouring it from a run, and
//! reading the file again.
//!
//! `flow_graph_pane/policy.rs` decides what a reload does and is tested without
//! a window; these check that the decision reaches the screen — a canvas kept,
//! a layout rebuilt, a repaint budget held.

use super::*;

/// A graph pane has to come back after a restart. It persists the flow's
/// path and nothing else — the graph itself is derived from the file, so
/// storing coordinates would let a stale layout outlive the YAML that
/// produced it.
#[gpui::test]
async fn a_flow_graph_pane_survives_a_save_and_restore(cx: &mut TestAppContext) {
    let (lane, ws, flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let (saved_workspace, saved_projects) = {
        let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
        ws.update_in(&mut vcx, |ws, window, cx| {
            ws.open_flow_graph(&flow_path, window, cx)
        });
        ws.read_with(&vcx, |ws, app_cx| ws.snapshot_for_disk(app_cx))
            .expect("snapshot_for_disk")
    };
    let saved_tabs = &saved_projects[0].lanes[0].tabs;
    assert!(
        saved_tabs.iter().any(|t| matches!(
            &t.layout,
            daruda_store::project::SerializedLayout::Leaf {
                content: daruda_store::project::SerializedPaneContent::FlowGraph(fg),
                ..
            } if fg.path == flow_path
        )),
        "saved state must carry a flow-graph leaf for {}",
        flow_path.display()
    );

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(lane.path());
    let (wh2, ws2) = build_workspace_with(cx, &config, Some(project));
    let mut vcx2 = gpui::VisualTestContext::from_window(wh2.into(), cx);
    ws2.update_in(&mut vcx2, |ws2, window, cx| {
        ws2.restore_from_disk(&saved_workspace, &saved_projects, window, cx)
    });
    vcx2.run_until_parked();
    drop(vcx2);

    let restored: Vec<std::path::PathBuf> = ws2.read_with(cx, |ws2, _| {
        ws2.active_runtime()
            .panes
            .iter()
            .filter_map(|p| match &p.content {
                crate::workspace::main_area::pane::PaneContent::FlowGraph(fg) => {
                    Some(fg.path.clone())
                }
                _ => None,
            })
            .collect()
    });
    assert_eq!(
        restored,
        vec![flow_path],
        "the restored lane should hold exactly the graph pane that was saved"
    );
}

/// Opening a graph puts **all** of it on screen.
///
/// This is a behaviour assertion on purpose: it holds only if the canvas was
/// measured, the nodes survived visibility culling against that measurement,
/// and the pane frames the graph to the drawable. Any one of those regressing
/// — a re-vendor that drops the viewport patch, a framing plugin that stopped
/// being registered — lands here rather than in a screenshot nobody took.
#[gpui::test]
async fn opening_a_graph_brings_every_node_into_view(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, LONG_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();

    let canvas = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the lane should hold the graph pane just opened");

    let (offenders, unfitted_width, drawable_width) = canvas.read_with(&vcx, |view, cx| {
        let canvas = view
            .canvas_for_test()
            .expect("a loadable flow draws a canvas")
            .read(cx);
        let viewport = canvas.viewport();
        let drawable = viewport
            .window_bounds()
            .expect("the pane must have been laid out by now");
        // What the graph would span with no framing applied — the premise
        // that makes the assertion below mean anything.
        let unfitted_width = canvas
            .graph()
            .nodes()
            .values()
            .map(|n| f32::from(n.point().x) + f32::from(n.size_ref().width))
            .fold(0.0_f32, f32::max);
        let offenders: Vec<String> = canvas
            .graph()
            .nodes()
            .values()
            .filter_map(|node| {
                let at = viewport.world_to_screen(node.point());
                let size = node.size_ref();
                let right = at.x + viewport.world_length_to_screen(size.width);
                let bottom = at.y + viewport.world_length_to_screen(size.height);
                let inside = at.x >= gpui::px(0.)
                    && at.y >= gpui::px(0.)
                    && right <= drawable.size.width
                    && bottom <= drawable.size.height;
                (!inside).then(|| format!("({:?},{:?})..({right:?},{bottom:?})", at.x, at.y))
            })
            .collect();
        (offenders, unfitted_width, f32::from(drawable.size.width))
    });

    assert!(
        unfitted_width > drawable_width,
        "the fixture no longer outgrows the drawable ({unfitted_width} vs {drawable_width}), so \
         this test would pass without any framing — lengthen the chain (but stay under the \
         ZOOM_MIN ceiling, see LONG_CHAIN)"
    );
    assert!(
        offenders.is_empty(),
        "every node should sit inside the drawable, these did not: {offenders:?}"
    );
}

/// Framing a graph to the pane must not magnify one that already fits.
///
/// A node card's text does not scale with its box, so a zoomed-in single-node
/// flow reads as a giant mostly-empty card — worse than the 1:1 view it
/// replaced. "All of it in view" is already true at 1:1 here; the framing only
/// has licence to shrink.
#[gpui::test]
async fn a_graph_smaller_than_the_pane_is_centred_not_magnified(cx: &mut TestAppContext) {
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
        .expect("the lane should hold the graph pane just opened");

    let (zoom, node_left, node_top) = view.read_with(&vcx, |view, cx| {
        let canvas = view
            .canvas_for_test()
            .expect("a loadable flow draws a canvas")
            .read(cx);
        let viewport = canvas.viewport();
        let node = canvas.graph().nodes().values().next().expect("one node");
        let at = viewport.world_to_screen(node.point());
        (viewport.zoom(), at.x, at.y)
    });

    assert!(
        zoom <= 1.0,
        "a graph that already fits was magnified to {zoom}"
    );
    assert!(
        node_left > gpui::px(0.) && node_top > gpui::px(0.),
        "the lone node should be centred, not parked at the origin: ({node_left:?}, {node_top:?})"
    );
}

/// A saved graph tab whose flow file is gone by the next launch.
///
/// The pane persists a path, not the graph, so the file is free to move or be
/// deleted between sessions. That has to arrive as the pane saying which path
/// it could not read — a blank canvas would look like a flow with no nodes,
/// which is a different thing entirely. This is the one `FlowGraphError`
/// variant no screenshot scenario can reach: the picker only ever offers files
/// that exist.
#[gpui::test]
async fn a_restored_graph_whose_file_vanished_says_so(cx: &mut TestAppContext) {
    let (lane, ws, flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let (saved_workspace, saved_projects) = {
        let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
        ws.update_in(&mut vcx, |ws, window, cx| {
            ws.open_flow_graph(&flow_path, window, cx)
        });
        ws.read_with(&vcx, |ws, app_cx| ws.snapshot_for_disk(app_cx))
            .expect("snapshot_for_disk")
    };

    std::fs::remove_file(&flow_path).expect("remove the flow the tab was saved for");

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(lane.path());
    let (wh2, ws2) = build_workspace_with(cx, &config, Some(project));
    let mut vcx2 = gpui::VisualTestContext::from_window(wh2.into(), cx);
    ws2.update_in(&mut vcx2, |ws2, window, cx| {
        ws2.restore_from_disk(&saved_workspace, &saved_projects, window, cx)
    });
    vcx2.run_until_parked();

    let view = ws2
        .read_with(&vcx2, |ws2, _| {
            ws2.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the tab should still be restored — the pane reports the failure");

    let named = view.read_with(&vcx2, |view, _| match view.unreadable_for_test() {
        Some(crate::workspace::main_area::flow_graph_pane::FlowGraphError::Read {
            path, ..
        }) => path.clone(),
        other => panic!("expected a Read failure naming the path, got {other:?}"),
    });
    assert_eq!(named, flow_path);
}

/// A run drives the colour of the graph pane drawing its flow.
///
/// The states are accumulated on the run and stamped onto the cards the
/// canvas holds, so this reads them back through the canvas — the workspace
/// having the right states means nothing if they never reach the graph the
/// renderer draws from.
#[gpui::test]
async fn a_run_colours_the_graph_of_the_flow_it_is_of(cx: &mut TestAppContext) {
    let (lane, ws, flow_path, wh) = workspace_with_a_flow(cx, LONG_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();

    let here = ws.update(&mut vcx, |ws, _| ws.active);
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane just opened");

    ws.update(&mut vcx, |ws, _| {
        ws.seed_flow_run_of_for_test(
            here,
            lane.path().join("run"),
            crate::workspace::flow_request::FlowSource::File(flow_path.clone()),
        )
    });

    let accent_of =
        |view: &gpui::Entity<crate::workspace::main_area::flow_graph_pane::FlowGraphView>,
         vcx: &mut gpui::VisualTestContext,
         node: &str| {
            view.read_with(vcx, |v, cx| {
                v.cards_for_test(cx)
                    .get(node)
                    .map(|(_, accent)| accent.clone())
                    .unwrap_or_else(|| panic!("no card for {node}"))
            })
        };

    assert_eq!(
        accent_of(&view, &mut vcx, "n1"),
        "Pending",
        "nothing has run yet"
    );

    ws.update(&mut vcx, |ws, cx| {
        ws.apply_flow_event_for_test(
            here,
            &daruda_flow::event::FlowEvent::NodeStarted {
                node: "n1".into(),
                attempt: 1,
            },
            cx,
        )
    });
    assert_eq!(accent_of(&view, &mut vcx, "n1"), "Running");

    ws.update(&mut vcx, |ws, cx| {
        ws.apply_flow_event_for_test(
            here,
            &daruda_flow::event::FlowEvent::NodePassed {
                node: "n1".into(),
                attempt: 1,
            },
            cx,
        )
    });
    assert_eq!(accent_of(&view, &mut vcx, "n1"), "Passed");
    assert_eq!(
        accent_of(&view, &mut vcx, "n2"),
        "Pending",
        "a node the run has not reached keeps its pending card"
    );

    // The transition the engine says a host cannot infer.
    ws.update(&mut vcx, |ws, cx| {
        ws.apply_flow_event_for_test(
            here,
            &daruda_flow::event::FlowEvent::Rerunning {
                gate: "n6".into(),
                members: vec!["n1".into()],
            },
            cx,
        )
    });
    assert_eq!(
        accent_of(&view, &mut vcx, "n1"),
        "Pending",
        "a re-derived node must lose its pass on the card too"
    );
}

/// A resumed run colours nothing, and that is the decision, not an oversight.
///
/// `resume::prepare` reads `run.yaml` and points `flow_dir` at the run
/// directory, so the `.daruda/flows/x.yaml` a picked-up run came from is not
/// recoverable. Colouring the graph anyway would mean guessing which open pane
/// the run belongs to. Closing the gap means recording the origin at run
/// start — a host/engine contract change, deliberately out of scope.
#[gpui::test]
async fn a_resumed_run_leaves_an_open_graph_alone(cx: &mut TestAppContext) {
    let (lane, ws, flow_path, wh) = workspace_with_a_flow(cx, LONG_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();

    let here = ws.update(&mut vcx, |ws, _| ws.active);
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane just opened");

    let run_dir = lane.path().join("run");
    ws.update(&mut vcx, |ws, _| {
        ws.seed_flow_run_of_for_test(
            here,
            run_dir.clone(),
            crate::workspace::flow_request::FlowSource::Resumed { run_dir },
        )
    });
    ws.update(&mut vcx, |ws, cx| {
        ws.apply_flow_event_for_test(
            here,
            &daruda_flow::event::FlowEvent::NodeStarted {
                node: "n1".into(),
                attempt: 1,
            },
            cx,
        )
    });

    let accent = view.read_with(&vcx, |v, cx| {
        v.cards_for_test(cx)
            .get("n1")
            .map(|(_, accent)| accent.clone())
            .expect("card for n1")
    });
    assert_eq!(
        accent, "Pending",
        "a run that cannot name its flow file must not colour a pane"
    );
}

/// What a run costs the window, measured rather than argued.
///
/// The design asked for a run's repaints to be "confined to the canvas
/// subtree". Two things about that turned out to be worth writing down.
///
/// It is not reachable as phrased: `Window::mark_view_dirty` marks a dirty
/// view's **ancestors** too, so a notified canvas necessarily re-runs
/// `Workspace::render`. What `.cached()` buys is sibling subtrees reusing
/// their *paint*, and gpui exposes no counter for that — so the part of the
/// claim that matters most is verified by construction (the walker embeds this
/// view through `AnyView::cached`, and the run notifies the canvas entity, not
/// the window) rather than by this test.
///
/// What this test does hold is the budget, with the numbers it was written
/// against: one event, one frame, one pass over the cards. Measured with a
/// six-node flow — `NodeStarted` / `NodePassed` / `Rerunning` each cost 1
/// window render and 6 card draws. An event that moves only the graph costs a
/// frame that would not otherwise happen; one that also moves the stage costs
/// the same single frame either way, because both notifies land in one effect
/// cycle. That coalescing is gpui's, not ours, which makes this a record of
/// the cost more than a guard against losing it.
#[gpui::test]
async fn one_run_event_costs_one_frame(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::CARDS_DRAWN;
    use crate::workspace::render::WORKSPACE_RENDERS;

    let (lane, ws, flow_path, wh) = workspace_with_a_flow(cx, LONG_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.update(&mut vcx, |ws, _| ws.active);
    ws.update(&mut vcx, |ws, _| {
        ws.seed_flow_run_of_for_test(
            here,
            lane.path().join("run"),
            crate::workspace::flow_request::FlowSource::File(flow_path.clone()),
        )
    });
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();

    // One of each kind: an event that moves the stage *and* the graph, one that
    // moves only the graph, and the repair transition that rewrites several
    // cards at once.
    for (label, event) in [
        (
            "NodeStarted",
            daruda_flow::event::FlowEvent::NodeStarted {
                node: "n3".into(),
                attempt: 1,
            },
        ),
        (
            "NodePassed",
            daruda_flow::event::FlowEvent::NodePassed {
                node: "n3".into(),
                attempt: 1,
            },
        ),
        (
            "Rerunning",
            daruda_flow::event::FlowEvent::Rerunning {
                gate: "n6".into(),
                members: vec!["n1".into(), "n2".into(), "n3".into()],
            },
        ),
    ] {
        WORKSPACE_RENDERS.with(|n| n.set(0));
        CARDS_DRAWN.with(|n| n.set(0));
        ws.update(&mut vcx, |ws, cx| {
            ws.apply_flow_event_for_test(here, &event, cx)
        });
        vcx.run_until_parked();

        let frames = WORKSPACE_RENDERS.with(|n| n.get());
        let cards = CARDS_DRAWN.with(|n| n.get());
        assert_eq!(
            frames, 1,
            "{label} cost {frames} frames; a run event may cost at most one"
        );
        // The canvas has no per-card caching, so a frame that draws the graph
        // draws all of it. Asserting the exact count is what would catch a
        // second pass over the cards inside one frame.
        assert_eq!(cards, 6, "{label} drew {cards} cards for a six-node flow");
    }
}

/// D5: opening a graph does not ask which profile to use.
///
/// Every other purpose puts a flow that declares profiles through a second
/// question. A picture of the file's shape is not worth two prompts, and the
/// one-line guard implementing that decision is the sort nobody notices going.
#[gpui::test]
async fn choosing_a_graph_never_asks_for_a_profile(cx: &mut TestAppContext) {
    const WITH_PROFILES: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
profiles:
  quick:
    agent:
      model: haiku
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write a line
";
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, WITH_PROFILES);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);

    // The control: the same file under a purpose that *does* ask.
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_picker(
            crate::workspace::command::flow_picker::FlowPurpose::Validate,
            cx,
        );
        ws.execute_flow_picker_selection(window, cx);
    });
    assert!(
        ws.read_with(&vcx, |ws, _| ws.flow_picker.choosing().is_some()),
        "a flow with profiles asks — for every purpose but Graph"
    );

    // The control left it asking; a fresh question is what Graph is answering.
    ws.update(&mut vcx, |ws, _| ws.flow_picker.close());
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_picker(
            crate::workspace::command::flow_picker::FlowPurpose::Graph,
            cx,
        );
        ws.execute_flow_picker_selection(window, cx);
    });
    vcx.run_until_parked();
    assert!(
        ws.read_with(&vcx, |ws, _| ws.flow_picker.choosing().is_none()),
        "Graph goes straight to the defaults graph"
    );
    let paths: Vec<std::path::PathBuf> = ws.read_with(&vcx, |ws, _| {
        ws.active_runtime()
            .panes
            .iter()
            .filter_map(|p| p.flow_graph_content().map(|fg| fg.path.clone()))
            .collect()
    });
    assert_eq!(paths, vec![flow_path], "and it drew");
}

/// A file that fails validation says what a person can read.
///
/// `ValidationIssue.message` is the engine's developer detail and says so; the
/// wording belongs to `s::flow_issue`, keyed off `kind`. Reaching for the
/// message is the mistake this guards.
#[gpui::test]
async fn a_flow_that_fails_validation_reports_it_in_the_pane(cx: &mut TestAppContext) {
    const CLASHING: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: one
    kind: agent
    output: same.md
    prompt: a
  - id: two
    kind: agent
    output: same.md
    prompt: b
";
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, CLASHING);
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
        .expect("the pane opened");
    let issues = view.read_with(&vcx, |v, _| match v.unreadable_for_test() {
        Some(crate::workspace::main_area::flow_graph_pane::FlowGraphError::Validate { issues }) => {
            issues.clone()
        }
        other => panic!("expected a validation failure, got {other:?}"),
    });
    assert!(!issues.is_empty(), "it says what is wrong");
    assert!(
        issues.iter().any(|line| line.contains("two")),
        "and names the node it is about: {issues:?}"
    );
    // The engine's own `message` is developer detail and says so. None of it may
    // reach the pane — being worded by `s::flow_issue` is the whole point.
    let engine_text: Vec<String> = match daruda_flow::load(CLASHING, None) {
        Err(daruda_flow::FlowError::Validate(raw)) => {
            raw.iter().map(|i| i.message.clone()).collect()
        }
        other => panic!("the fixture has to fail validation, got {other:?}"),
    };
    for developer_line in &engine_text {
        assert!(
            !issues
                .iter()
                .any(|line| line.contains(developer_line.as_str())),
            "the engine's wording reached the pane: {developer_line:?} in {issues:?}"
        );
    }
}

/// Reading the same bytes again does nothing.
///
/// This is the rule a watcher stands on (D5a): our own writes come back as
/// events, fsevents reports touches as well as writes, and the refresh key can
/// land while a watcher is already reloading. If any of those rebuilt the
/// graph, pointing a watcher at this pane would be a loop.
#[gpui::test]
async fn reloading_an_unchanged_flow_rebuilds_nothing(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::CARDS_DRAWN;

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
    let canvas_before = view
        .read_with(&vcx, |v, _| {
            v.canvas_for_test().map(gpui::Entity::entity_id)
        })
        .expect("it drew");

    // Touch the file without changing a byte — what an editor's save of an
    // unchanged buffer, and our own write, both look like.
    let same = std::fs::read_to_string(&flow_path).expect("readable");
    std::fs::write(&flow_path, &same).expect("write");

    CARDS_DRAWN.with(|n| n.set(0));
    view.update_in(&mut vcx, |v, window, cx| v.reload(window, cx));
    vcx.run_until_parked();

    assert_eq!(
        CARDS_DRAWN.with(|n| n.get()),
        0,
        "an unchanged file must not repaint the graph"
    );
    assert_eq!(
        view.read_with(&vcx, |v, _| v
            .canvas_for_test()
            .map(gpui::Entity::entity_id)),
        Some(canvas_before),
        "and must not build a second canvas"
    );
}

/// A file that changed is read again, and one that stopped loading says so.
#[gpui::test]
async fn reloading_a_changed_flow_redraws_it(cx: &mut TestAppContext) {
    const TWO_NODES: &str = "\
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
  - id: build
    kind: agent
    deps: [design]
    output: build.md
    prompt: build it
";
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
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.cards_for_test(cx).len()),
        1,
        "one node to start"
    );

    std::fs::write(&flow_path, TWO_NODES).expect("write");
    view.update_in(&mut vcx, |v, window, cx| v.reload(window, cx));
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.cards_for_test(cx).len()),
        2,
        "the added node is drawn"
    );

    // And a file that stops loading reports it rather than keeping the graph.
    std::fs::write(&flow_path, "version: 1\nnodes: [\n").expect("write");
    view.update_in(&mut vcx, |v, window, cx| v.reload(window, cx));
    vcx.run_until_parked();
    assert!(
        matches!(
            view.read_with(&vcx, |v, _| v.unreadable_for_test().cloned()),
            Some(crate::workspace::main_area::flow_graph_pane::FlowGraphError::Parse { .. })
        ),
        "a broken file is reported, not ignored"
    );
}

/// A flow file created outside the app appears in the panel, and an edit made
/// outside it reaches the open graph.
///
/// Driven through `apply_flows_event` rather than through a real filesystem
/// event: what a watcher delivers is one coalesced "something changed", and the
/// FSEvents leg of it is covered where the watcher lives
/// (`hooks::flow_watcher`, `#[ignore]`d because it needs a real event stream).
/// What is worth pinning here is the half that used to be missing — the list
/// cache dropped, and every open graph re-read.
#[gpui::test]
async fn a_change_on_disk_reaches_the_panel_and_the_open_graph(cx: &mut TestAppContext) {
    const TWO_NODES: &str = "\
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
  - id: build
    kind: agent
    deps: [design]
    output: build.md
    prompt: build it
";
    let (lane, ws, flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
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

    // Read the list once so there is a cache to invalidate — the state the
    // panel is in whenever the tab is up.
    let listed = ws.update(&mut vcx, |ws, cx| {
        ws.set_right_dock_view(daruda_store::project::RightDockView::Flows, cx);
        ws.flow_list_for_panel()
    });
    assert_eq!(listed.len(), 1, "one flow to start: {listed:?}");

    // Somebody else's editor: a new file, and an edit to the open one.
    let flows = crate::workspace::flow_paths::flows_dir(lane.path());
    std::fs::write(flows.join("release.yaml"), ONE_AGENT).expect("write");
    std::fs::write(&flow_path, TWO_NODES).expect("write");

    ws.update_in(&mut vcx, |ws, window, cx| ws.apply_flows_event(window, cx));
    vcx.run_until_parked();

    let listed = ws.update(&mut vcx, |ws, _| ws.flow_list_for_panel());
    let mut names: Vec<String> = listed
        .iter()
        .filter_map(|f| f.path.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["release.yaml".to_string(), "ship.yaml".to_string()],
        "the file added from outside is listed"
    );
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.cards_for_test(cx).len()),
        2,
        "and the open graph shows the node that was added to it"
    );
}

/// A watcher event that changed nothing costs no frame.
///
/// The watcher fires for our own writes and for anything that merely touches a
/// flow file, and gpui has no partial redraw — one notify here repaints the whole
/// window. The panel's list is the only thing rendered from the workspace that a
/// flow file can change, so a hidden Flows tab has nothing to repaint for.
#[gpui::test]
async fn a_flow_event_that_changed_nothing_costs_no_frame(cx: &mut TestAppContext) {
    use crate::workspace::render::WORKSPACE_RENDERS;

    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();

    // A touch, not a write: the same bytes back into the file.
    let same = std::fs::read_to_string(&flow_path).expect("readable");
    std::fs::write(&flow_path, &same).expect("write");

    WORKSPACE_RENDERS.with(|n| n.set(0));
    ws.update_in(&mut vcx, |ws, window, cx| ws.apply_flows_event(window, cx));
    vcx.run_until_parked();
    assert_eq!(
        WORKSPACE_RENDERS.with(|n| n.get()),
        0,
        "nothing changed and nothing was on screen to change"
    );

    // With the Flows tab up, the list is rendered from the workspace and a file
    // added from outside has to reach it.
    ws.update(&mut vcx, |ws, cx| {
        ws.set_right_dock_view(daruda_store::project::RightDockView::Flows, cx)
    });
    vcx.run_until_parked();
    WORKSPACE_RENDERS.with(|n| n.set(0));
    ws.update_in(&mut vcx, |ws, window, cx| ws.apply_flows_event(window, cx));
    vcx.run_until_parked();
    assert_eq!(
        WORKSPACE_RENDERS.with(|n| n.get()),
        1,
        "the panel's list is read again, once"
    );
}

/// The graph follows the theme. Its colours are taken when the canvas is built
/// and there is no setter for them, so a switch has to rebuild — and the node
/// that was selected has to survive that, the same way it survives a reload.
#[gpui::test]
async fn switching_the_theme_recolours_the_graph(cx: &mut TestAppContext) {
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
        v.select_node_for_test(&"design".into(), window, cx)
    });
    vcx.run_until_parked();
    let dark_canvas = view
        .read_with(&vcx, |v, _| v.canvas_for_test().cloned())
        .expect("it drew");

    vcx.update(|_, cx| {
        assert!(
            crate::ui::theme::apply_ui_theme("daruda_light", cx),
            "the light theme is installed"
        )
    });
    vcx.run_until_parked();

    let light_canvas = view
        .read_with(&vcx, |v, _| v.canvas_for_test().cloned())
        .expect("it drew again");
    assert_ne!(
        dark_canvas.entity_id(),
        light_canvas.entity_id(),
        "the canvas was built again, which is the only way its colours change"
    );
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.selected_node(cx)),
        Some("design".into()),
        "and the node that was selected still is"
    );
}

/// Saving a field keeps the canvas. The graph's shape did not change, so
/// rebuilding it would move the picture — and the pan, the zoom and the
/// selection with it — under the person who pressed Save.
#[gpui::test]
async fn a_save_that_changes_no_shape_keeps_the_canvas(cx: &mut TestAppContext) {
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
        v.select_node_for_test(&"design".into(), window, cx)
    });
    vcx.run_until_parked();
    let before = view
        .read_with(&vcx, |v, _| {
            v.canvas_for_test().map(gpui::Entity::entity_id)
        })
        .expect("it drew");

    let output = view.read_with(&vcx, |v, cx| {
        v.form().expect("a form").body_states(cx).output.clone()
    });
    output.update_in(&mut vcx, |state, window, cx| {
        state.set_value("spec.md".to_string(), window, cx)
    });
    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::Save));
    vcx.run_until_parked();

    assert_eq!(
        view.read_with(&vcx, |v, _| v
            .canvas_for_test()
            .map(gpui::Entity::entity_id)),
        Some(before),
        "the same canvas, re-stamped rather than rebuilt"
    );
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.cards_for_test(cx).len()),
        2,
        "and it still draws the flow"
    );
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.selected_node(cx)),
        Some("design".into()),
        "with the same node selected"
    );

    // A node arriving *is* a shape change, and that has to rebuild.
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.add_node(&flow_path, view.clone(), window, cx)
    });
    vcx.run_until_parked();
    assert_ne!(
        view.read_with(&vcx, |v, _| v
            .canvas_for_test()
            .map(gpui::Entity::entity_id)),
        Some(before),
        "a new node means a new layout, which means a new canvas"
    );
}

/// A reload draws the flow; the colours are the run's, and they have to
/// survive one. Editing a field mid-run otherwise greys out every node that
/// already passed, until the next event happens to repaint it.
#[gpui::test]
async fn a_reload_keeps_the_colours_of_the_run(cx: &mut TestAppContext) {
    let (lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let here = ws.update(&mut vcx, |ws, _| ws.active);
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");

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
    let accent = |vcx: &mut gpui::VisualTestContext, node: &str| {
        view.read_with(vcx, |v, cx| {
            v.cards_for_test(cx)
                .get(node)
                .map(|(_, accent)| accent.clone())
                .unwrap_or_else(|| panic!("no card for {node}"))
        })
    };
    assert_eq!(accent(&mut vcx, "design"), "Passed");

    // The file changes under the run — an edit, a watcher event, either way a
    // reload. The shape is the same, so the canvas stays and the cards are
    // stamped again.
    let text = std::fs::read_to_string(&flow_path).expect("on disk");
    std::fs::write(&flow_path, text.replace("build it", "build it twice")).expect("write");
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.reload_flow_graphs(None, window, cx)
    });
    vcx.run_until_parked();

    assert_eq!(
        accent(&mut vcx, "design"),
        "Passed",
        "the node that already passed still says so"
    );
}

/// A run's colours are about the nodes it started with. Delete one mid-run and
/// give its name to another, and the freed id would otherwise wear the first
/// node's state — green on something that never ran.
#[gpui::test]
async fn a_run_does_not_colour_a_flow_whose_nodes_have_changed(cx: &mut TestAppContext) {
    let (lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let here = ws.update(&mut vcx, |ws, _| ws.active);
    let view = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find_map(|p| p.flow_graph_content().map(|fg| fg.view.clone()))
        })
        .expect("the graph pane opened");

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
    let accent = |vcx: &mut gpui::VisualTestContext, node: &str| {
        view.read_with(vcx, |v, cx| {
            v.cards_for_test(cx)
                .get(node)
                .map(|(_, accent)| accent.clone())
                .unwrap_or_else(|| panic!("no card for {node}"))
        })
    };
    assert_eq!(accent(&mut vcx, "design"), "Passed");

    // The node that passed is gone and `build` has taken its name.
    let rewritten = std::fs::read_to_string(&flow_path)
        .expect("on disk")
        .replace(
            "  - id: design\n    kind: agent\n    output: design.md\n    prompt: write a line\n",
            "",
        )
        .replace("  - id: build", "  - id: design")
        .replace("    deps: [design]\n", "");
    std::fs::write(&flow_path, rewritten).expect("write");
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.reload_flow_graphs(None, window, cx)
    });
    vcx.run_until_parked();

    assert_eq!(
        accent(&mut vcx, "design"),
        "Pending",
        "the id came back on a different node, and the run never ran that one"
    );
}

/// The menu that holds Add Node has to be reachable from the graph itself.
///
/// It was attached to the pane *header*, which is drawn only when the tab is
/// split — so a graph opened on its own had no way to reach it at all, and Add
/// Node was unreachable. Right-clicking the pane body is that way.
#[gpui::test]
async fn right_clicking_a_graph_opens_the_pane_menu(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, TWO_NODE_CHAIN);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(&flow_path, window, cx)
    });
    vcx.run_until_parked();
    let pane_id = ws
        .read_with(&vcx, |ws, _| {
            ws.active_runtime()
                .panes
                .iter()
                .find(|p| p.flow_graph_content().is_some())
                .map(|p| p.id)
        })
        .expect("the graph pane opened");
    assert!(
        ws.read_with(&vcx, |ws, _| ws.main_area.popup_menu_deploy.is_none()),
        "nothing deployed yet"
    );

    // The click itself, not the handler it should reach: calling the opener
    // directly would pass with the wiring absent, which is what was wrong.
    let _ = pane_id;
    vcx.simulate_mouse_down(
        gpui::point(gpui::px(300.), gpui::px(300.)),
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    vcx.run_until_parked();

    assert!(
        ws.read_with(&vcx, |ws, _| ws.main_area.popup_menu_deploy.is_some()),
        "a right-click on the graph has to reach the menu"
    );
}

/// The toolbar's Add is a second way into the same op, so what has to be tested
/// is the wiring: the view emits, the workspace writes the file.
///
/// Without it the button would be a button that does nothing — the menu path
/// would still pass every test it has.
#[gpui::test]
async fn the_toolbar_add_reaches_the_file(cx: &mut TestAppContext) {
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
    let before = view.read_with(&vcx, |v, cx| v.cards_for_test(cx).len());

    view.update(&mut vcx, |_, cx| cx.emit(FlowGraphEvent::AddNode));
    vcx.run_until_parked();

    assert_eq!(
        view.read_with(&vcx, |v, cx| v.cards_for_test(cx).len()),
        before + 1,
        "the graph has the new node"
    );
    let text = std::fs::read_to_string(&flow_path).expect("on disk");
    assert!(
        daruda_flow::parse::parse_flow_file(&text)
            .expect("still a flow")
            .nodes
            .len()
            == before + 1,
        "and so does the file:\n{text}"
    );
}

/// A press on the toolbar must not reach the canvas underneath.
///
/// The toolbar sits inside the canvas's own bounds, so without `.occlude()`
/// both get the same mouse-down and the canvas starts a marquee — the button
/// then only worked if you dragged off it and back.
#[gpui::test]
async fn a_press_on_the_toolbar_does_not_start_a_drag(cx: &mut TestAppContext) {
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
        v.select_node_for_test(&"design".into(), window, cx)
    });
    vcx.run_until_parked();
    let canvas = view
        .read_with(&vcx, |v, _| v.canvas_for_test().cloned())
        .expect("it drew");

    // Just inside the canvas's top-right corner, where the toolbar is.
    let on_toolbar = canvas.read_with(&vcx, |c, _| {
        let b = c.viewport().window_bounds().expect("measured");
        gpui::point(
            b.origin.x + b.size.width - gpui::px(16.0),
            b.origin.y + gpui::px(16.0),
        )
    });
    vcx.simulate_mouse_down(
        on_toolbar,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    vcx.run_until_parked();

    assert_eq!(
        view.read_with(&vcx, |v, cx| v.selection(cx)),
        crate::workspace::main_area::flow_graph_pane::Selection::One("design".into()),
        "the canvas never saw the press, so the selection stands"
    );
}

/// A first open is a reload against nothing, and it has to keep the same rule:
/// bytes that were *read* are kept even when they do not load.
///
/// Dropping them made the first watcher tick on a broken file look like a
/// change — a rebuild and a notify for a file nobody had touched.
#[gpui::test]
async fn a_file_that_does_not_load_keeps_its_bytes_from_the_first_open(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, "nodes: [\n");
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
        .expect("the pane opened even though the flow does not load");

    assert!(
        view.read_with(&vcx, |v, _| v.unreadable_for_test().is_some()),
        "it says why"
    );
    assert!(
        view.read_with(&vcx, |v, _| v.text().is_some()),
        "and keeps the bytes, so reading them again is nothing to do"
    );
}

/// Three nodes, and a `docs` nothing runs after — something to draw a line to.
const A_SPARE_NODE: &str = "\
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
  - id: build
    kind: agent
    deps: [design]
    output: build.md
    prompt: build it
  - id: docs
    kind: agent
    output: docs.md
    prompt: write them
";

/// Drawing a line has to reach the file, because the file is the graph: the
/// canvas edge the vendor plugin added is thrown away on the same notify, and
/// the line only comes back because the write's reload draws it again.
///
/// Asserts the direction on disk, which is the claim `connect.rs` makes in
/// isolation — here it is carried through the event, the op and the differ.
#[gpui::test]
async fn a_line_drawn_between_two_cards_reaches_the_file(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::FlowGraphEvent;

    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, A_SPARE_NODE);
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

    view.update(&mut vcx, |_, cx| {
        cx.emit(FlowGraphEvent::Connect {
            out_of: "build".into(),
            into: "docs".into(),
        })
    });
    vcx.run_until_parked();

    let text = std::fs::read_to_string(&flow_path).expect("on disk");
    let file = daruda_flow::parse::parse_flow_file(&text).expect("still a flow");
    let docs = file.nodes.iter().find(|n| n.id == "docs").expect("docs");
    assert_eq!(
        docs.deps,
        vec![daruda_flow::NodeId::from("build")],
        "the dep is on the card drawn into:\n{text}"
    );
    let build = file.nodes.iter().find(|n| n.id == "build").expect("build");
    assert_eq!(
        build.deps,
        vec![daruda_flow::NodeId::from("design")],
        "and the card drawn out of gained nothing:\n{text}"
    );
}

/// A line that would make the flow run in a circle is refused — the file is
/// left exactly as it was, **and the canvas does not keep the line**.
///
/// The refusal is the engine's, not ours: `edit_flow` puts the whole candidate
/// back through `load`, so this is the same gate every other flow edit passes.
/// Asserting the file rather than the toast, as every other refusal test here
/// does — a toast that said so over a file that changed would be worse than no
/// toast at all.
///
/// The edge count is the half that only a refusal can check. On a *successful*
/// write the reload rebuilds the canvas from the file, which would hide a
/// phantom left behind; here nothing is rebuilt, so a `DropEdge` that did not
/// fire shows up as an extra line that the file does not declare.
#[gpui::test]
async fn a_line_that_would_loop_is_refused_and_not_left_on_the_canvas(cx: &mut TestAppContext) {
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
    let before_text = std::fs::read_to_string(&flow_path).expect("on disk");
    let before_edges = view.read_with(&vcx, |v, cx| v.drawn_edges_for_test(cx));

    // `build` already runs after `design`; running `design` after `build` too
    // closes the loop.
    view.update(&mut vcx, |v, cx| {
        v.draw_edge_for_test(&"build".into(), &"design".into(), cx)
    });
    vcx.run_until_parked();

    assert_eq!(
        std::fs::read_to_string(&flow_path).expect("on disk"),
        before_text,
        "a cycle leaves the file untouched"
    );
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.drawn_edges_for_test(cx)),
        before_edges,
        "and the refused line is off the canvas, so the picture still matches the file"
    );
}

/// The half daruda owns, end to end: an edge appears on the canvas the way a
/// finished port drag leaves it, and the file comes back saying so.
///
/// The two tests above emit `Connect` directly and so never run
/// `reconcile_edges` — the code holding the invariant. This one does, which is
/// what makes them more than a test of the differ: without it, a reconcile that
/// found nothing would leave every one of them passing.
#[gpui::test]
async fn an_edge_that_appears_on_the_canvas_is_written_and_taken_off_it(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, A_SPARE_NODE);
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
    // One already: `build` runs after `design`.
    let before = view.read_with(&vcx, |v, cx| v.drawn_edges_for_test(cx));

    view.update(&mut vcx, |v, cx| {
        v.draw_edge_for_test(&"build".into(), &"docs".into(), cx)
    });
    vcx.run_until_parked();

    let text = std::fs::read_to_string(&flow_path).expect("on disk");
    let file = daruda_flow::parse::parse_flow_file(&text).expect("still a flow");
    let docs = file.nodes.iter().find(|n| n.id == "docs").expect("docs");
    assert_eq!(
        docs.deps,
        vec![daruda_flow::NodeId::from("build")],
        "the drawn edge reached the file:\n{text}"
    );

    // And gained exactly one line. The reload rebuilt the canvas from the
    // file, so the new edge is there because the file says so — a drawn edge
    // left behind beside it would make this two.
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.drawn_edges_for_test(cx)),
        before + 1,
        "one new line, not the drawn one plus the reloaded one"
    );
}

/// The vendored port plugin will add a node nobody asked for: drop a wire on
/// empty space and click the dangling endpoint, and it builds a blank node plus
/// an edge into it. Nothing in the flow file names that node, so the canvas
/// stops being the file's picture — and the reconcile that only looked at
/// *edges it could name* skipped straight past it.
///
/// Both come off, and the file is never touched: a connection joins two cards
/// that already exist, which is a decision this asserts rather than assumes.
#[gpui::test]
async fn a_blank_node_the_file_never_named_is_taken_off_the_canvas(cx: &mut TestAppContext) {
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
    let before_text = std::fs::read_to_string(&flow_path).expect("on disk");
    let (nodes, edges) = view.read_with(&vcx, |v, cx| {
        (v.drawn_nodes_for_test(cx), v.drawn_edges_for_test(cx))
    });

    view.update(&mut vcx, |v, cx| {
        v.strand_a_blank_node_for_test(&"design".into(), cx)
    });
    vcx.run_until_parked();

    let (after_nodes, after_edges) = view.read_with(&vcx, |v, cx| {
        (v.drawn_nodes_for_test(cx), v.drawn_edges_for_test(cx))
    });
    assert_eq!(after_nodes, nodes, "the blank node is gone");
    assert_eq!(
        after_edges, edges,
        "and so is the wire drawn to it — removing the node cascades"
    );
    assert_eq!(
        std::fs::read_to_string(&flow_path).expect("on disk"),
        before_text,
        "and nothing was written about a node the file never had"
    );
}

/// The mirror of drawing one: a line taken off the canvas has to reach the file
/// as a removed `deps` entry.
///
/// Goes through `drop_selected_edges` — the funnel both affordances share, the
/// Delete key and the context-menu row — rather than emitting `Disconnect`, so
/// what is covered is the reconcile noticing the picture stopped drawing a
/// dependency the file still declares.
#[gpui::test]
async fn a_line_taken_off_the_canvas_is_removed_from_the_file(cx: &mut TestAppContext) {
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

    view.update(&mut vcx, |v, cx| v.select_every_edge_for_test(cx));
    vcx.run_until_parked();
    assert!(
        view.read_with(&vcx, |v, cx| v.has_selected_edge(cx)),
        "the line is selected, which is what both affordances act on"
    );

    view.update(&mut vcx, |v, cx| v.drop_selected_edges(cx));
    vcx.run_until_parked();

    let text = std::fs::read_to_string(&flow_path).expect("on disk");
    let file = daruda_flow::parse::parse_flow_file(&text).expect("still a flow");
    let build = file.nodes.iter().find(|n| n.id == "build").expect("build");
    assert!(
        build.deps.is_empty(),
        "`build` no longer waits for `design`:\n{text}"
    );
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.drawn_edges_for_test(cx)),
        0,
        "and the picture agrees — the reload rebuilt it from the file"
    );
}

/// Drawing and removing are the same invariant read two ways, so one must not
/// undo the other: a line drawn, written, and then removed leaves the file as
/// it started, not with a stray dep or a stray line.
#[gpui::test]
async fn drawing_a_line_and_taking_it_away_returns_the_file(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, A_SPARE_NODE);
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

    view.update(&mut vcx, |v, cx| {
        v.draw_edge_for_test(&"build".into(), &"docs".into(), cx)
    });
    vcx.run_until_parked();
    view.update(&mut vcx, |v, cx| {
        v.select_edge_for_test(&"build".into(), &"docs".into(), cx)
    });
    vcx.run_until_parked();
    view.update(&mut vcx, |v, cx| v.drop_selected_edges(cx));
    vcx.run_until_parked();

    let after = std::fs::read_to_string(&flow_path).expect("on disk");
    let file = daruda_flow::parse::parse_flow_file(&after).expect("still a flow");
    let docs = file.nodes.iter().find(|n| n.id == "docs").expect("docs");
    assert!(docs.deps.is_empty(), "the dep it gained is gone:\n{after}");
    let build = file.nodes.iter().find(|n| n.id == "build").expect("build");
    assert_eq!(
        build.deps,
        vec![daruda_flow::NodeId::from("design")],
        "and the dep it always had is untouched:\n{after}"
    );
    let _ = before;
}

/// ▶ and ✓ are off while the inspector holds unsaved edits.
///
/// A run reads the file, so pressing it then would run the version on disk and
/// say nothing about the one on screen. `Pane::is_dirty` deliberately answers
/// `false` for a graph — it is a view of a file, not a buffer over it — so this
/// button asks its own narrower question, and that is what is asserted here.
///
/// The flow declares a profile so an *enabled* press is observable without
/// starting anything: it would stop at the profile question. Nothing spawns
/// either way, and the picker is the discriminator.
#[gpui::test]
async fn the_act_buttons_are_off_while_the_inspector_has_unsaved_edits(cx: &mut TestAppContext) {
    use crate::workspace::main_area::flow_graph_pane::{
        TOOLBAR_CHECK_SELECTOR, TOOLBAR_RUN_SELECTOR,
    };

    const PROFILED: &str = "\
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
";
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, PROFILED);
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
        v.select_node_for_test(&"design".into(), window, cx)
    });
    vcx.run_until_parked();

    // Clean first, or the assertion below would hold for a button that is
    // always off.
    assert!(
        !view.read_with(&vcx, |v, cx| v.has_unsaved_form(cx)),
        "the form was dirty before anything was typed"
    );
    let press = |vcx: &mut gpui::VisualTestContext, selector: &'static str| {
        let at = vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("the toolbar paints {selector}"));
        vcx.simulate_click(at.center(), gpui::Modifiers::none());
        vcx.run_until_parked();
    };
    // Both are live while it is clean, or the assertions below would hold for
    // buttons that never work at all.
    for selector in [TOOLBAR_RUN_SELECTOR, TOOLBAR_CHECK_SELECTOR] {
        press(&mut vcx, selector);
        assert!(
            ws.read_with(&vcx, |ws, _| ws.flow_picker.is_open()),
            "{selector} did nothing on a clean form, so this fixture proves nothing"
        );
        ws.update(&mut vcx, |ws, cx| ws.close_flow_picker(cx));
        vcx.run_until_parked();
    }

    // Now type into it and press again.
    let output = view
        .read_with(&vcx, |v, cx| {
            v.form().expect("a form").body_states(cx).output.clone()
        })
        .clone();
    output.update_in(&mut vcx, |state, window, cx| {
        state.set_value("spec.md".to_string(), window, cx)
    });
    vcx.run_until_parked();
    assert!(
        view.read_with(&vcx, |v, cx| v.has_unsaved_form(cx)),
        "an edited box is dirty"
    );

    press(&mut vcx, TOOLBAR_RUN_SELECTOR);
    assert!(
        !ws.read_with(&vcx, |ws, _| ws.flow_picker.is_open()),
        "the run button was pressable with unsaved edits in the inspector"
    );
    // ✓ reads the file too, and would go further than ▶ — it would call the
    // version on disk valid while the screen shows something else.
    press(&mut vcx, TOOLBAR_CHECK_SELECTOR);
    assert!(
        !ws.read_with(&vcx, |ws, _| ws.flow_picker.is_open()),
        "the check button was pressable with unsaved edits in the inspector"
    );
}
