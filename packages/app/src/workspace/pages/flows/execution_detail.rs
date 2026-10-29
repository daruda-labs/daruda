//! Run inspection state and operations, independent of editor drafts.

use super::{
    detail::{FlowDetail, FlowDetailBody, FlowDetailId},
    execution_selection::ExecutionSelection,
    graph::execution::ExecutionGraph,
};
use crate::workspace::Workspace;
use daruda_flow::{NodeId, history::RunHistory};
use daruda_store::project::LaneRef;
use gpui::{Context, SharedString};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct RunOrigin {
    pub lane: LaneRef,
    pub dir: PathBuf,
    pub node: NodeId,
    pub evidence: u32,
}

#[derive(Default)]
pub(in crate::workspace) enum Inspection {
    #[default]
    Loading,
    Ready(Box<RunHistory>),
    Failed(SharedString),
}

#[derive(Default)]
pub(in crate::workspace) enum Evidence {
    #[default]
    Empty,
    Loading,
    Ready {
        text: SharedString,
        truncated: bool,
    },
    Missing,
    Failed(SharedString),
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::workspace) enum EvidenceTab {
    #[default]
    Conversation,
    Output,
    Metadata,
}

#[derive(Clone)]
pub(in crate::workspace) struct HistoryEntry {
    pub entry: crate::workspace::flow_history::FlowRunEntry,
    pub source: Option<PathBuf>,
    pub started_at: Option<std::time::SystemTime>,
    pub elapsed: Option<std::time::Duration>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum ExecutionContent {
    #[default]
    Attempts,
    Report,
}

#[derive(Default)]
pub(in crate::workspace) struct ExecutionDetail {
    pub inspection: Inspection,
    pub graph: Option<ExecutionGraph>,
    pub watch: Option<gpui::Subscription>,
    pub theme_watch: Option<gpui::Subscription>,
    read: read::ReadState,
    pub evidence_epoch: u64,
    pub selection: Option<ExecutionSelection>,
    pub evidence: Evidence,
    pub tab: EvidenceTab,
    pub(super) report_epoch: u64,
    pub content: ExecutionContent,
    pub rail: Vec<HistoryEntry>,
    pub source_only: bool,
    pub filter: crate::workspace::flow_browser::RunFilter,
}

mod read;

impl Workspace {
    fn land_run_execution(
        &mut self,
        id: FlowDetailId,
        result: Result<RunHistory, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = self.pages.flows.detail.as_mut().filter(|d| d.id == id) else {
            return;
        };
        let FlowDetailBody::Run(run) = &mut detail.body else {
            return;
        };
        run.execution.inspection = match result {
            Ok(history) => {
                if run.execution.selection.is_none() {
                    run.execution.selection = history
                        .latest_selection()
                        .map(|(node, evidence)| ExecutionSelection::Attempt(node, evidence))
                        .or_else(|| {
                            history
                                .flow
                                .as_ref()?
                                .nodes
                                .first()
                                .map(|n| ExecutionSelection::Node(n.id.clone()))
                        });
                } else if let Some(ExecutionSelection::Node(node)) = &run.execution.selection {
                    run.execution.selection =
                        Some(ExecutionSelection::for_node(&history, node.clone()));
                }
                Inspection::Ready(Box::new(history))
            }
            Err(error) => {
                let text = error.clone();
                self.report_error(
                    daruda_store::observability::error_report::ErrorReport::new(
                        crate::surface::strings::flow::run_history_failed(),
                    )
                    .message(error)
                    .at(file!(), line!())
                    .dedup("flow.history.read")
                    .build(),
                    cx,
                );
                // Reacquire after error reporting, which also updates workspace state.
                if let Some(FlowDetail {
                    body: FlowDetailBody::Run(run),
                    ..
                }) = self.pages.flows.detail.as_mut().filter(|d| d.id == id)
                {
                    run.execution.inspection = Inspection::Failed(text.into());
                }
                cx.notify();
                return;
            }
        };
        self.load_run_evidence(id, cx);
        let handle = self.window_runtime.handle;
        let entity = cx.entity();
        // No live Window exists in this background completion; enter after the entity update ends.
        cx.defer(move |cx| {
            crate::windows::try_update_workspace_window(
                handle,
                cx,
                "flow.execution.graph",
                move |window, cx| {
                    entity.update(cx, |ws, cx| ws.build_execution_graph(id, window, cx));
                },
            )
        });
        cx.notify();
    }

    fn build_execution_graph(
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
        if history.flow.is_none() {
            run.execution.graph = None;
            run.execution.watch = None;
            return;
        }
        if let Some(graph) = &run.execution.graph
            && graph.matches_theme(cx)
        {
            graph.refresh(history, cx);
            return;
        }
        let graph = ExecutionGraph::new(history, window, cx);
        if let (Some(graph), Some(selection)) = (&graph, &run.execution.selection) {
            graph.select(selection.node(), cx);
        }
        if run.execution.theme_watch.is_none() {
            let weak = cx.weak_entity();
            run.execution.theme_watch = Some(
                window.observe_global::<crate::ui::theme::DarudaTheme>(cx, move |window, cx| {
                    if let Some(ws) = weak.upgrade() {
                        ws.update(cx, |ws, cx| {
                            ws.refresh_execution_graph_theme(id, window, cx)
                        });
                    }
                }),
            );
        }
        run.execution.watch = graph.as_ref().map(|graph| {
            let weak = cx.weak_entity();
            window.observe(&graph.canvas, cx, move |_, window, cx| {
                if let Some(ws) = weak.upgrade() {
                    ws.update(cx, |ws, cx| ws.execution_graph_changed(id, window, cx));
                }
            })
        });
        run.execution.graph = graph;
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
                FlowDetailBody::Run(run) => run.execution.graph.as_ref(),
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
        let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_mut().filter(|d| d.id == id)
        else {
            return;
        };
        let Some(graph) = run.execution.graph.as_mut() else {
            return;
        };
        let selected = graph.selected(cx);
        let repaint = graph
            .needs_layout_repaint(cx)
            .then(|| graph.canvas.entity_id());
        let selection_changed = selected.filter(|node| {
            run.execution
                .selection
                .as_ref()
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
        let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_ref().filter(|d| d.id == id)
        else {
            return;
        };
        if let Some(graph) = &run.execution.graph
            && graph.canvas.entity_id() == canvas
        {
            graph.canvas.update(cx, |_, cx| cx.notify());
        }
    }

    pub(in crate::workspace) fn set_execution_filter(
        &mut self,
        id: FlowDetailId,
        filter: crate::workspace::flow_browser::RunFilter,
        cx: &mut Context<Self>,
    ) {
        if let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_mut().filter(|d| d.id == id)
        {
            run.execution.filter = filter;
            cx.notify();
        }
    }

    pub(in crate::workspace) fn toggle_execution_source(
        &mut self,
        id: FlowDetailId,
        cx: &mut Context<Self>,
    ) {
        if let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_mut().filter(|d| d.id == id)
        {
            run.execution.source_only = !run.execution.source_only;
            cx.notify();
        }
    }

    pub(in crate::workspace) fn refresh_execution(
        &mut self,
        id: FlowDetailId,
        cx: &mut Context<Self>,
    ) {
        if let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_ref().filter(|d| d.id == id)
        {
            let dir = run.dir.clone();
            if run.execution.content == ExecutionContent::Report {
                self.show_execution_report(id, cx);
            }
            self.read_run_execution(id, dir, cx);
        }
    }

    pub(in crate::workspace) fn select_execution_node(
        &mut self,
        id: FlowDetailId,
        node: NodeId,
        cx: &mut Context<Self>,
    ) {
        let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_mut().filter(|d| d.id == id)
        else {
            return;
        };
        let Inspection::Ready(history) = &run.execution.inspection else {
            return;
        };
        if run
            .execution
            .selection
            .as_ref()
            .is_some_and(|s| s.node() == &node)
        {
            return;
        }
        run.execution.selection = Some(ExecutionSelection::for_node(history, node.clone()));
        if let Some(graph) = &run.execution.graph {
            graph.select(&node, cx);
        }
        self.load_run_evidence(id, cx);
        cx.notify();
    }

    pub(in crate::workspace) fn select_execution_attempt(
        &mut self,
        id: FlowDetailId,
        evidence: u32,
        cx: &mut Context<Self>,
    ) {
        let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_mut().filter(|d| d.id == id)
        else {
            return;
        };
        let Some(selection) = &run.execution.selection else {
            return;
        };
        let Inspection::Ready(history) = &run.execution.inspection else {
            return;
        };
        let node = selection.node();
        if !history
            .attempts(node)
            .iter()
            .any(|(_, seq)| *seq == evidence)
        {
            return;
        }
        run.execution.selection = Some(ExecutionSelection::Attempt(node.clone(), evidence));
        self.load_run_evidence(id, cx);
        cx.notify();
    }

    pub(in crate::workspace) fn select_execution_tab_index(
        &mut self,
        id: FlowDetailId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let tab = match index {
            0 => EvidenceTab::Conversation,
            1 => EvidenceTab::Output,
            2 => EvidenceTab::Metadata,
            _ => return,
        };
        self.select_execution_tab(id, tab, cx);
    }

    pub(in crate::workspace) fn select_execution_tab(
        &mut self,
        id: FlowDetailId,
        tab: EvidenceTab,
        cx: &mut Context<Self>,
    ) {
        let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_mut().filter(|d| d.id == id)
        else {
            return;
        };
        if run.execution.tab == tab {
            return;
        }
        run.execution.tab = tab;
        self.load_run_evidence(id, cx);
        cx.notify();
    }

    fn load_run_evidence(&mut self, id: FlowDetailId, cx: &mut Context<Self>) {
        let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_mut().filter(|d| d.id == id)
        else {
            return;
        };
        let Inspection::Ready(history) = &run.execution.inspection else {
            return;
        };
        let Some(selection) = run.execution.selection.clone() else {
            run.execution.evidence = Evidence::Empty;
            return;
        };
        let Some((node, evidence)) = selection.attempt() else {
            run.execution.evidence_epoch += 1;
            run.execution.evidence = Evidence::Empty;
            return;
        };
        let path = match run.execution.tab {
            EvidenceTab::Conversation => history.transcript(&run.dir, node, *evidence),
            EvidenceTab::Output => history.output(&run.dir, node, *evidence),
            EvidenceTab::Metadata => None,
        };
        run.execution.evidence_epoch += 1;
        let epoch = run.execution.evidence_epoch;
        run.execution.evidence = Evidence::Loading;
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || super::execution_evidence::read(path),
            move |ws, text, cx| {
                let Some(FlowDetail {
                    body: FlowDetailBody::Run(run),
                    ..
                }) = ws.pages.flows.detail.as_mut().filter(|d| d.id == id)
                else {
                    return;
                };
                if run.execution.selection.as_ref() != Some(&selection)
                    || run.execution.evidence_epoch != epoch
                {
                    return;
                }
                run.execution.evidence = match text {
                    super::execution_evidence::ReadEvidence::Missing => Evidence::Missing,
                    super::execution_evidence::ReadEvidence::Ready { text, truncated } => {
                        Evidence::Ready {
                            text: text.into(),
                            truncated,
                        }
                    }
                    super::execution_evidence::ReadEvidence::Failed(error) => {
                        let text = error.clone();
                        ws.report_error(
                            daruda_store::observability::error_report::ErrorReport::new(
                                crate::surface::strings::flow::run_evidence_failed(),
                            )
                            .message(error)
                            .at(file!(), line!())
                            .dedup("flow.evidence.read")
                            .build(),
                            cx,
                        );
                        if let Some(FlowDetail {
                            body: FlowDetailBody::Run(run),
                            ..
                        }) = ws.pages.flows.detail.as_mut().filter(|d| d.id == id)
                        {
                            run.execution.evidence = Evidence::Failed(text.into());
                        }
                        cx.notify();
                        return;
                    }
                };
                cx.notify();
            },
        )
        .detach();
    }

    pub(in crate::workspace) fn open_selected_execution_file(
        &mut self,
        id: FlowDetailId,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let tab = self
            .pages
            .flows
            .detail
            .as_ref()
            .filter(|d| d.id == id)
            .and_then(|d| match &d.body {
                FlowDetailBody::Run(run) => Some(run.execution.tab),
                _ => None,
            });
        match tab {
            Some(EvidenceTab::Output) => self.open_execution_output(id, window, cx),
            Some(EvidenceTab::Conversation) => self.open_execution_evidence(id, window, cx),
            _ => {}
        }
    }

    pub(in crate::workspace) fn open_execution_evidence(
        &mut self,
        id: FlowDetailId,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_ref().filter(|d| d.id == id)
        else {
            return;
        };
        let (Inspection::Ready(history), Some((node, evidence))) = (
            &run.execution.inspection,
            run.execution
                .selection
                .as_ref()
                .and_then(ExecutionSelection::attempt),
        ) else {
            return;
        };
        if let Some(path) = history.transcript(&run.dir, node, *evidence) {
            self.open_linked_file(path, window, cx);
        }
    }

    pub(in crate::workspace) fn open_execution_output(
        &mut self,
        id: FlowDetailId,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_ref().filter(|d| d.id == id)
        else {
            return;
        };
        let (Inspection::Ready(history), Some((node, evidence))) = (
            &run.execution.inspection,
            run.execution
                .selection
                .as_ref()
                .and_then(ExecutionSelection::attempt),
        ) else {
            return;
        };
        if let Some(path) = history.output(&run.dir, node, *evidence) {
            self.open_linked_file(path, window, cx);
        }
    }
}

#[cfg(test)]
mod tests;
