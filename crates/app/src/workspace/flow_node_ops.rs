//! Node changes, written into the flow file.
//!
//! Every method here turns one gesture on the graph — a field saved, a card
//! added or taken away, a line drawn or cut — into a `FlowFile` mutation and
//! hands it to [`Workspace::edit_flow`](super::Workspace::edit_flow). That is
//! the whole of the writing done here: which text edits the mutation becomes,
//! and whether they may land at all, is that function's business. Two gates
//! stand in the way and the file is untouched unless both pass — the bytes on
//! disk still say what the change was made against, and the result still loads
//! through `daruda_flow::load`.
//!
//! So a refusal is an ordinary outcome rather than an exception, and where it
//! goes is the only thing that differs between these methods: a save has the
//! inspector's banner and can put the engine's words beside the fields they
//! name, while a menu item or a dragged wire has no such place and takes a
//! toast through `report_edit_refusal`.
//!
//! Split from `flow_graph_ops.rs`: that file owns the pane these gestures
//! arrive from, and writes nothing.

use std::path::Path;

use daruda_flow::NodeId;
use gpui::{Context, Window};

use super::Workspace;
use crate::surface::strings as s;

impl Workspace {
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
        let added = std::rc::Rc::new(std::cell::RefCell::new(NodeId::from("")));
        let record = added.clone();
        let outcome = self.edit_flow(
            path,
            &base,
            move |file| {
                *record.borrow_mut() =
                    super::main_area::flow_graph_pane::form::apply::new_node(file, after.as_ref())
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

    /// Record a line the person drew between two cards.
    ///
    /// The pane has already taken the edge off the canvas, so a refusal needs
    /// no undoing here — the picture is the file's again either way, and the
    /// write's own reload draws the line back when it lands.
    ///
    /// A duplicate reaches `edit_flow` as no edits at all, which is
    /// `NothingToDo` — and `report_edit_refusal` stays quiet on that, because
    /// the wire had already gone red under the cursor.
    pub(in crate::workspace) fn connect_nodes(
        &mut self,
        path: &Path,
        view: gpui::Entity<super::main_area::flow_graph_pane::FlowGraphView>,
        out_of: &NodeId,
        into: &NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(base) = view.read_with(cx, |view, _| view.text().map(str::to_string)) else {
            return;
        };
        let (out_of, into) = (out_of.clone(), into.clone());
        let outcome = self.edit_flow(
            path,
            &base,
            move |file| {
                super::main_area::flow_graph_pane::form::apply::connect(file, &out_of, &into)
            },
            window,
            cx,
        );
        if let Err(refusal) = outcome {
            self.report_edit_refusal(&refusal, cx);
        }
    }

    /// Forget a line the person took away.
    ///
    /// The mirror of [`Self::connect_nodes`], and refused the same way: the
    /// pane has already taken the edge off the canvas, so nothing here has to
    /// put a picture back. A dependency the file no longer has is
    /// `NothingToDo`, which stays quiet.
    pub(in crate::workspace) fn disconnect_nodes(
        &mut self,
        path: &Path,
        view: gpui::Entity<super::main_area::flow_graph_pane::FlowGraphView>,
        out_of: &NodeId,
        into: &NodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(base) = view.read_with(cx, |view, _| view.text().map(str::to_string)) else {
            return;
        };
        let (out_of, into) = (out_of.clone(), into.clone());
        let outcome = self.edit_flow(
            path,
            &base,
            move |file| {
                super::main_area::flow_graph_pane::form::apply::disconnect(file, &out_of, &into)
            },
            window,
            cx,
        );
        if let Err(refusal) = outcome {
            self.report_edit_refusal(&refusal, cx);
        }
    }

    /// Remove whatever line the graph has selected. The menu's half of the
    /// gesture Delete performs; the pane turns it into a `Disconnect`.
    pub(in crate::workspace) fn disconnect_selected_edge_in_pane(
        &mut self,
        pane_id: super::main_area::pane_tree::PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some((_, view)) = self.flow_graph_of_pane(pane_id) else {
            return;
        };
        view.update(cx, |view, cx| view.drop_selected_edges(cx));
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
        nodes: Vec<NodeId>,
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
        nodes: Vec<NodeId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(first) = nodes.first().cloned() else {
            return;
        };
        let dependents = self.dependents_outside(&view, &nodes, cx);
        let body = match nodes.len() {
            1 => s::flow_delete_node_confirm_body(first.as_str(), dependents),
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
        going: &[NodeId],
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
}
