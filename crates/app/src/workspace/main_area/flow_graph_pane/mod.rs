//! The flow graph pane — a flow's shape, and the run driving it.
//!
//! Entity-backed rather than a plain struct: a run reports node by node,
//! and a plain-struct pane renders inline under `Workspace::render`, where
//! one `cx.notify()` dirties the whole window. Holding the canvas in its
//! own entity and caching it keeps a run's repaints inside this subtree.

mod build;
mod click;
mod commands;
mod connect;
mod delete_key;
mod disconnect;
pub(in crate::workspace) mod form;
mod form_bridge;
mod frame;
pub(in crate::workspace) mod model;
mod node_ids;
mod overlay;
pub(in crate::workspace) mod pins;
mod policy;
mod port_drag;
mod render;
mod renderer;
mod run_states;
mod selection;

pub(in crate::workspace) use pins::PinSet;
/// The toolbar's test selectors keep their old path: a test names this module,
/// not the file a thing happens to live in.
#[cfg(test)]
pub(in crate::workspace) use render::toolbar::{
    TOOLBAR_CHECK_SELECTOR, TOOLBAR_PIN_SELECTOR, TOOLBAR_RUN_SELECTOR, TOOLBAR_RUN_UNTIL_SELECTOR,
};
/// The card-draw counter, for the repaint measurement in `workspace/tests`.
/// Re-exported rather than opening the module: nothing else in there is any
/// caller's business.
#[cfg(test)]
pub(in crate::workspace) use renderer::CARDS_DRAWN;
pub(in crate::workspace) use selection::Selection;

#[cfg(test)]
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui::{App, Context, Entity, EventEmitter, FocusHandle, Focusable, Window};

use daruda_flow::NodeId;

use self::build::{build_graph_state, read_flow};
use self::commands::StampCards;
use self::model::{FlowGraphModel, GraphNodeKind, NodeRunState};
use self::node_ids::NodeIds;
use self::renderer::card_for;

use crate::surface::strings as s;
use crate::ui::flow_canvas::{CanvasNodeId, FlowCanvas};

/// Why a flow could not be drawn. Three paths with three wordings, so they
/// stay three variants rather than one collapsed message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) enum FlowGraphError {
    /// The file is gone or unreadable — an app-layer failure, before the
    /// engine sees any text.
    Read { path: PathBuf, message: String },
    /// Not YAML, or not this schema.
    Parse { detail: String },
    /// Parsed, but the flow cannot run as written.
    Validate { issues: Vec<String> },
}

/// A flow that draws, or the reason it does not.
enum FlowGraphState {
    Graph {
        canvas: Entity<FlowCanvas>,
        /// The model the cards were built from, kept so a run's states can be
        /// stamped back onto them: a card is `(node, state)` and the node half
        /// is not recoverable from the canvas without re-reading the file,
        /// which would let a mid-run edit change what a run is colouring.
        model: FlowGraphModel,
        /// Flow node id → the canvas node holding its card.
        ids: NodeIds,
    },
    Unreadable(FlowGraphError),
}

/// What the inspector asks the workspace to do. The view emits; the workspace
/// writes — a view that touched the file would be a second place that knows how.
///
/// Not `Copy` since `Connect` carries two ids. Every handler matches on a
/// reference, so nothing needed changing for that.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum FlowGraphEvent {
    Save,
    Revert,
    /// Delete the selected node. Asks first — the workspace owns the dialog.
    Delete,
    /// A reload replaced what the inspector held, and the person had typed
    /// something into it that was not saved.
    TypingDropped,
    /// Add a node, chained after the selected one when there is one.
    AddNode,
    /// Run this flow, or report what is wrong with it without running.
    /// Which flow is not asked — the pane is of one, and it is the one.
    Run,
    /// Run only as far as the selected node. A second glyph rather than a mode
    /// on [`Self::Run`]: the selection is what the *inspector* follows, so
    /// clicking a node to edit it must not quietly narrow the next run.
    RunUntil,
    /// Reuse the selected nodes' finished output instead of computing it, or
    /// stop doing so. Which nodes is the view's to say — the same selection a
    /// delete reads.
    TogglePins,
    Validate,
    /// A line was drawn between two cards: `into` is to run after `out_of`.
    /// The names say the ports rather than from/to, which is the pair that
    /// gets swapped — see [`connect::dep_from_edge`].
    Connect {
        out_of: NodeId,
        into: NodeId,
    },
    /// A line was taken away: `into` no longer waits for `out_of`.
    Disconnect {
        out_of: NodeId,
        into: NodeId,
    },
}

pub(in crate::workspace) struct FlowGraphView {
    /// The flow file this view is of.
    ///
    /// Held here rather than taken as an argument: the pane wrapper keeps the
    /// same path for its tab title, and two copies of an identity can disagree
    /// — a reload handed the wrong one would quietly draw another file.
    path: PathBuf,
    state: FlowGraphState,
    /// The file's text as this view last read it. Kept so [`Self::reload`] can
    /// tell "the file changed" from "something touched the file": a watcher
    /// reports our own writes back to us, and fsevents reports more than
    /// writes. `None` when the file could not be read at all.
    text: Option<String>,
    /// Nodes whose finished output this pane will reuse rather than run.
    ///
    /// Here and nowhere else: a pin is this machine at this moment, and the
    /// flow file is committed and read by people who never made the run it
    /// points at. Dropped node by node on reload — see [`pins::surviving`].
    pins: PinSet,
    /// The selected node's fields, when exactly one node is selected. Built in
    /// [`Self::reconcile_selection`] — a handler — rather than in `render`,
    /// which must not create entities.
    form: Option<form::NodeForm>,
    /// Pins this pane has just let go of, and why.
    ///
    /// Kept until the next edit or the next run — an edit replaces the answer
    /// and a run is the person having acted on it. A toast would be gone
    /// before either, and the node it is about is on screen the whole time.
    unpinned: Vec<(NodeId, pins::PinDropped)>,
    /// Set by the canvas plugin that sees a Delete on a selected card, read
    /// on the next canvas notify. Here rather than in the canvas because the
    /// canvas is rebuilt on every reload and the question outlives it.
    delete_request: delete_key::DeleteRequest,
    /// Keeps the canvas observation alive: the canvas is where a click lands, so
    /// its notify is what tells this view the selection moved.
    _canvas_watch: Option<gpui::Subscription>,
    /// Keeps the theme observation alive. The canvas takes its colours when it
    /// is built and has no setter for them, so a theme switch is answered by
    /// building it again.
    _theme_watch: gpui::Subscription,
    focus_handle: FocusHandle,
}

impl EventEmitter<FlowGraphEvent> for FlowGraphView {}

impl Focusable for FlowGraphView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl FlowGraphView {
    /// Read the file and draw it. Every failure lands in the pane rather
    /// than a toast — the pane is where the person is looking.
    pub(in crate::workspace) fn new(
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The same rule [`policy::Reload::Unreadable`] carries, because a first
        // open is a reload against nothing: bytes that were *read* are kept even
        // when they do not load, so reading them again is nothing to do. Losing
        // them here made the first watcher tick on a broken file look like a
        // change and rebuild for no reason.
        let delete_request = delete_key::DeleteRequest::default();
        let (text, state) = match read_flow(path) {
            Ok(text) => match policy::model_from(&text) {
                Ok(model) => (
                    Some(text),
                    build_graph_state(model, &PinSet::default(), &delete_request, window, cx),
                ),
                Err(err) => (Some(text), FlowGraphState::Unreadable(err)),
            },
            Err(err) => (None, FlowGraphState::Unreadable(err)),
        };
        let this = cx.entity().downgrade();
        let theme_watch = window.observe_global::<crate::ui::theme::DarudaTheme>(cx, {
            move |window, cx| {
                // SILENT-OK: the view is gone, so this subscription is being dropped with it — there is no canvas left to re-colour.
                let _ = this.update(cx, |view: &mut Self, cx| view.rebuild_for_theme(window, cx));
            }
        });
        let mut view = Self {
            unpinned: Vec::new(),
            delete_request,
            path: path.to_path_buf(),
            state,
            text,
            pins: PinSet::default(),
            form: None,
            _canvas_watch: None,
            _theme_watch: theme_watch,
            focus_handle: cx.focus_handle(),
        };
        view.watch_canvas(window, cx);
        view
    }

    /// Write the cards again onto the canvas that is already there — which
    /// [`policy::Reload::Restamp`] has decided is still the right one.
    fn restamp(&mut self, model: &FlowGraphModel, cx: &mut Context<Self>) {
        // Read off `self` before the state is borrowed mutably below: the
        // cards need both, and the pins are the smaller half to copy.
        let pins = self.pins.clone();
        let unpinned = self.unpinned.clone();
        let FlowGraphState::Graph {
            canvas,
            model: current,
            ids,
        } = &mut self.state
        else {
            return;
        };
        let cards: Vec<(CanvasNodeId, serde_json::Value)> = model
            .nodes
            .iter()
            .filter_map(|node| {
                let id = ids.canvas(&node.id)?;
                let card = card_for(
                    node,
                    renderer::CardFacts {
                        run: NodeRunState::default(),
                        pinned: pins.contains(&node.id),
                        issues: model.issues_naming(&node.id),
                        unpinned: unpinned
                            .iter()
                            .find(|(id, _)| id == &node.id)
                            .map(|(_, why)| why),
                    },
                );
                Some((id, serde_json::to_value(&card).unwrap_or_default()))
            })
            .collect();
        canvas.update(cx, |canvas, cx| {
            canvas.dispatch_command(StampCards { cards }, cx);
        });
        *current = model.clone();
    }

    /// Build the graph again under the theme that is now installed.
    ///
    /// The canvas is given its colours at build time — `FlowCanvas` has no
    /// setter for them, and a node renderer cannot read a theme — so a switch is
    /// answered by rebuilding. Rare enough to be worth it, and it keeps the
    /// selection the same way a reload does.
    fn rebuild_for_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let FlowGraphState::Graph { model, .. } = &self.state else {
            return;
        };
        let was_selected = self.form.as_ref().map(|form| form.node.clone());
        self.state = build_graph_state(
            model.clone(),
            &self.pins.clone(),
            &self.delete_request,
            window,
            cx,
        );
        self.form = None;
        self.watch_canvas(window, cx);
        if let Some(node) = was_selected {
            self.select_node(&node, window, cx);
        }
        cx.notify();
    }

    /// Observe the canvas so a click on a card reaches this side.
    ///
    /// `Window::observe` rather than `Context::observe` because rebuilding the
    /// form makes `InputState` entities, which need a window. Re-installed after
    /// every reload: a reload builds a new canvas, and the old subscription
    /// watches an entity nobody draws any more.
    /// Forget why pins went away, because a run is starting.
    ///
    /// The other half of the rule an edit's drop follows: an edit replaces the
    /// answer, and pressing Run is the person having acted on it. Restoring a
    /// *finished* run's colours is neither, which is why this is its own call
    /// rather than something the stamping does.
    pub(in crate::workspace) fn forget_unpinned(&mut self, cx: &mut Context<Self>) {
        if self.unpinned.is_empty() {
            return;
        }
        self.unpinned.clear();
        let FlowGraphState::Graph { model, .. } = &self.state else {
            return;
        };
        let model = model.clone();
        self.restamp(&model, cx);
    }

    /// Take what an edit did to the pins: what still holds, and what does not
    /// along with the reason the cards will show.
    fn absorb_surviving(&mut self, surviving: pins::Surviving) {
        self.pins = surviving.kept;
        self.unpinned = surviving.dropped;
    }

    /// What the engine refuses about the file as it stands.
    ///
    /// Cloned rather than borrowed: every caller goes on to build a form,
    /// which needs `&mut self`.
    fn current_issues(&self) -> Vec<daruda_flow::error::ValidationIssue> {
        match &self.state {
            FlowGraphState::Graph { model, .. } => model.issues.clone(),
            FlowGraphState::Unreadable(_) => Vec::new(),
        }
    }

    /// Everything this view owes the canvas after it changed.
    ///
    /// A method rather than a closure body so a test can run it without a
    /// canvas notify to hang it on — the delete below is reported by a plugin
    /// and answered here, and neither half is reachable from a key press in a
    /// headless window.
    fn on_canvas_notified(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reconcile_selection(window, cx);
        self.reconcile_edges(cx);
        // The workspace owns the dialog and the write, so this asks rather
        // than acts — the same event the toolbar and the menu emit.
        if self.delete_request.take() {
            cx.emit(FlowGraphEvent::Delete);
        }
    }

    fn watch_canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let FlowGraphState::Graph { canvas, .. } = &self.state else {
            self._canvas_watch = None;
            return;
        };
        let this = cx.entity().downgrade();
        self._canvas_watch = Some(window.observe(canvas, cx, move |_canvas, window, cx| {
            // SILENT-OK: the view is gone, so this subscription is being dropped with it — nothing left to reconcile, and nobody left to report to.
            let _ = this.update(cx, |view, cx| view.on_canvas_notified(window, cx));
        }));
    }

    /// The bytes this view is drawing, which an edit has to be made against.
    /// `None` when the file could not be read — there is nothing to edit then.
    pub(in crate::workspace) fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Read the file again and rebuild the graph from it.
    ///
    /// **A read that finds the same bytes does nothing.** That one rule is what
    /// makes a watcher safe to point at this: our own writes come back as
    /// events, fsevents reports touches and attribute changes as well as
    /// writes, and a person can press the refresh key while a watcher is
    /// already doing it. None of those become work, and none of them can
    /// become a loop.
    ///
    /// A file that has stopped loading lands in `Unreadable` exactly as it
    /// would on first open — the pane says why rather than keeping a graph of
    /// something that is no longer true.
    pub(in crate::workspace) fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // What was selected has to survive the rebuild: a save writes the file,
        // the watcher reads it back, and an inspector that closed itself on
        // every save would be unusable. What was *typed* is taken for the same
        // reason from the other side — the rebuild is about to replace it.
        let was_selected = self.form.as_ref().map(|form| form.node.clone());
        let typed = self
            .form
            .as_ref()
            .filter(|form| form.is_dirty(cx))
            .map(|form| (form.node.clone(), form.fields(cx)));

        match policy::decide(read_flow(&self.path), self.text.as_deref(), self.drawn()) {
            policy::Reload::Unchanged => return,
            policy::Reload::Restamp { text, model } => {
                self.absorb_surviving(pins::surviving(&self.pins, self.text.as_deref(), &text));
                self.restamp(&model, cx);
                self.text = Some(text);
                self.rebuild_form(window, cx);
            }
            policy::Reload::Rebuild { text, model } => {
                self.absorb_surviving(pins::surviving(&self.pins, self.text.as_deref(), &text));
                self.state =
                    build_graph_state(model, &self.pins.clone(), &self.delete_request, window, cx);
                self.text = Some(text);
                self.form = None;
                self.watch_canvas(window, cx);
                if let Some(node) = was_selected {
                    self.select_node(&node, window, cx);
                }
            }
            policy::Reload::Unreadable { text, error } => {
                // Nothing left to compare a pin against, and the graph it named
                // is gone off the screen with it.
                self.pins.clear();
                self.state = FlowGraphState::Unreadable(error);
                self.text = text;
                self.form = None;
                self.watch_canvas(window, cx);
            }
        }
        self.say_if_typing_was_dropped(typed, cx);
        cx.notify();
    }

    /// What the decision needs to know about what is on screen.
    fn drawn(&self) -> policy::Drawn<'_> {
        match &self.state {
            FlowGraphState::Graph { model, .. } => policy::Drawn::Graph(model),
            FlowGraphState::Unreadable(error) => policy::Drawn::Nothing(error),
        }
    }
}

impl FlowGraphView {
    /// What one press of the pin button would do, given what is selected.
    pub(in crate::workspace) fn pin_action(&self, cx: &App) -> pins::PinAction {
        self.pins.action_for(self.reusable_selection(cx))
    }

    /// Every pinned node, for the run about to be submitted.
    pub(in crate::workspace) fn pinned_nodes(&self) -> Vec<NodeId> {
        self.pins.to_vec()
    }

    /// Pin the selection, or unpin it, and write the cards again so the graph
    /// says which. A run's colours are not put back here — the caller does
    /// that, exactly as it does after a reload, because the run's state lives
    /// on the workspace and not on this view.
    /// Forget pins the run could not honour.
    ///
    /// A pin whose source run has been swept is gone for good, so leaving it
    /// set drew the card as reused on every later press while the node was
    /// quietly paid for again. The one warning that said so is long dismissed
    /// by then; the card is the only thing still on screen.
    pub(in crate::workspace) fn drop_pins(&mut self, nodes: &[NodeId], cx: &mut Context<Self>) {
        let before = self.pins.clone();
        for node in nodes {
            self.pins.remove(node);
        }
        if self.pins == before {
            return;
        }
        // Said on the card for the same reason an edit's drop is: the toast is
        // gone in a moment and the node is on screen the whole time.
        self.unpinned = nodes
            .iter()
            .filter(|node| before.contains(node))
            .map(|node| (node.clone(), pins::PinDropped::SourceGone))
            .collect();
        {}
        let FlowGraphState::Graph { model, .. } = &self.state else {
            return;
        };
        let model = model.clone();
        self.restamp(&model, cx);
        cx.notify();
    }

    pub(in crate::workspace) fn toggle_pins(&mut self, cx: &mut Context<Self>) {
        let nodes = self.reusable_selection(cx);
        if nodes.is_empty() {
            return;
        }
        self.pins.toggle(&nodes);
        let FlowGraphState::Graph { model, .. } = &self.state else {
            return;
        };
        let model = model.clone();
        self.restamp(&model, cx);
        cx.notify();
    }

    /// The selected nodes that have an output to reuse — the agent ones. A gate
    /// writes nothing, so pinning one would name a file that does not exist and
    /// the engine would refuse the whole run over it.
    fn reusable_selection(&self, cx: &App) -> Vec<NodeId> {
        let FlowGraphState::Graph { model, .. } = &self.state else {
            return Vec::new();
        };
        let selected = self.selected_nodes(cx);
        model
            .nodes
            .iter()
            .filter(|node| matches!(node.kind, GraphNodeKind::Agent { .. }))
            .map(|node| node.id.clone())
            .filter(|id| selected.contains(id))
            .collect()
    }
}

impl FlowGraphView {
    /// Follow the file to its new name. The pane wrapper renames its tab; this
    /// is the other half, so the next reload reads the file that now exists.
    pub(in crate::workspace) fn repoint(&mut self, to: &Path, cx: &mut Context<Self>) {
        self.path = to.to_path_buf();
        cx.notify();
    }

    /// Fall to "the file is gone", because it is.
    ///
    /// Nothing re-reads a flow after the first open, so a deleted file would
    /// otherwise keep drawing from the model already in memory — a graph that
    /// looks fine and is not there, and a path that goes on being persisted.
    pub(in crate::workspace) fn report_file_gone(&mut self, cx: &mut Context<Self>) {
        self.state = FlowGraphState::Unreadable(FlowGraphError::Read {
            path: self.path.clone(),
            message: s::flow_file_gone(),
        });
        // The text with it, like every other fall to `Unreadable`: `reload`
        // returns early when the bytes it read are the ones it has, so a file
        // restored exactly as it was would otherwise never redraw.
        self.text = None;
        self.form = None;
        cx.notify();
    }
}

impl FlowGraphView {
    /// The canvas entity, for tests that assert where the graph ended up on
    /// screen. `None` when the flow did not load.
    #[cfg(test)]
    pub(in crate::workspace) fn canvas_for_test(&self) -> Option<&Entity<FlowCanvas>> {
        match &self.state {
            FlowGraphState::Graph { canvas, .. } => Some(canvas),
            FlowGraphState::Unreadable(_) => None,
        }
    }

    /// The flow's node ids in the order the file declares them — what a
    /// scripted capture walks to put one card in each state.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn node_ids_for_shot(&self) -> Vec<NodeId> {
        match &self.state {
            FlowGraphState::Graph { model, .. } => {
                model.nodes.iter().map(|n| n.id.clone()).collect()
            }
            FlowGraphState::Unreadable(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
impl FlowGraphView {
    /// A canvas notify with no key behind it — pan, zoom, run colouring.
    pub(in crate::workspace) fn notified_for_test(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_canvas_notified(window, cx);
    }

    /// Press Delete on the selected cards, as far as the canvas plugin gets:
    /// it records the key and the next notify is what answers it.
    pub(in crate::workspace) fn press_delete_for_test(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_request.ask_for_test();
        self.on_canvas_notified(window, cx);
    }

    /// Each node's card as the canvas actually holds it, keyed by flow node id
    /// and reduced to the two fields a run changes. Reading it back through the
    /// canvas is the point: it proves the stamp reached the graph the renderer
    /// draws from, not just the states the workspace accumulated.
    pub(in crate::workspace) fn cards_for_test(
        &self,
        cx: &App,
    ) -> HashMap<NodeId, (String, String)> {
        let FlowGraphState::Graph { canvas, ids, .. } = &self.state else {
            return HashMap::new();
        };
        let graph = canvas.read(cx).graph();
        ids.iter()
            .filter_map(|(flow_id, node_id)| {
                let node = graph.nodes().get(&node_id)?;
                let card: renderer::CardData =
                    serde_json::from_value(node.data_ref().clone()).ok()?;
                Some((flow_id.clone(), (card.badge, format!("{:?}", card.accent))))
            })
            .collect()
    }

    /// How many rules each card says its node breaks.
    pub(in crate::workspace) fn card_issues_for_test(&self, cx: &App) -> HashMap<NodeId, usize> {
        let FlowGraphState::Graph { canvas, ids, .. } = &self.state else {
            return HashMap::new();
        };
        let graph = canvas.read(cx).graph();
        ids.iter()
            .filter_map(|(flow_id, node_id)| {
                let node = graph.nodes().get(&node_id)?;
                let card: renderer::CardData =
                    serde_json::from_value(node.data_ref().clone()).ok()?;
                Some((flow_id.clone(), card.issues))
            })
            .collect()
    }

    /// Why the pane is showing prose instead of a graph. `None` when it drew.
    pub(in crate::workspace) fn unreadable_for_test(&self) -> Option<&FlowGraphError> {
        match &self.state {
            FlowGraphState::Graph { .. } => None,
            FlowGraphState::Unreadable(err) => Some(err),
        }
    }
}
