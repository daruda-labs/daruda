//! Run inspection state and operations, independent of editor drafts.

use super::{
    detail::{FlowDetailBody, FlowDetailId},
    execution_selection::ExecutionSelection,
};
use crate::workspace::Workspace;
use daruda_flow::{NodeId, history::RunHistory};
use gpui::{Context, SharedString};
use std::path::PathBuf;

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
    pub graph: graph::GraphState,
    read: read::ReadState,
    pub inspector: inspector::AttemptInspector,
    pub content: ExecutionContent,
    pub rail: Vec<HistoryEntry>,
    pub source_only: bool,
    pub filter: crate::workspace::flow_browser::RunFilter,
}

mod evidence;
mod graph;
mod inspector;
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
                run.execution.inspector.reconcile(&history);
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
                if let Some(run) = self.pages.flows.run_mut(id) {
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

    pub(in crate::workspace) fn set_execution_filter(
        &mut self,
        id: FlowDetailId,
        filter: crate::workspace::flow_browser::RunFilter,
        cx: &mut Context<Self>,
    ) {
        if let Some(run) = self.pages.flows.run_mut(id) {
            run.execution.filter = filter;
            cx.notify();
        }
    }

    pub(in crate::workspace) fn toggle_execution_source(
        &mut self,
        id: FlowDetailId,
        cx: &mut Context<Self>,
    ) {
        if let Some(run) = self.pages.flows.run_mut(id) {
            run.execution.source_only = !run.execution.source_only;
            cx.notify();
        }
    }

    pub(in crate::workspace) fn refresh_execution(
        &mut self,
        id: FlowDetailId,
        cx: &mut Context<Self>,
    ) {
        if let Some(run) = self.pages.flows.run(id) {
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
        let Some(run) = self.pages.flows.run_mut(id) else {
            return;
        };
        let Inspection::Ready(history) = &run.execution.inspection else {
            return;
        };
        if !run.execution.inspector.select_node(history, node.clone()) {
            return;
        }
        if let Some(graph) = run.execution.graph.view() {
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
        let Some(run) = self.pages.flows.run_mut(id) else {
            return;
        };
        let Inspection::Ready(history) = &run.execution.inspection else {
            return;
        };
        if !run.execution.inspector.select_attempt(history, evidence) {
            return;
        }
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
        let Some(run) = self.pages.flows.run_mut(id) else {
            return;
        };
        if !run.execution.inspector.select_tab(tab) {
            return;
        }
        self.load_run_evidence(id, cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests;
