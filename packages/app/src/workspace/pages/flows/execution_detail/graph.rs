//! Own the execution canvas and its observers as one lifecycle.

use super::super::graph::execution::ExecutionGraph;
use super::*;

#[derive(Default)]
pub(in crate::workspace) struct GraphState {
    binding: Option<GraphBinding>,
}

struct GraphBinding {
    graph: ExecutionGraph,
    _watch: gpui::Subscription,
    _theme_watch: gpui::Subscription,
}

impl GraphState {
    pub fn view(&self) -> Option<&ExecutionGraph> {
        self.binding.as_ref().map(|binding| &binding.graph)
    }
    fn view_mut(&mut self) -> Option<&mut ExecutionGraph> {
        self.binding.as_mut().map(|binding| &mut binding.graph)
    }
    fn clear(&mut self) {
        self.binding = None;
    }
}

impl Workspace {
    pub(super) fn build_execution_graph(
        &mut self,
        id: FlowDetailId,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = self.pages.flows.detail.as_mut().filter(|d| d.id == id) else {
            return;
        };
        let FlowDetailBody::Run(run) = &mut detail.body else {
            return;
        };
        let Inspection::Ready(history) = &run.execution.inspection else {
            return;
        };
        if history.flow().is_none() {
            run.execution.graph.clear();
            return;
        }
        if let Some(graph) = run.execution.graph.view()
            && graph.matches_theme(cx)
        {
            graph.refresh(history, cx);
            return;
        }
        let graph = ExecutionGraph::new(history, window, cx);
        if let (Some(graph), Some(selection)) = (&graph, run.execution.inspector.selection()) {
            graph.select(selection.node(), cx);
        }
        let binding = graph.map(|graph| {
            let weak = cx.weak_entity();
            let theme_watch =
                window.observe_global::<crate::ui::theme::DarudaTheme>(cx, move |window, cx| {
                    if let Some(ws) = weak.upgrade() {
                        ws.update(cx, |ws, cx| {
                            ws.refresh_execution_graph_theme(id, window, cx)
                        });
                    }
                });
            let weak = cx.weak_entity();
            let watch = window.observe(&graph.canvas, cx, move |_, window, cx| {
                if let Some(ws) = weak.upgrade() {
                    ws.update(cx, |ws, cx| ws.execution_graph_changed(id, window, cx));
                }
            });
            GraphBinding {
                graph,
                _watch: watch,
                _theme_watch: theme_watch,
            }
        });
        run.execution.graph.binding = binding;
        cx.notify();
    }

    fn refresh_execution_graph_theme(
        &mut self,
        id: FlowDetailId,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .pages
            .flows
            .detail
            .as_ref()
            .filter(|d| d.id == id)
            .and_then(|d| match &d.body {
                FlowDetailBody::Run(run) => run.execution.graph.view(),
                _ => None,
            })
            .is_some_and(|graph| !graph.matches_theme(cx));
        // Chrome theme changes do not necessarily change the pane palette.
        // Preserve the measured canvas and pan when its actual colors are unchanged.
        if changed {
            self.build_execution_graph(id, window, cx);
        }
    }

    fn execution_graph_changed(
        &mut self,
        id: FlowDetailId,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_page() != Some(crate::workspace::pages::Page::Flows) {
            return;
        }
        let Some(run) = self.pages.flows.run_mut(id) else {
            return;
        };
        let Some(graph) = run.execution.graph.view_mut() else {
            return;
        };
        let selected = graph.selected(cx);
        let repaint = graph
            .needs_layout_repaint(cx)
            .then(|| graph.canvas.entity_id());
        let selection_changed = selected.filter(|node| {
            run.execution
                .inspector
                .selection()
                .is_none_or(|selection| selection.node() != node)
        });
        if let Some(canvas) = repaint {
            // SAFETY: bounds arrive during prepaint, where notify cannot queue another frame.
            // Defer until draw completes; on_next_frame alone can wait for an unscheduled frame.
            cx.defer_in(window, move |ws, window, cx| {
                ws.repaint_execution_canvas(id, canvas, window, cx)
            });
        }
        if let Some(node) = selection_changed {
            self.select_execution_node(id, node, cx);
        }
    }

    fn repaint_execution_canvas(
        &mut self,
        id: FlowDetailId,
        canvas: gpui::EntityId,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_page() != Some(crate::workspace::pages::Page::Flows)
            || !self.execution_layout(window, cx).overview()
        {
            return;
        }
        let Some(run) = self.pages.flows.run(id) else {
            return;
        };
        if let Some(graph) = run.execution.graph.view()
            && graph.canvas.entity_id() == canvas
        {
            graph.canvas.update(cx, |_, cx| cx.notify());
        }
    }
}

#[cfg(test)]
mod tests {}
