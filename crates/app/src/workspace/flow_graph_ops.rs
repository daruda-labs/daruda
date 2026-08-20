//! The flow graph pane's own lifecycle, and the bridge from a run to it.
//!
//! Split from `flow_ops.rs` because a pane's tabs and a run's submission are
//! two domains that only met in one file: nothing here talks to the engine,
//! and nothing in `flow_ops.rs` knows a pane exists.

use std::path::Path;
#[cfg(feature = "screenshot")]
use std::path::PathBuf;

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
        let Some((path, view)) = self
            .active_runtime()
            .panes
            .iter()
            .find(|pane| pane.id == pane_id)
            .and_then(|pane| pane.flow_graph_content())
            .map(|fg| (fg.path.clone(), fg.view.clone()))
        else {
            return;
        };
        view.update(cx, |view, cx| view.reload(window, cx));
        if let Some(colouring) = self.runs.colouring_of(self.active, &path) {
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
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
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

    /// Write the inspector's fields into the flow file.
    ///
    /// The form's values become a `FlowFile` mutation and nothing more — which
    /// text edits that is, and whether they may be written at all, is
    /// [`Self::edit_flow`]'s business (two gates, and the file untouched unless
    /// both pass). Renaming is part of it: a node's mentions in `deps` and in a
    /// gate's `rerun` move with its id, or the flow would stop loading and the
    /// second gate would refuse the whole change.
    pub(in crate::workspace) fn save_node_form(
        &mut self,
        path: &Path,
        view: gpui::Entity<super::main_area::flow_graph_pane::FlowGraphView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((base, node, fields)) = view.read_with(cx, |view, cx| {
            let form = view.form()?;
            Some((view.text()?.to_string(), form.node.clone(), form.fields(cx)))
        }) else {
            return;
        };
        // The name the notes are filtered by — `node` moves into the closure.
        let node_id = node.clone();
        // Cleared before the attempt so what the banner shows is always about the
        // save the person just asked for.
        view.update(cx, |view, cx| view.set_form_refusal(None, Vec::new(), cx));
        let outcome = self.edit_flow(
            path,
            &base,
            move |file| {
                super::main_area::flow_graph_pane::form::apply::node_fields(file, &node, &fields)
            },
            window,
            cx,
        );
        match outcome {
            Ok(()) | Err(super::flow_file_ops::EditRefusal::NothingToDo) => {}
            // Not about the edit: a permission or a disk failure is the same thing
            // wherever it happens, and the toast is where the app says it.
            Err(refusal @ super::flow_file_ops::EditRefusal::Io(_)) => {
                self.report_edit_refusal(&refusal, cx)
            }
            Err(refusal) => {
                let message = refusal.message();
                // The boxes the issues name, for the node this form is about.
                let notes = match &refusal {
                    super::flow_file_ops::EditRefusal::WouldNotLoad { issues, .. } => {
                        super::main_area::flow_graph_pane::form::notes::notes_for(issues, &node_id)
                    }
                    _ => Vec::new(),
                };
                view.update(cx, |view, cx| {
                    view.set_form_refusal(Some(message), notes, cx)
                });
            }
        }
    }

    /// The menu's entry points: resolve the pane to its file and view first, so
    /// the menu carries a pane id and nothing about flows.
    pub(in crate::workspace) fn add_node_to_pane(
        &mut self,
        pane_id: super::main_area::pane_tree::PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((path, view)) = self.flow_graph_of_pane(pane_id) {
            self.add_node(&path, view, window, cx);
        }
    }

    pub(in crate::workspace) fn delete_node_in_pane(
        &mut self,
        pane_id: super::main_area::pane_tree::PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, view)) = self.flow_graph_of_pane(pane_id) else {
            return;
        };
        let nodes = view.read(cx).selected_nodes(cx);
        self.confirm_delete_nodes(&path, view, nodes, window, cx);
    }

    fn flow_graph_of_pane(
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

    /// Add a node to the flow this pane draws, chained after the selected one.
    ///
    /// Selected afterwards, so the inspector is already showing the new node's
    /// fields: what it needs next is a prompt, and the person is looking at the
    /// place to type it. The selection survives the write's reload the same way a
    /// save's does.
    pub(in crate::workspace) fn add_node(
        &mut self,
        path: &Path,
        view: gpui::Entity<super::main_area::flow_graph_pane::FlowGraphView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((base, after)) = view.read_with(cx, |view, cx| {
            Some((view.text()?.to_string(), view.selected_node(cx)))
        }) else {
            return;
        };
        // The name the write chose, so the selection can land on it.
        let added = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let record = added.clone();
        let outcome = self.edit_flow(
            path,
            &base,
            move |file| {
                *record.borrow_mut() =
                    super::main_area::flow_graph_pane::form::apply::new_node(file, after.as_deref())
            },
            window,
            cx,
        );
        match outcome {
            Ok(()) => {
                let id = added.borrow().clone();
                view.update(cx, |view, cx| view.select_node_after_add(&id, window, cx));
            }
            Err(refusal) => self.report_edit_refusal(&refusal, cx),
        }
    }

    /// Take the selected node out, and out of everything that pointed at it.
    ///
    /// Refuses the last node: the differ takes the element's lines out and the
    /// file is left with `nodes:` and nothing under it, which does not parse —
    /// the engine would refuse it, in words about YAML rather than about flows.
    pub(in crate::workspace) fn delete_nodes(
        &mut self,
        path: &Path,
        view: gpui::Entity<super::main_area::flow_graph_pane::FlowGraphView>,
        nodes: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(base) = view.read_with(cx, |view, _| view.text().map(str::to_string)) else {
            return;
        };
        if nodes.is_empty() {
            return;
        }
        // Refused against the *count being taken*, not against one: a marquee can
        // catch every node there is, and the differ would leave `nodes:` with
        // nothing under it — which does not parse.
        if daruda_flow::parse::parse_flow_file(&base)
            .is_ok_and(|file| file.nodes.len() <= nodes.len())
        {
            self.report_own_flow_refusal(s::flow_delete_node_last(), "flow.delete_node_last", cx);
            return;
        }
        let targets = nodes.clone();
        let outcome = self.edit_flow(
            path,
            &base,
            move |file| {
                for node in &targets {
                    super::main_area::flow_graph_pane::form::apply::remove_node(file, node);
                }
            },
            window,
            cx,
        );
        if let Err(refusal) = outcome {
            self.report_edit_refusal(&refusal, cx);
        }
    }

    /// Ask first, then delete. The body says what else changes — a node other
    /// nodes run after does not go quietly.
    pub(in crate::workspace) fn confirm_delete_nodes(
        &mut self,
        path: &Path,
        view: gpui::Entity<super::main_area::flow_graph_pane::FlowGraphView>,
        nodes: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(first) = nodes.first().cloned() else {
            return;
        };
        let dependents = self.dependents_outside(&view, &nodes, cx);
        let body = match nodes.len() {
            1 => s::flow_delete_node_confirm_body(&first, dependents),
            _ => s::flow_delete_nodes_confirm_body(&nodes, dependents),
        };
        let weak = cx.weak_entity();
        let owned_path = path.to_path_buf();
        super::dialog_helpers::open_confirm_dialog(
            s::flow_delete_node_confirm_title(),
            body,
            s::flow_delete_node(),
            gpui_component::button::ButtonVariant::Danger,
            move |_, window, app_cx| {
                if let Some(ws) = weak.upgrade() {
                    let (path, view, nodes) = (owned_path.clone(), view.clone(), nodes.clone());
                    ws.update(app_cx, |ws, cx| {
                        ws.delete_nodes(&path, view, nodes, window, cx);
                    });
                }
            },
            window,
            cx,
        );
    }

    /// How many other nodes name `node` in their `deps` — what the confirm says
    /// will change besides the node itself.
    fn dependents_outside(
        &self,
        view: &gpui::Entity<super::main_area::flow_graph_pane::FlowGraphView>,
        going: &[String],
        cx: &gpui::App,
    ) -> usize {
        view.read(cx)
            .text()
            .and_then(|text| daruda_flow::parse::parse_flow_file(text).ok())
            .map(|file| {
                super::main_area::flow_graph_pane::form::apply::dependents_outside(&file, going)
            })
            .unwrap_or(0)
    }

    /// Throw the form's edits away and read the node again from the file.
    pub(in crate::workspace) fn revert_node_form(
        &mut self,
        _path: &Path,
        view: gpui::Entity<super::main_area::flow_graph_pane::FlowGraphView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        view.update(cx, |view, cx| view.rebuild_form(window, cx));
    }

    /// The pane already drawing `path` in the active lane, if any.
    fn find_flow_graph_pane(&self, path: &Path) -> Option<super::main_area::pane_tree::PaneId> {
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
        let title: gpui::SharedString = to
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| to.display().to_string())
            .into();
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

        let nodes: Vec<String> = self
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
        let (cwd, project, global) = self.flow_sources()?;
        let found = super::flow_paths::list_flows(&cwd, &project, &global);
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
