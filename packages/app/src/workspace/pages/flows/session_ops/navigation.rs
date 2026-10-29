//! History navigation and return links.

use super::*;

impl Workspace {
    pub(in crate::workspace) fn on_open_flow_history(
        &mut self,
        _: &crate::workspace::OpenFlowHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((_, lane, graph)) = self.open_graph() {
            let path = graph.read(cx).path().to_path_buf();
            self.open_graph_execution_history(lane, &path, window, cx);
        } else {
            self.set_flow_tab(crate::workspace::flow_browser::FlowTab::Runs, cx);
            self.open_page(crate::workspace::pages::Page::Flows, window, cx);
        }
    }

    pub(in crate::workspace) fn on_return_to_flow_execution(
        &mut self,
        _: &crate::workspace::ReturnToFlowExecution,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let origin = self
            .main_area
            .pane(self.active_runtime().focused_pane_id)
            .and_then(|pane| pane.agent_chat_content())
            .and_then(|chat| chat.restore.flow_origin())
            .cloned();
        if let Some(origin) = origin {
            self.open_flow_run_origin(origin, window, cx);
        } else {
            self.on_open_flow_history(&crate::workspace::OpenFlowHistory, window, cx);
        }
    }

    pub(in crate::workspace) fn open_graph_execution_history(
        &mut self,
        lane: daruda_store::project::LaneRef,
        path: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(cwd) = self.lane_for(lane).map(|lane| lane.path.clone()) else {
            return;
        };
        let source = daruda_core::path::strip_root(path, &cwd);
        let runs = crate::workspace::flow_paths::runs_dir(&cwd);
        self.leave_page_detail_then(
            crate::workspace::pages::Page::Flows,
            window,
            cx,
            move |ws, _, cx| {
                ws.set_flow_scope(
                    crate::workspace::flow_browser::FlowScope::Worktree(lane),
                    cx,
                );
                ws.set_flow_tab(crate::workspace::flow_browser::FlowTab::Runs, cx);
                ws.show_page(crate::workspace::pages::Page::Flows, cx);
                let epoch = ws.alloc_id();
                ws.pages.flows.history_request = Some(epoch);
                crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
                    cx,
                    move || {
                        let history =
                            crate::workspace::flow_history::FlowHistory::read(&runs, None);
                        history
                            .runs()
                            .iter()
                            .find(|entry| {
                                source.is_some()
                                    && daruda_flow::run_log::reader::inspect(&entry.dir)
                                        .ok()
                                        .flatten()
                                        .is_some_and(|i| i.state.source() == source.as_deref())
                            })
                            .map(|entry| entry.dir.clone())
                    },
                    move |ws, dir, cx| {
                        if ws.pages.flows.history_request != Some(epoch)
                            || ws.pages.flows.detail.is_some()
                            || ws.active_page() != Some(crate::workspace::pages::Page::Flows)
                            || ws.flow_browser_lane() != lane
                        {
                            return;
                        }
                        ws.pages.flows.history_request = None;
                        if let Some(dir) = dir {
                            ws.mutate_durable(cx, |ws, cx| ws.install_run_detail(lane, dir, cx));
                            if let Some(FlowDetail {
                                body: FlowDetailBody::Run(run),
                                ..
                            }) = ws.pages.flows.detail.as_mut()
                            {
                                run.execution.source_only = true;
                            }
                        }
                    },
                )
                .detach();
            },
        );
    }

    pub(in crate::workspace) fn open_flow_run_origin(
        &mut self,
        origin: RunOrigin,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.lane_for(origin.lane).is_none() || !origin.dir.is_dir() {
            self.flow_session_error(s::flow::session_worktree_missing(), cx);
            return;
        }
        self.leave_page_detail_then(
            crate::workspace::pages::Page::Flows,
            window,
            cx,
            move |ws, _, cx| {
                ws.mutate_durable(cx, |ws, cx| {
                    ws.install_run_detail(origin.lane, origin.dir, cx);
                    if let Some(FlowDetail {
                        body: FlowDetailBody::Run(run),
                        ..
                    }) = ws.pages.flows.detail.as_mut()
                    {
                        run.execution
                            .inspector
                            .restore_selection(ExecutionSelection::Attempt(
                                origin.node,
                                origin.evidence,
                            ));
                    }
                });
                ws.show_page(crate::workspace::pages::Page::Flows, cx);
            },
        );
    }
}

#[cfg(test)]
mod tests {}
