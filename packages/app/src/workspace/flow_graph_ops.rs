//! The Flows page's graph: opening it, the bridge from its gestures to the
//! writes in `flow_node_ops.rs`, reading its file again, and the one thing a
//! run has to say to it — what colour each card is.
//!
//! The graph is the page's detail: found by its [`FlowDetailId`], run in the
//! lane it was opened for — never simply the active one.

use std::path::Path;
#[cfg(feature = "screenshot")]
use std::path::PathBuf;

#[cfg(feature = "screenshot")]
use daruda_flow::NodeId;
use daruda_flow::event::FlowEvent;
use daruda_store::project::LaneRef;
use gpui::{AppContext as _, Context, Entity, Focusable as _, Window};

use super::Workspace;
use super::command::flow_picker::FlowPurpose;
use super::pages::Page;
use super::pages::flows::detail::{FlowDetail, FlowDetailBody, FlowDetailId};
use super::pages::flows::graph::{FlowGraphEvent, FlowGraphView};
use crate::surface::strings as s;

impl Workspace {
    pub(in crate::workspace) fn on_show_flow_graph(
        &mut self,
        _: &super::ShowFlowGraph,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_flow_picker(self.active, FlowPurpose::Graph, cx);
    }

    /// Read the open graph's file again.
    ///
    /// A watcher does this on its own (`sync/flows.rs`), but the key stays: a
    /// file system that reports nothing — a network volume, an editor that
    /// writes without an event — leaves the watcher silent, and this is the way
    /// out of that. It is also what the tests drive.
    pub(in crate::workspace) fn on_reload_flow_graph(
        &mut self,
        _: &super::ReloadFlowGraph,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((id, _, _)) = self.open_graph() {
            self.reload_flow_graph(id, window, cx);
        }
    }

    /// Read graph `id`'s file again. Does nothing when the bytes are the ones
    /// it already has.
    pub(in crate::workspace) fn reload_flow_graph(
        &mut self,
        id: FlowDetailId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((lane, view)) = self.flow_graph(id) else {
            return;
        };
        view.update(cx, |view, cx| view.reload(window, cx));
        self.recolour_flow_graph(lane, &view, cx);
    }

    /// Put the run's colours back on a graph that was just drawn again.
    ///
    /// A reload draws the flow, not the run of it: every card comes back
    /// pending. The run's own state is here, so put it back — otherwise editing
    /// a field mid-run greys out everything that already passed, until the
    /// next event happens to repaint it.
    fn recolour_flow_graph(
        &mut self,
        lane: LaneRef,
        view: &Entity<FlowGraphView>,
        cx: &mut Context<Self>,
    ) {
        let path = view.read(cx).path().to_path_buf();
        if let Some(colouring) = self.flows.runs.colouring_of(lane, &path) {
            view.update(cx, |view, cx| view.set_run_states(&colouring, cx));
        }
    }

    /// Run the flow graph `id` draws, as far as `until` and reusing whatever
    /// it has pinned.
    ///
    /// The pins are resolved here rather than at the button: this is the last
    /// moment before the run, and the newest run directory — which is where a
    /// reused output comes from — is what the guard is about to lock.
    pub(in crate::workspace) fn run_flow_from_graph(
        &mut self,
        id: FlowDetailId,
        until: Option<daruda_flow::NodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((lane, view)) = self.flow_graph(id) else {
            return;
        };
        let path = view.read(cx).path().to_path_buf();
        let pinned = view.read(cx).pinned_nodes();
        // Pressing Run is the person having acted on whatever the cards were
        // saying about pins that went away.
        view.update(cx, |view, cx| view.forget_unpinned(cx));
        let selection = super::flow_request::FlowSelection { until, pinned };
        // A refusal has already said so on screen — this caller is the graph's
        // ▶, and the person pressing it is looking at that toast.
        let _refused_on_screen =
            self.run_flow_at(lane, &path, FlowPurpose::Run, selection, window, cx);
    }

    /// Pin the graph's selection, or unpin it.
    ///
    /// The colouring goes back on afterwards for the same reason a reload puts
    /// it back: writing the cards again draws the flow, not the run of it.
    pub(in crate::workspace) fn toggle_flow_pins(
        &mut self,
        id: FlowDetailId,
        cx: &mut Context<Self>,
    ) {
        let Some((lane, view)) = self.flow_graph(id) else {
            return;
        };
        view.update(cx, |view, cx| view.toggle_pins(cx));
        self.recolour_flow_graph(lane, &view, cx);
    }

    /// Show `path` as the Flows page's graph, run in `lane`.
    pub(in crate::workspace) fn open_flow_graph(
        &mut self,
        lane: LaneRef,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_flow_graph_then(lane, path, window, cx, |_, _, _, _| {});
    }

    /// Show `path` as the Flows page's graph, run in `lane`, then hand its view
    /// to `then`. The same graph already open is shown as it is; anything else
    /// replaces the page's detail, asking first if that holds edits — so
    /// `then` may run after an answer, or not at all.
    pub(in crate::workspace) fn open_flow_graph_then(
        &mut self,
        lane: LaneRef,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, Entity<FlowGraphView>, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if let Some((id, open_lane, view)) = self.open_graph_of(path, cx)
            && open_lane == lane
        {
            self.show_flow_graph(id, window, cx);
            then(self, view, window, cx);
            return;
        }
        let path = path.to_path_buf();
        self.leave_page_detail_then(Page::Flows, window, cx, move |ws, window, cx| {
            let id = ws.install_flow_graph(lane, &path, window, cx);
            ws.show_flow_graph(id, window, cx);
            if let Some((_, view)) = ws.flow_graph(id) {
                then(ws, view, window, cx);
            }
        });
    }

    /// Bring the Flows page up on graph `id` and give it the keyboard.
    fn show_flow_graph(&mut self, id: FlowDetailId, window: &mut Window, cx: &mut Context<Self>) {
        self.show_page(Page::Flows, cx);
        if let Some((_, view)) = self.flow_graph(id) {
            view.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    /// Make a new graph of `path` the page's detail. The caller has left the
    /// old one. Shared by opening and restoring, which have to agree on how a
    /// graph is built and what its gestures reach.
    pub(in crate::workspace) fn install_flow_graph(
        &mut self,
        lane: LaneRef,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> FlowDetailId {
        let id = FlowDetailId(self.alloc_id());
        let owned = path.to_path_buf();
        let view = cx.new(|cx| FlowGraphView::new(&owned, window, cx));
        // Detached: the subscription's life is the view's, which ends when the
        // detail is dropped. Every handler goes through `id`, so one that
        // arrives after the graph was replaced does nothing.
        cx.subscribe_in(
            &view,
            window,
            move |ws, _, event: &FlowGraphEvent, window, cx| {
                ws.handle_flow_graph_event(id, event, window, cx)
            },
        )
        .detach();
        // A run already under way colours the graph now, not at its next event.
        self.recolour_flow_graph(lane, &view, cx);
        self.mutate_durable(cx, |ws, _| {
            ws.pages.flows.detail = Some(FlowDetail {
                id,
                lane,
                body: FlowDetailBody::Graph(view),
            });
        });
        self.respawn_flow_watcher(cx);
        id
    }

    /// What a press in graph `id` asks for. The inspector's buttons emit; the
    /// writing happens in `flow_node_ops.rs`.
    fn handle_flow_graph_event(
        &mut self,
        id: FlowDetailId,
        event: &FlowGraphEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((lane, view)) = self.flow_graph(id) else {
            return;
        };
        let path = view.read(cx).path().to_path_buf();
        match event {
            FlowGraphEvent::BackToList => self.back_to_flow_list(window, cx),
            FlowGraphEvent::Save => self.save_node_form(&path, view, window, cx),
            FlowGraphEvent::Revert => self.revert_node_form(&path, view, window, cx),
            FlowGraphEvent::Delete => {
                let nodes = view.read(cx).selected_nodes(cx);
                self.confirm_delete_nodes(&path, view, nodes, window, cx)
            }
            // A toast rather than something in the graph: what would have
            // carried it — the node's form — is the thing that was replaced.
            FlowGraphEvent::AddNode => self.add_node(&path, view, window, cx),
            // Straight to the funnel the picker enters one question later:
            // the flow is already named, and the lock — plus the profile
            // question, when the file declares any — still is not.
            FlowGraphEvent::Run => self.run_flow_from_graph(id, None, window, cx),
            FlowGraphEvent::RunUntil => {
                // No node selected is no stopping point, so there is nothing
                // to run — the button is off for exactly this, and falling
                // through would run the whole flow instead.
                if let Some(until) = view.read(cx).selected_node(cx) {
                    self.run_flow_from_graph(id, Some(until), window, cx);
                }
            }
            FlowGraphEvent::TogglePins => self.toggle_flow_pins(id, cx),
            FlowGraphEvent::Validate => {
                // Same as the ▶ above: a refused validate has already said so
                // on screen, to the person who pressed the button.
                let _refused_on_screen = self.run_flow_at(
                    lane,
                    &path,
                    FlowPurpose::Validate,
                    super::flow_request::FlowSelection::default(),
                    window,
                    cx,
                );
            }
            FlowGraphEvent::Connect { out_of, into } => {
                self.connect_nodes(&path, view, out_of, into, window, cx)
            }
            FlowGraphEvent::Disconnect { out_of, into } => {
                self.disconnect_nodes(&path, view, out_of, into, window, cx)
            }
            FlowGraphEvent::TypingDropped => self.report_own_flow_refusal(
                s::flow::edit_dropped_typing(),
                "flow.edit_dropped_typing",
                cx,
            ),
        }
    }

    /// Read the file again in the open graph if it draws one that changed.
    ///
    /// `only` narrows it to one file (our own write); `None` is any file (a
    /// watcher event, which does not say which file changed). Either way a
    /// graph whose bytes did not change does nothing.
    pub(in crate::workspace) fn reload_flow_graphs(
        &mut self,
        only: Option<&Path>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((id, _, view)) = self.open_graph() else {
            return;
        };
        if only.is_none_or(|path| view.read(cx).path() == path) {
            self.reload_flow_graph(id, window, cx);
        }
    }

    /// Follow a renamed file in the graph drawing it.
    ///
    /// Without this the graph survives but its path does not, so the next
    /// repaint reports the old name as unreadable — technically honest and
    /// useless: the person renamed the file, they did not lose it.
    pub(in crate::workspace) fn repoint_flow_graph(
        &mut self,
        from: &Path,
        to: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((_, _, view)) = self.open_graph_of(from, cx) {
            // The saved detail names the path, so it moves with it.
            self.mutate_durable_in(window, cx, |_, window, cx| {
                view.update(cx, |view, cx| view.repoint(to, window, cx));
            });
        }
    }

    /// Fold the event into this run's per-node states and hand the result to
    /// the graph, if it draws that flow for that lane.
    ///
    /// Matched by lane *and* path: two lanes can run the same flow. A resumed
    /// run matches nothing — it cannot say which file it is of
    /// ([`FlowSource`]) — so the graph stays the static picture it was.
    ///
    /// The view is `.cached()`, so what makes a colour change visible is a
    /// notify on the canvas entity inside it — `set_run_states` raises it.
    /// Marking a view dirty marks its ancestors, so that reaches this cached
    /// wrapper (CLAUDE.md render-cost rule 10).
    pub(in crate::workspace) fn colour_flow_graph(
        &mut self,
        lane_ref: LaneRef,
        event: &FlowEvent,
        cx: &mut Context<Self>,
    ) {
        let Some((path, colouring)) = self.flows.runs.colour_after(lane_ref, event) else {
            return;
        };
        if let Some((_, lane, view)) = self.open_graph_of(&path, cx)
            && lane == lane_ref
        {
            view.update(cx, |view, cx| view.set_run_states(&colouring, cx));
        }
    }

    /// Draw the first flow this lane has and colour it from a scripted run —
    /// the `--screenshot-scenario flow-graph-running` entry point.
    ///
    /// The events are the real ones through the real projection, so what is
    /// captured is what a run produces. A scripted sequence rather than a live
    /// run because a capture cannot wait for agents, and because the point is
    /// to get every colour of card on screen at once: a pass, a second attempt,
    /// a gate under repair, and nodes not yet reached.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_first_flow_graph_running_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.first_flow_path_for_shot() else {
            return;
        };
        let lane = self.active;
        self.open_flow_graph(lane, &path, window, cx);
        let run_dir = self.active_lane_root().unwrap_or_default();
        self.seed_flow_run_of_for_test(
            lane,
            run_dir,
            super::flow_request::FlowSource::File(path.clone()),
        );

        let nodes: Vec<NodeId> = self
            .open_graph_of(&path, cx)
            .map(|(_, _, view)| view.read(cx).node_ids_for_shot())
            .unwrap_or_default();
        // Walk the flow in order so the picture reads left to right: the ones
        // behind are done, the one in the middle is working, the rest wait.
        let mut script = Vec::new();
        for (ix, node) in nodes.iter().enumerate() {
            match ix {
                0 => {
                    script.push(FlowEvent::NodeStarted {
                        node: node.clone(),
                        attempt: 1,
                    });
                    script.push(FlowEvent::NodePassed {
                        node: node.clone(),
                        attempt: 1,
                    });
                }
                1 => script.push(FlowEvent::NodeStarted {
                    node: node.clone(),
                    attempt: 2,
                }),
                2 => {
                    script.push(FlowEvent::FixStarted { gate: node.clone() });
                }
                _ => {}
            }
        }
        for event in &script {
            // The two halves the real pump runs for a non-terminal event.
            self.colour_flow_graph(lane, event, cx);
            self.advance_flow_stage(lane, event, cx);
        }
    }

    /// Draw the first flow this lane has and select its first node, so the
    /// inspector is on screen — the `--screenshot-scenario flow-graph-form`
    /// entry point.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_first_flow_graph_selected_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.first_flow_path_for_shot() else {
            return;
        };
        self.open_flow_graph(self.active, &path, window, cx);
        let node = self
            .open_graph_of(&path, cx)
            .map(|(_, _, view)| view)
            .and_then(|view| {
                let first = view.read(cx).node_ids_for_shot().first().cloned();
                first.map(|node| (view, node))
            });
        if let Some((view, node)) = node {
            view.update(cx, |view, cx| view.select_node_for_shot(&node, window, cx));
        }
    }

    /// A flow written for the capture, exercising every card affordance at
    /// once — the `--screenshot-scenario flow-graph-authoring` entry point.
    ///
    /// Written rather than found: what a seeded lane happens to hold cannot be
    /// relied on to break a rule, declare a retry, and take `defaults`, and
    /// these four affordances only became worth looking at together — three of
    /// them share the card header and nothing in code says whether they fit.
    ///
    /// In a temp directory, so a capture leaves nothing in the lane. The pin is
    /// pressed and then invalidated by rewriting what it depends on, which is
    /// the only way to see the reason a pin went away.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_authoring_flow_graph_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // `one` and `two` write the same file, which the graph-dependent rules
        // refuse — and refuse about a flow that still draws. `two` retries, so
        // the policy chip has something to say beside the issue count.
        const BEFORE: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: design
    kind: agent
    output: same.md
    prompt: Read DESIGN.md and write the design.
  - id: build
    kind: agent
    deps: [design]
    output: same.md
    prompt: Implement {{node.design.output}}.
    on_fail:
      retry:
        max_attempts: 2
        hint: The build did not land.
";
        let dir = std::env::temp_dir().join("daruda-shot-authoring");
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let path = dir.join("authoring.yaml");
        if std::fs::write(&path, BEFORE).is_err() {
            return;
        }
        self.open_flow_graph(self.active, &path, window, cx);
        let Some((id, _, view)) = self.open_graph_of(&path, cx) else {
            return;
        };

        // Pin `build`, then rewrite what it reads so the pin goes and says why.
        view.update(cx, |view, cx| {
            view.select_node_for_shot(&"build".into(), window, cx)
        });
        self.toggle_flow_pins(id, cx);
        if std::fs::write(
            &path,
            BEFORE.replace("write the design", "write the design twice"),
        )
        .is_err()
        {
            return;
        }
        view.update(cx, |view, cx| view.reload(window, cx));

        // Land on an agent node that overrides nothing and open the block it
        // does not fill: closed by default is right for the app and wrong for
        // a capture, and the placeholders are the only thing in there worth
        // looking at.
        view.update(cx, |view, cx| {
            view.select_node_for_shot(&"design".into(), window, cx);
            view.toggle_agent_section(cx);
        });
    }

    /// The same graph with its first node's output pinned and the *second* node
    /// selected — the `--screenshot-scenario flow-graph-pinned` entry point.
    ///
    /// The selection moves off the pinned card on purpose: selection wins the
    /// border, so a pinned card that is also selected shows only its badge, and
    /// what needs looking at is whether the indicator stands on its own beside a
    /// pending card. Driven through the real toggle, not a seeded field.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_first_flow_graph_pinned_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_first_flow_graph_selected_for_shot(window, cx);
        let Some(path) = self.first_flow_path_for_shot() else {
            return;
        };
        let Some((id, _, view)) = self.open_graph_of(&path, cx) else {
            return;
        };
        self.toggle_flow_pins(id, cx);
        let second = view.read(cx).node_ids_for_shot().get(1).cloned();
        if let Some(node) = second {
            view.update(cx, |view, cx| view.select_node_for_shot(&node, window, cx));
        }
    }

    /// The same graph with a save the engine refuses, so the inspector's banner
    /// is on screen — the `--screenshot-scenario flow-graph-form-refused` entry
    /// point.
    ///
    /// Naming an agent without a mode is the refusal to drive: it is one field,
    /// it is the engine's rule rather than the form's, and it leaves the file
    /// untouched — a capture must not edit the flow it opened.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_first_flow_graph_refused_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_first_flow_graph_selected_for_shot(window, cx);
        let Some(path) = self.first_flow_path_for_shot() else {
            return;
        };
        let Some((_, _, view)) = self.open_graph_of(&path, cx) else {
            return;
        };
        let agent_id = view
            .read(cx)
            .form()
            .map(|form| form.agent_states().id.clone());
        let Some(agent_id) = agent_id else {
            return;
        };
        agent_id.update(cx, |state, cx| {
            state.set_value("codex".to_string(), window, cx)
        });
        self.save_node_form(&path, view, window, cx);
    }

    /// Draw the first flow this lane has — the `--screenshot-scenario
    /// flow-graph` entry point, which cannot go through the picker.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_first_flow_graph_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = self.first_flow_path_for_shot() {
            self.open_flow_graph(self.active, &path, window, cx);
        }
    }

    /// The flow the graph scenarios open: the first one that **loads**.
    ///
    /// Not simply the first one listed. A file that does not load draws the
    /// error pane, and a capture of that says nothing about the graph — which is
    /// exactly what happened on a machine whose only flow was a stub. Falls back
    /// to the first listed so a workspace where nothing loads still captures the
    /// error it should.
    #[cfg(feature = "screenshot")]
    fn first_flow_path_for_shot(&self) -> Option<PathBuf> {
        let found = self.flow_sources()?.list_flows();
        found
            .iter()
            .find(|flow| {
                std::fs::read_to_string(&flow.path)
                    .is_ok_and(|text| daruda_flow::load(&text, None).is_ok())
            })
            .or_else(|| found.first())
            .map(|flow| flow.path.clone())
    }
}
