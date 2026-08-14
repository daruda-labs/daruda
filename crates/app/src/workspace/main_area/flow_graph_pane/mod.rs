//! The flow graph pane — a flow's shape, and the run driving it.
//!
//! Entity-backed rather than a plain struct: a run reports node by node,
//! and a plain-struct pane renders inline under `Workspace::render`, where
//! one `cx.notify()` dirties the whole window. Holding the canvas in its
//! own entity and caching it keeps a run's repaints inside this subtree.

mod click;
pub(in crate::workspace) mod form;
mod frame;
pub(in crate::workspace) mod model;
mod overlay;
mod policy;
mod renderer;

/// The card-draw counter, for the repaint measurement in `workspace/tests`.
/// Re-exported rather than opening the module: nothing else in there is any
/// caller's business.
#[cfg(test)]
pub(in crate::workspace) use renderer::CARDS_DRAWN;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px,
};

use self::click::NodeClickPlugin;
use self::frame::FrameGraphPlugin;
use self::model::{FlowGraphModel, NodeRunState, RunColouring};
use self::overlay::RerunOverlay;
use self::renderer::{FlowNodeRenderer, NODE_TYPE, card_for, flow_theme};

use crate::surface::strings as s;
use crate::ui::flow_canvas::{
    BackgroundPlugin, Command, CommandContext, FlowCanvas, Graph, GraphPlugin, NodeId,
    SelectionPlugin, ViewportPlugin,
    layout::{LayeredDagLayout, LayoutOptions, LayoutOutput, LayoutStrategy, PositionHint},
};
use crate::ui::theme::palette;

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
        ids: HashMap<String, NodeId>,
    },
    Unreadable(FlowGraphError),
}

/// What the inspector asks the workspace to do. The view emits; the workspace
/// writes — a view that touched the file would be a second place that knows how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// The selected node's fields, when exactly one node is selected. Built in
    /// [`Self::reconcile_selection`] — a handler — rather than in `render`,
    /// which must not create entities.
    form: Option<form::NodeForm>,
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
        let (text, state) = match read_flow(path).and_then(|text| {
            let model = policy::model_from(&text)?;
            Ok((text, model))
        }) {
            Ok((text, model)) => (Some(text), build_graph_state(model, window, cx)),
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
            path: path.to_path_buf(),
            state,
            text,
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
        let FlowGraphState::Graph {
            canvas,
            model: current,
            ids,
        } = &mut self.state
        else {
            return;
        };
        let cards: Vec<(NodeId, serde_json::Value)> = model
            .nodes
            .iter()
            .filter_map(|node| {
                let id = ids.get(&node.id)?;
                let card = card_for(node, NodeRunState::default());
                Some((*id, serde_json::to_value(&card).unwrap_or_default()))
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
        self.state = build_graph_state(model.clone(), window, cx);
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
    fn watch_canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let FlowGraphState::Graph { canvas, .. } = &self.state else {
            self._canvas_watch = None;
            return;
        };
        let this = cx.entity().downgrade();
        self._canvas_watch = Some(window.observe(canvas, cx, move |_canvas, window, cx| {
            // SILENT-OK: the view is gone, so this subscription is being dropped with it — nothing left to reconcile, and nobody left to report to.
            let _ = this.update(cx, |view, cx| view.reconcile_selection(window, cx));
        }));
    }

    /// Follow the canvas's selection: build the form for a newly selected node,
    /// drop it when the selection goes away or grows.
    ///
    /// Does nothing when the selection has not changed — the canvas notifies for
    /// pan, zoom and run colouring too, and rebuilding the form on those would
    /// throw away what the person is typing.
    fn reconcile_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = match self.selection(cx) {
            Selection::One(node) => Some(node),
            Selection::None | Selection::Many(_) => None,
        };
        if wanted == self.form.as_ref().map(|form| form.node.clone()) {
            return;
        }
        let text = self.text.clone();
        self.form = wanted
            .zip(text)
            .and_then(|(node, text)| form::NodeForm::build(&text, &node, window, cx));
        cx.notify();
    }

    /// A flow that loaded but holds no nodes — `nodes: []`, which the engine
    /// accepts. There is nothing to click, so the inspector says to add one.
    fn is_empty_graph(&self) -> bool {
        match &self.state {
            FlowGraphState::Graph { model, .. } => model.nodes.is_empty(),
            FlowGraphState::Unreadable(_) => false,
        }
    }

    /// Which node is selected, by the flow's own id. `None` when nothing or
    /// several are.
    pub(in crate::workspace) fn selected_node(&self, cx: &App) -> Option<String> {
        match self.selection(cx) {
            Selection::One(node) => Some(node),
            Selection::None | Selection::Many(_) => None,
        }
    }

    /// Select a node that has just been written into the file.
    ///
    /// The write's reload has already rebuilt the graph, so the node exists on
    /// the canvas by now; this is the same path a reload uses to put a selection
    /// back, called with a name that was not selected before.
    pub(in crate::workspace) fn select_node_after_add(
        &mut self,
        node: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_node(node, window, cx);
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
                self.restamp(&model, cx);
                self.text = Some(text);
                self.rebuild_form(window, cx);
            }
            policy::Reload::Rebuild { text, model } => {
                self.state = build_graph_state(model, window, cx);
                self.text = Some(text);
                self.form = None;
                self.watch_canvas(window, cx);
                if let Some(node) = was_selected {
                    self.select_node(&node, window, cx);
                }
            }
            policy::Reload::Unreadable { text, error } => {
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

    /// The form for the selected node, when there is one.
    pub(in crate::workspace) fn form(&self) -> Option<&form::NodeForm> {
        self.form.as_ref()
    }

    /// Open or close the inspector's agent-override block.
    pub(in crate::workspace) fn toggle_agent_section(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.form.as_mut() {
            form.toggle_agent_open();
            cx.notify();
        }
    }

    /// Put a refused save's reason on the form — the sentence, and the boxes it
    /// names — or clear both.
    pub(in crate::workspace) fn set_form_refusal(
        &mut self,
        message: Option<String>,
        notes: Vec<form::notes::FieldNote>,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.form.as_mut() {
            form.set_banner(message);
            form.set_notes(notes);
            cx.notify();
        }
    }

    /// Build the form again from what the file says now — the Revert path.
    /// Keeps the same node selected; only the boxes go back.
    pub(in crate::workspace) fn rebuild_form(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self.form.as_ref().map(|form| form.node.clone()) else {
            return;
        };
        let text = self.text.clone();
        self.form = text.and_then(|text| form::NodeForm::build(&text, &node, window, cx));
        cx.notify();
    }

    /// Say so when a reload replaced something the person had typed.
    ///
    /// Not "was the form dirty": the pane that pressed Save is dirty too at the
    /// moment its own write comes back, and saying it on every successful save
    /// would be noise. What is actually lost is the difference between what they
    /// had and what the file now says — zero for the pane that wrote it, and the
    /// typing itself for a pane that did not. A node that is gone after the
    /// reload counts too: nothing came back to compare against.
    ///
    /// Emitted rather than written onto the form, because the form is the thing
    /// being replaced. Adding a node rebuilds it for the new one, and a node
    /// renamed out from under the inspector leaves no form at all — either way a
    /// banner would be gone before it was read.
    fn say_if_typing_was_dropped(
        &mut self,
        typed: Option<(String, form::NodeFields)>,
        cx: &mut Context<Self>,
    ) {
        let Some((node, fields)) = typed else {
            return;
        };
        let rebuilt = self
            .form
            .as_ref()
            .map(|form| (form.node.clone(), form.fields(cx)));
        let rebuilt = rebuilt
            .as_ref()
            .map(|(node, fields)| (node.as_str(), fields));
        if policy::typing_survived(&node, &fields, rebuilt) {
            return;
        }
        cx.emit(FlowGraphEvent::TypingDropped);
    }

    /// Select `node` on the canvas, if the graph still has it, and build its
    /// form. Used after a reload to put back what was selected before.
    fn select_node(&mut self, node: &str, window: &mut Window, cx: &mut Context<Self>) {
        let FlowGraphState::Graph { canvas, ids, .. } = &self.state else {
            return;
        };
        let Some(canvas_id) = ids.get(node).copied() else {
            return;
        };
        canvas.update(cx, |canvas, cx| {
            canvas.dispatch_command(SelectNode { node: canvas_id }, cx);
        });
        self.reconcile_selection(window, cx);
    }

    /// Re-stamp the cards from a run's per-node states.
    ///
    /// Only the cards change — the graph's shape came from the file and a run
    /// cannot alter it, so nothing is re-laid-out and nothing moves under the
    /// person's eyes as a run progresses.
    ///
    /// What makes it visible past this view's `.cached()` wrapper is a notify on
    /// the *canvas*: `execute_command` raises one itself (`plugin.rs`), and the
    /// one below is ours rather than borrowed, so a future stamp that stops
    /// going through a `Command` does not silently lose the repaint.
    /// Colour the cards from `states`, which the run reported about the nodes
    /// in `of_nodes`.
    ///
    /// Nothing is painted when the file no longer has those nodes: a run
    /// executes the flow it resolved at the start, and an id freed by a delete
    /// and taken by a rename would otherwise wear the first node's colour.
    pub(in crate::workspace) fn set_run_states(
        &mut self,
        colouring: &RunColouring,
        cx: &mut Context<Self>,
    ) {
        let FlowGraphState::Graph { canvas, model, ids } = &self.state else {
            return;
        };
        if !colouring.is_about(model.nodes.iter().map(|node| node.id.clone())) {
            return;
        }
        let stamped: Vec<(NodeId, serde_json::Value)> = model
            .nodes
            .iter()
            .filter_map(|node| {
                let id = ids.get(&node.id)?;
                let state = colouring.states.get(&node.id).copied().unwrap_or_default();
                let card = card_for(node, state);
                Some((*id, serde_json::to_value(&card).unwrap_or_default()))
            })
            .collect();
        canvas.update(cx, |canvas, cx| {
            canvas.dispatch_command(StampCards { cards: stamped }, cx);
            cx.notify();
        });
    }
}

impl FlowGraphView {
    /// Fall to "the file is gone", because it is.
    ///
    /// Nothing re-reads a flow after the first open, so a deleted file would
    /// otherwise keep drawing from the model already in memory — a graph that
    /// looks fine and is not there, and a path that goes on being persisted.
    /// Follow the file to its new name. The pane wrapper renames its tab; this
    /// is the other half, so the next reload reads the file that now exists.
    pub(in crate::workspace) fn repoint(&mut self, to: &Path, cx: &mut Context<Self>) {
        self.path = to.to_path_buf();
        cx.notify();
    }

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

/// Write fresh card data onto nodes that already exist.
///
/// A command because the canvas hands out its graph immutably and takes edits
/// this way — there is no `graph_mut`. `undo` is implemented for the trait's
/// sake and never reached: the flow pane installs no `HistoryPlugin`, so
/// nothing is bound to run it (the YAML file is the only undo stack).
struct StampCards {
    cards: Vec<(NodeId, serde_json::Value)>,
}

impl Command for StampCards {
    fn name(&self) -> &'static str {
        "daruda_stamp_flow_cards"
    }

    fn execute(&mut self, ctx: &mut CommandContext) {
        for (id, data) in &self.cards {
            if let Some(node) = ctx.graph.get_node_mut(id) {
                node.set_data(data.clone());
            }
        }
    }

    fn undo(&mut self, _ctx: &mut CommandContext) {}
}

/// Put the selection back on a node after the graph was rebuilt. A command
/// because that is the only public way to write to the graph.
struct SelectNode {
    node: NodeId,
}

impl Command for SelectNode {
    fn name(&self) -> &'static str {
        "daruda_select_flow_node"
    }

    fn execute(&mut self, ctx: &mut CommandContext) {
        ctx.add_selected_node(self.node, false);
    }

    fn undo(&mut self, _ctx: &mut CommandContext) {}
}

/// What the canvas has selected, in the flow's own terms.
///
/// Three variants rather than `Option<String>` plus a count: a marquee can take
/// several cards, and "several" is a state the inspector has to say something
/// about — it cannot show one node's fields and stay honest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum Selection {
    None,
    One(String),
    Many(usize),
}

impl FlowGraphView {
    /// Which flow node the canvas has selected.
    ///
    /// Read from the canvas rather than mirrored here: the canvas's graph is the
    /// one thing that knows what was clicked, and a copy on this side would be a
    /// second answer to the same question.
    pub(in crate::workspace) fn selection(&self, cx: &App) -> Selection {
        let FlowGraphState::Graph { canvas, ids, .. } = &self.state else {
            return Selection::None;
        };
        let selected = canvas.read(cx).graph().selected_node();
        match selected.len() {
            0 => Selection::None,
            1 => {
                let node = selected.iter().next().copied();
                match node.and_then(|node| {
                    ids.iter()
                        .find(|(_, canvas_id)| **canvas_id == node)
                        .map(|(flow_id, _)| flow_id.clone())
                }) {
                    Some(flow_id) => Selection::One(flow_id),
                    // Selected on the canvas but not one of ours — nothing this
                    // side can show fields for.
                    None => Selection::None,
                }
            }
            many => Selection::Many(many),
        }
    }

    /// Select a node so a capture can show the inspector.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn select_node_for_shot(
        &mut self,
        node: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_node(node, window, cx);
    }

    /// Select a node the way a click would, for tests that are about what the
    /// selection *does* rather than about hit-testing (which
    /// `clicking_a_card_selects_it_and_does_not_move_it` covers).
    #[cfg(test)]
    pub(in crate::workspace) fn select_node_for_test(
        &mut self,
        node: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_node(node, window, cx);
    }

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
    pub(in crate::workspace) fn node_ids_for_shot(&self) -> Vec<String> {
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
    /// Each node's card as the canvas actually holds it, keyed by flow node id
    /// and reduced to the two fields a run changes. Reading it back through the
    /// canvas is the point: it proves the stamp reached the graph the renderer
    /// draws from, not just the states the workspace accumulated.
    pub(in crate::workspace) fn cards_for_test(
        &self,
        cx: &App,
    ) -> HashMap<String, (String, String)> {
        let FlowGraphState::Graph { canvas, ids, .. } = &self.state else {
            return HashMap::new();
        };
        let graph = canvas.read(cx).graph();
        ids.iter()
            .filter_map(|(flow_id, node_id)| {
                let node = graph.nodes().get(node_id)?;
                let card: renderer::CardData =
                    serde_json::from_value(node.data_ref().clone()).ok()?;
                Some((flow_id.clone(), (card.badge, format!("{:?}", card.accent))))
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

/// The profile is deliberately `None`. A graph is the file's shape; which
/// named profile a *run* merged under is a question the run answers.
/// Read the file, keeping the text: the graph is derived from it and
/// [`FlowGraphView::reload`] compares against it.
fn read_flow(path: &Path) -> Result<String, FlowGraphError> {
    std::fs::read_to_string(path).map_err(|e| FlowGraphError::Read {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

fn build_graph_state(
    model: FlowGraphModel,
    window: &mut Window,
    cx: &mut Context<FlowGraphView>,
) -> FlowGraphState {
    let (graph, ids) = build_canvas_graph(&model);
    // Resolved here, where the colours are readable: neither the canvas's own
    // theme nor a node renderer can reach `cx` once the canvas is built.
    let tokens = crate::ui::theme::PaneSurfaceTokens::flow_graph(cx);
    let rerun = RerunOverlay::new(ids.clone(), model.rerun.clone());
    let canvas = cx.new(|c| {
        // Deliberately not `default_plugins()`: no node drag (auto-placed
        // nodes carry no position to move) and no history (the YAML file
        // is the single undo stack). Framing is daruda's own (`frame.rs`):
        // a graph laid out one column-pitch apart outgrows a pane, and how
        // far out it is worth zooming depends on what our cards say when
        // they get there.
        FlowCanvas::builder(graph, c, window)
            .theme(flow_theme(&tokens))
            .node_renderer(
                NODE_TYPE,
                FlowNodeRenderer {
                    palette: renderer::CardPalette::of(&tokens),
                },
            )
            .plugin(BackgroundPlugin::new())
            .plugin(ViewportPlugin::new())
            .plugin(GraphPlugin::new())
            .plugin(SelectionPlugin::new())
            .plugin(NodeClickPlugin::new())
            .plugin(FrameGraphPlugin::new())
            .plugin(rerun)
            .build()
    });
    FlowGraphState::Graph { canvas, model, ids }
}

/// Build the canvas graph and lay it out. Coordinates are never persisted —
/// the flow file declares dependencies, not positions — so every open
/// places the nodes again.
fn build_canvas_graph(model: &FlowGraphModel) -> (Graph, HashMap<String, NodeId>) {
    let mut ids: HashMap<String, NodeId> = HashMap::new();
    let mut inputs = HashMap::new();
    let mut outputs = HashMap::new();

    let mut graph = Graph::build(|g| {
        for node in &model.nodes {
            let card = card_for(node, NodeRunState::default());
            let (nid, ins, outs) = g
                .create_node(NODE_TYPE)
                .size(palette::FLOW_GRAPH_NODE_W, palette::FLOW_GRAPH_NODE_H)
                .input()
                .output()
                .data(serde_json::to_value(&card).unwrap_or_default())
                .build_with_ports();
            ids.insert(node.id.clone(), nid);
            inputs.insert(node.id.clone(), ins);
            outputs.insert(node.id.clone(), outs);
        }
        for edge in &model.deps {
            let (Some(from), Some(to)) = (outputs.get(&edge.from), inputs.get(&edge.to)) else {
                continue;
            };
            let (Some(from), Some(to)) = (from.first(), to.first()) else {
                continue;
            };
            g.create_edge().source(*from).target(*to).build();
        }
    });

    if let Ok(LayoutOutput::Delta(delta)) =
        LayeredDagLayout.compute(&graph, &LayoutOptions::default(), None)
    {
        // `NodePositionDelta`'s own fields are crate-private upstream;
        // `PositionHint` is the public way to read the result.
        let placed: Vec<_> = PositionHint::from_delta_to(&delta)
            .positions()
            .iter()
            .map(|(id, p)| (*id, *p))
            .collect();
        for (id, at) in placed {
            if let Some(node) = graph.get_node_mut(&id) {
                node.set_position_with_point(at);
            }
        }
    }
    (graph, ids)
}

/// Buttons over the graph for the two things a person does to it.
///
/// The menu (`pane_menu::FlowGraphMenu`) has the same two and calls the same
/// ops — this is a second way in, not a second implementation. It exists because
/// the menu is a right-click nobody is told about, and adding the first node to
/// a new flow is the moment that matters most.
fn toolbar(has_selection: bool, cx: &mut Context<FlowGraphView>) -> impl IntoElement {
    use crate::ui::{Disableable as _, button_bare};

    div()
        .absolute()
        .top(px(palette::FLOW_TOOLBAR_INSET))
        .right(px(palette::FLOW_TOOLBAR_INSET))
        .flex()
        .flex_row()
        .gap(px(palette::FLOW_TOOLBAR_GAP))
        .child(
            button_bare("flow-toolbar-add")
                .icon(crate::ui::IconName::Plus)
                .tooltip(s::flow_add_node())
                .on_click(cx.listener(|_, _, _, cx| cx.emit(FlowGraphEvent::AddNode))),
        )
        .child(
            // Disabled rather than absent: a button that comes and goes under
            // the pointer is worse than one that says it is not available.
            button_bare("flow-toolbar-delete")
                .icon(crate::ui::IconName::Minus)
                .tooltip(s::flow_delete_node())
                .disabled(!has_selection)
                .on_click(cx.listener(|_, _, _, cx| cx.emit(FlowGraphEvent::Delete))),
        )
}

impl Render for FlowGraphView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = div().size_full().track_focus(&self.focus_handle);
        match &self.state {
            FlowGraphState::Graph { canvas, .. } => {
                // A row rather than an overlay: floating the inspector over the
                // graph would hide the cards it is about.
                //
                // Its width is reserved whether or not anything is selected.
                // Showing it only on selection was tried and is worse: the canvas
                // narrows, the graph re-fits into what is left, and the whole
                // picture shifts under the pointer that just clicked — a
                // shift-click on a second card then misses it. A fixed column
                // costs the width once and never moves the graph.
                let inspector = match self.selection(cx) {
                    Selection::One(_) => match &self.form {
                        Some(form) => form::render(form, cx).into_any_element(),
                        None => form::render_empty(cx).into_any_element(),
                    },
                    Selection::Many(n) => form::render_many(n, cx).into_any_element(),
                    // Nothing to click is not the same as nothing clicked yet.
                    Selection::None if self.is_empty_graph() => {
                        form::render_no_nodes(cx).into_any_element()
                    }
                    Selection::None => form::render_empty(cx).into_any_element(),
                };
                // The toolbar goes inside the canvas half, not the pane: over the
                // pane it would sit on the inspector column instead of the graph.
                let has_selection = matches!(self.selection(cx), Selection::One(_));
                body.child(
                    div()
                        .size_full()
                        .flex()
                        .flex_row()
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .h_full()
                                .child(canvas.clone())
                                .child(toolbar(has_selection, cx)),
                        )
                        .child(inspector),
                )
            }
            FlowGraphState::Unreadable(err) => body
                .flex()
                .flex_col()
                .gap(px(palette::FLOW_GRAPH_CARD_ROW_GAP))
                .p(px(palette::FLOW_GRAPH_CARD_PAD))
                .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
                .text_color(crate::ui::theme::current(cx).text_muted)
                .children(error_lines(err).into_iter().map(|line| div().child(line))),
        }
    }
}

/// One line per thing wrong. A validation failure reports every issue the
/// stage saw, and collapsing them to the first would hide the rest.
fn error_lines(err: &FlowGraphError) -> Vec<String> {
    match err {
        FlowGraphError::Read { path, message } => {
            vec![s::flow_graph_read_failed(
                &path.display().to_string(),
                message,
            )]
        }
        FlowGraphError::Parse { detail } => vec![s::flow_graph_parse_failed(detail)],
        FlowGraphError::Validate { issues } => issues.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_of(yaml: &str) -> FlowGraphModel {
        let loaded = daruda_flow::load(yaml, None).expect("fixture should load");
        FlowGraphModel::from_flow(loaded.flow())
    }

    const ONE_NODE: &str = concat!(
        "version: 1\n",
        "defaults:\n",
        "  agent:\n",
        "    id: claude\n",
        "    mode: bypassPermissions\n",
        "nodes:\n",
        "  - id: hello\n",
        "    kind: agent\n",
        "    output: hello.md\n",
        "    prompt: hi\n",
    );

    /// A single node still has to land somewhere the viewport can see, and
    /// still has to carry the card the renderer reads back.
    #[test]
    fn a_lone_node_is_placed_and_stamped() {
        let (graph, ids) = build_canvas_graph(&model_of(ONE_NODE));
        assert_eq!(ids.len(), 1, "the flow id maps to a canvas node");
        let node = graph.nodes().values().next().expect("one node");
        let (x, y) = node.position();
        println!("POS ({x:?}, {y:?}) size={:?}", node.size_ref());
        assert!(
            f32::from(x).is_finite() && f32::from(y).is_finite(),
            "placed at ({x:?}, {y:?})"
        );
        let card: renderer::CardData =
            serde_json::from_value(node.data_ref().clone()).expect("the node carries a card");
        assert_eq!(card.id, "hello");
    }
}
