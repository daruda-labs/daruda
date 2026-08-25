//! The flow graph pane's own lifecycle, and the bridge from a run to it.
//!
//! Opening a graph, building the pane, finding one that already draws a file,
//! reading a file again in every pane that draws it, and following a rename —
//! plus the one thing a run has to say to a pane, which is what colour each
//! card is. Nothing here changes a flow: the writing lives in
//! `flow_node_ops.rs`, and the tabs a pane sits in are the only reason both
//! files exist rather than one.

use std::path::Path;
#[cfg(feature = "screenshot")]
use std::path::PathBuf;

#[cfg(feature = "screenshot")]
use daruda_flow::NodeId;
use daruda_flow::event::FlowEvent;
use gpui::{AppContext as _, Context, Window};

use super::Workspace;
use super::command::flow_picker::FlowPurpose;
use super::main_area::flow_graph_pane::FlowGraphEvent;
use crate::surface::strings as s;

impl Workspace {
    pub(in crate::workspace) fn on_show_flow_graph(
        &mut self,
        _: &super::ShowFlowGraph,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_flow_picker(FlowPurpose::Graph, cx);
    }

    /// Read the focused graph pane's file again.
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
        let focused = self.active_runtime().focused_pane_id;
        self.reload_flow_graph_pane(focused, window, cx);
    }

    /// Read one graph pane's file again. Does nothing when that pane is not a
    /// graph, or when the bytes are the ones it already has.
    pub(in crate::workspace) fn reload_flow_graph_pane(
        &mut self,
        pane_id: super::main_area::pane_tree::PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, view)) = self.flow_graph_of_pane(pane_id) else {
            return;
        };
        view.update(cx, |view, cx| view.reload(window, cx));
        if let Some(colouring) = self.runs.colouring_of(self.active, &path) {
            view.update(cx, |view, cx| view.set_run_states(&colouring, cx));
        }
    }

    /// Run the flow this pane draws, as far as `until` and reusing whatever it
    /// has pinned.
    ///
    /// The pins are resolved here rather than at the button: this is the last
    /// moment before the run, and the newest run directory — which is where a
    /// reused output comes from — is what the guard is about to lock.
    pub(in crate::workspace) fn run_flow_from_graph(
        &mut self,
        path: &Path,
        until: Option<daruda_flow::NodeId>,
        view: gpui::Entity<super::main_area::flow_graph_pane::FlowGraphView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pinned = view.read(cx).pinned_nodes();
        // Pressing Run is the person having acted on whatever the cards were
        // saying about pins that went away.
        view.update(cx, |view, cx| view.forget_unpinned(cx));
        let selection = super::flow_request::FlowSelection { until, pinned };
        self.run_flow_at(path, FlowPurpose::Run, selection, window, cx);
    }

    /// Pin the graph pane's selection, or unpin it.
    ///
    /// The colouring goes back on afterwards for the same reason a reload puts
    /// it back: writing the cards again draws the flow, not the run of it, and
    /// the run's state lives here rather than on the view.
    pub(in crate::workspace) fn toggle_flow_pins(
        &mut self,
        path: &Path,
        view: gpui::Entity<super::main_area::flow_graph_pane::FlowGraphView>,
        cx: &mut Context<Self>,
    ) {
        view.update(cx, |view, cx| view.toggle_pins(cx));
        if let Some(colouring) = self.runs.colouring_of(self.active, path) {
            view.update(cx, |view, cx| view.set_run_states(&colouring, cx));
        }
    }

    /// Open `path` as a graph in a new tab. A dedupe first: a flow already
    /// on screen is activated rather than drawn twice, the same way the file
    /// viewer activates an open file's tab.
    pub(in crate::workspace) fn open_flow_graph(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(pane_id) = self.find_flow_graph_pane(path) {
            // By what the tab *contains*, not by what it last focused: a graph
            // pane in a split whose sibling has focus is still in that tab, and
            // matching on `last_focused_pane` made the row click do nothing.
            if let Some(ix) = self.tab_index_for_pane(pane_id) {
                self.activate_tab(ix, window, cx);
            }
            self.focus_pane(pane_id, window, cx);
            return;
        }
        let pane = self.create_flow_graph_pane(path, window, cx);
        let pane_id = pane.id;
        let tab_id = self.alloc_id();
        self.active_runtime_mut().panes.push(pane);
        self.active_runtime_mut()
            .tabs
            .push(super::main_area::pane::TabEntry {
                id: tab_id,
                layout: super::main_area::pane_tree::PaneLayout::Pane(pane_id),
                last_focused_pane: pane_id,
                user_label: None,
            });
        let cur_tab = self.active_runtime().active_tab_index;
        self.active_runtime_mut().tab_history.push(cur_tab);
        let last_tab = self.active_runtime().tabs.len() - 1;
        self.active_runtime_mut().active_tab_index = last_tab;
        self.set_focused_pane(pane_id, window, cx);
        self.bump_activity(pane_id);
        self.focus_pane(pane_id, window, cx);
        self.resize_all_tabs(window, cx);
        cx.notify();
    }

    /// Construct a flow-graph `Pane` (no tab side-effects). Shared by the open
    /// path above and cold restore (`rebuild_layout`), which have to agree on
    /// the title and the path the pane is keyed by.
    pub(in crate::workspace) fn create_flow_graph_pane(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> super::main_area::pane::Pane {
        let title = super::flow_paths::flow_label(path);
        let owned = path.to_path_buf();
        let view =
            cx.new(|cx| super::main_area::flow_graph_pane::FlowGraphView::new(&owned, window, cx));
        // The inspector's buttons emit; the writing happens here. Detached
        // because the subscription's life is the view's: when the pane closes the
        // view is released and gpui drops its subscribers with it.
        let for_path = owned.clone();
        cx.subscribe_in(
            &view,
            window,
            move |workspace, view, event: &FlowGraphEvent, window, cx| match event {
                FlowGraphEvent::Save => {
                    workspace.save_node_form(&for_path, view.clone(), window, cx)
                }
                FlowGraphEvent::Revert => {
                    workspace.revert_node_form(&for_path, view.clone(), window, cx)
                }
                FlowGraphEvent::Delete => {
                    let nodes = view.read(cx).selected_nodes(cx);
                    workspace.confirm_delete_nodes(&for_path, view.clone(), nodes, window, cx)
                }
                // A toast rather than something in the pane: what would have
                // carried it — the node's form — is the thing that was replaced.
                FlowGraphEvent::AddNode => workspace.add_node(&for_path, view.clone(), window, cx),
                // Straight to the funnel the picker enters one question later:
                // the flow is already named, and the lock — plus the profile
                // question, when the file declares any — still is not.
                FlowGraphEvent::Run => {
                    workspace.run_flow_from_graph(&for_path, None, view.clone(), window, cx)
                }
                FlowGraphEvent::RunUntil => {
                    let until = view.read(cx).selected_node(cx);
                    // No node selected is no stopping point, so there is
                    // nothing to run — the button is off for exactly this, and
                    // falling through would run the whole flow instead.
                    if let Some(until) = until {
                        workspace.run_flow_from_graph(
                            &for_path,
                            Some(until),
                            view.clone(),
                            window,
                            cx,
                        );
                    }
                }
                FlowGraphEvent::TogglePins => {
                    workspace.toggle_flow_pins(&for_path, view.clone(), cx)
                }
                FlowGraphEvent::Validate => workspace.run_flow_at(
                    &for_path,
                    FlowPurpose::Validate,
                    super::flow_request::FlowSelection::default(),
                    window,
                    cx,
                ),
                FlowGraphEvent::Connect { out_of, into } => {
                    workspace.connect_nodes(&for_path, view.clone(), out_of, into, window, cx)
                }
                FlowGraphEvent::Disconnect { out_of, into } => {
                    workspace.disconnect_nodes(&for_path, view.clone(), out_of, into, window, cx)
                }
                FlowGraphEvent::TypingDropped => workspace.report_own_flow_refusal(
                    s::flow_edit_dropped_typing(),
                    "flow.edit_dropped_typing",
                    cx,
                ),
            },
        )
        .detach();
        super::main_area::pane::Pane {
            id: self.alloc_id(),
            content: super::main_area::pane::PaneContent::FlowGraph(
                super::main_area::pane::FlowGraphContent {
                    view,
                    path: owned,
                    cached_title: title.into(),
                },
            ),
        }
    }

    /// The file and view of the graph pane `pane_id` names, if it is one.
    ///
    /// The pane list is the only place an id resolves to what it draws, and
    /// both a reload and every node write in `flow_node_ops.rs` start from an
    /// id and need the pair.
    pub(super) fn flow_graph_of_pane(
        &self,
        pane_id: super::main_area::pane_tree::PaneId,
    ) -> Option<(
        std::path::PathBuf,
        gpui::Entity<super::main_area::flow_graph_pane::FlowGraphView>,
    )> {
        self.active_runtime()
            .panes
            .iter()
            .find(|pane| pane.id == pane_id)
            .and_then(|pane| pane.flow_graph_content())
            .map(|fg| (fg.path.clone(), fg.view.clone()))
    }

    /// The pane already drawing `path` in the active lane, if any.
    pub(super) fn find_flow_graph_pane(
        &self,
        path: &Path,
    ) -> Option<super::main_area::pane_tree::PaneId> {
        self.active_runtime()
            .panes
            .iter()
            .find_map(|pane| match pane.flow_graph_content() {
                Some(fg) if fg.path == path => Some(pane.id),
                _ => None,
            })
    }

    /// Read the file again in every graph pane drawing it — every lane's, not
    /// just the active one's, because the project's and the person's own flow
    /// directories are shared between them.
    ///
    /// `only` narrows it to one file (our own write); `None` is every graph
    /// (a watcher event, which does not say which file changed). Either way a
    /// pane whose bytes did not change does nothing.
    pub(in crate::workspace) fn reload_flow_graphs(
        &mut self,
        only: Option<&Path>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets: Vec<_> = self
            .main_area
            .runtimes
            .iter()
            .flat_map(|(lane_ref, runtime)| runtime.panes.iter().map(move |pane| (*lane_ref, pane)))
            .filter_map(|(lane_ref, pane)| Some((lane_ref, pane.flow_graph_content()?)))
            .filter(|(_, fg)| only.is_none_or(|path| fg.path == path))
            .map(|(lane_ref, fg)| (lane_ref, fg.path.clone(), fg.view.clone()))
            .collect();
        for (lane_ref, path, view) in targets {
            view.update(cx, |view, cx| view.reload(window, cx));
            // A reload draws the flow, not the run of it: every card comes back
            // pending. The run's own state is here, so put it back — otherwise
            // editing a field mid-run greys out everything that already passed,
            // until the next event happens to repaint it.
            if let Some(colouring) = self.runs.colouring_of(lane_ref, &path) {
                view.update(cx, |view, cx| view.set_run_states(&colouring, cx));
            }
        }
    }

    /// Follow a renamed file in every pane drawing it.
    ///
    /// Without this the tab survives but its path does not, so the next repaint
    /// reports the old name as unreadable — technically honest and useless: the
    /// person renamed the file, they did not lose it.
    pub(in crate::workspace) fn repoint_flow_graph_panes(
        &mut self,
        from: &Path,
        to: &Path,
        cx: &mut Context<Self>,
    ) {
        let title: gpui::SharedString = super::flow_paths::flow_label(to).into();
        // The view holds the path too, and the two have to move together.
        let mut repointed = Vec::new();
        for runtime in self.main_area.runtimes.values_mut() {
            for pane in runtime.panes.iter_mut() {
                if let Some(fg) = pane.flow_graph_content_mut()
                    && fg.path == from
                {
                    fg.path = to.to_path_buf();
                    fg.cached_title = title.clone();
                    repointed.push(fg.view.clone());
                }
            }
        }
        for view in repointed {
            view.update(cx, |view, cx| view.repoint(to, cx));
        }
        cx.notify();
    }

    /// Fold the event into this run's per-node states and hand the result to
    /// the graph pane drawing that flow, if one is open.
    ///
    /// The pane is matched by lane *and* path: two lanes can hold the same
    /// flow, and one lane can hold graphs of several. A resumed run matches
    /// nothing — it cannot say which file it is of ([`FlowSource`]) — so its
    /// pane stays the static picture it was.
    ///
    /// The view is `.cached()`, so what makes a colour change visible is a
    /// notify on the canvas entity inside it — `set_run_states` raises it, and
    /// `dispatch_command` raises one too. Marking a view dirty marks its
    /// ancestors, so that reaches this cached wrapper; a bare `Entity::update`
    /// with no notify anywhere would leave the old paint on screen (CLAUDE.md
    /// render-cost rule 10).
    pub(in crate::workspace) fn colour_flow_graph(
        &mut self,
        lane_ref: daruda_store::project::LaneRef,
        event: &FlowEvent,
        cx: &mut Context<Self>,
    ) {
        let Some((path, colouring)) = self.runs.colour_after(lane_ref, event) else {
            return;
        };
        let Some(view) = self
            .main_area
            .runtimes
            .get(&lane_ref)
            .and_then(|runtime| {
                runtime
                    .panes
                    .iter()
                    .find_map(|pane| pane.flow_graph_content().filter(|fg| fg.path == path))
            })
            .map(|fg| fg.view.clone())
        else {
            return;
        };
        view.update(cx, |view, cx| view.set_run_states(&colouring, cx));
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
        self.open_flow_graph(&path, window, cx);
        let lane = self.active;
        let run_dir = self.active_lane_root().unwrap_or_default();
        self.seed_flow_run_of_for_test(
            lane,
            run_dir,
            super::flow_request::FlowSource::File(path.clone()),
        );

        let nodes: Vec<NodeId> = self
            .main_area
            .runtimes
            .get(&lane)
            .and_then(|runtime| {
                runtime
                    .panes
                    .iter()
                    .find_map(|pane| pane.flow_graph_content().filter(|fg| fg.path == path))
            })
            .map(|fg| fg.view.read(cx).node_ids_for_shot())
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
        self.open_flow_graph(&path, window, cx);
        let node = self
            .active_runtime()
            .panes
            .iter()
            .find_map(|pane| pane.flow_graph_content().filter(|fg| fg.path == path))
            .map(|fg| fg.view.clone())
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
        self.open_flow_graph(&path, window, cx);
        let Some(view) = self
            .active_runtime()
            .panes
            .iter()
            .find_map(|pane| pane.flow_graph_content().filter(|fg| fg.path == path))
            .map(|fg| fg.view.clone())
        else {
            return;
        };

        // Pin `build`, then rewrite what it reads so the pin goes and says why.
        view.update(cx, |view, cx| {
            view.select_node_for_shot(&"build".into(), window, cx)
        });
        self.toggle_flow_pins(&path, view.clone(), cx);
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
        let Some(view) = self
            .active_runtime()
            .panes
            .iter()
            .find_map(|pane| pane.flow_graph_content().filter(|fg| fg.path == path))
            .map(|fg| fg.view.clone())
        else {
            return;
        };
        self.toggle_flow_pins(&path, view.clone(), cx);
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
        let Some((_, view)) = self
            .active_runtime()
            .panes
            .iter()
            .find(|pane| pane.flow_graph_content().is_some_and(|fg| fg.path == path))
            .and_then(|pane| pane.flow_graph_content())
            .map(|fg| (fg.path.clone(), fg.view.clone()))
        else {
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
            self.open_flow_graph(&path, window, cx);
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
