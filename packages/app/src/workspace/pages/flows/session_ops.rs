//! Continue an exact recorded session in the existing Workspace chat host.

use super::{
    detail::{FlowDetail, FlowDetailBody, FlowDetailId},
    execution_detail::{Inspection, RunOrigin},
    execution_selection::ExecutionSelection,
};
use crate::{
    surface::strings as s,
    workspace::{Workspace, layout::snap::RestorableSession},
};
use daruda_store::accounts::{AccountId, AccountSelection};
use gpui::{Context, Window};

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

    pub(in crate::workspace) fn on_continue_flow_session(
        &mut self,
        _: &crate::workspace::ContinueFlowSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let detail = self
            .pages
            .flows
            .detail
            .as_ref()
            .filter(|d| matches!(d.body, FlowDetailBody::Run(_)))
            .map(|d| d.id);
        if self.active_page() == Some(crate::workspace::pages::Page::Flows)
            && let Some(id) = detail
        {
            if let Some(reason) = self.flow_session_problem(id) {
                self.flow_session_error(reason, cx);
            } else {
                self.continue_flow_session(id, window, cx);
            }
        } else {
            self.flow_session_error(s::flow::run_select_node(), cx);
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

    pub(in crate::workspace) fn continue_flow_session(
        &mut self,
        id: FlowDetailId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(FlowDetail {
            lane,
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
        if !history.can_continue() {
            return;
        }
        let Some(binding) = history.binding(node, *evidence).cloned() else {
            return;
        };
        let origin = RunOrigin {
            lane: *lane,
            dir: run.dir.clone(),
            node: node.clone(),
            evidence: *evidence,
        };
        // Re-read off the UI thread before attaching: the run may have resumed since the detail loaded.
        let dir = origin.dir.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(cx, move || {
            daruda_flow::history::RunHistory::read(&dir).is_ok_and(|history| history.can_continue()
                && history.binding(&binding.node, binding.evidence) == Some(&binding))
                .then_some(binding)
        }, move |ws, binding, cx| {
            let handle = ws.window_runtime.handle;
            let entity = cx.entity();
            cx.defer(move |cx| crate::windows::try_update_workspace_window(handle, cx, "flow.session.continue", move |window, cx| {
                entity.update(cx, |ws, cx| {
                    if ws.active_page() != Some(crate::workspace::pages::Page::Flows)
                        || !ws.pages.flows.detail.as_ref().is_some_and(|d| d.id == id
                            && matches!(&d.body, FlowDetailBody::Run(run) if run.execution.selection.as_ref() == Some(&ExecutionSelection::Attempt(origin.node.clone(), origin.evidence)))) { return; }
                    if let Some(binding) = binding { ws.attach_flow_session(origin, binding, window, cx); }
                    else { ws.flow_session_error(s::flow::session_not_ended(), cx); }
                });
            }));
        }).detach();
    }

    fn flow_session_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.report_error(
            daruda_store::observability::error_report::ErrorReport::new(message)
                .at(file!(), line!())
                .dedup("flow.session.restore")
                .build(),
            cx,
        );
    }

    fn resolve_flow_session_identity(
        &self,
        lane_ref: daruda_store::project::LaneRef,
        binding: &daruda_flow::session::SessionBinding,
    ) -> Result<AccountSelection, String> {
        let recorded = binding
            .account
            .as_ref()
            .ok_or_else(s::flow::session_legacy)?;
        let account = match &recorded.managed {
            None => AccountSelection::SystemDefault,
            Some(id) => AccountSelection::Managed(AccountId(
                uuid::Uuid::parse_str(id).map_err(|_| s::flow::session_identity_missing())?,
            )),
        };
        if !self.task_chat_identity_available(&binding.agent, account.to_persisted()) {
            return Err(s::flow::session_identity_missing());
        }
        let local = self
            .lane_for(lane_ref)
            .zip(self.mirrors.agents.iter().find(|a| a.id == binding.agent))
            .is_some_and(|(lane, agent)| {
                lane.effective_session_host(
                    &agent.launch,
                    &self.mirrors.session_hosts,
                    &self.mirrors.session_host_tombstones,
                )
                .is_ok_and(|host| !host.is_remote())
            });
        if !local {
            return Err(s::flow::session_identity_missing());
        }
        Ok(account)
    }

    pub(in crate::workspace) fn flow_session_problem(&self, id: FlowDetailId) -> Option<String> {
        let detail = self.pages.flows.detail.as_ref().filter(|d| d.id == id)?;
        let FlowDetailBody::Run(run) = &detail.body else {
            return None;
        };
        let (Inspection::Ready(history), Some((node, evidence))) = (
            &run.execution.inspection,
            run.execution
                .selection
                .as_ref()
                .and_then(ExecutionSelection::attempt),
        ) else {
            return Some(s::common::loading());
        };
        if !history.can_continue() {
            return Some(s::flow::session_not_ended());
        }
        let Some(binding) = history.binding(node, *evidence) else {
            return Some(s::flow::session_legacy());
        };
        self.resolve_flow_session_identity(detail.lane, binding)
            .err()
    }

    fn attach_flow_session(
        &mut self,
        origin: RunOrigin,
        binding: daruda_flow::session::SessionBinding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let account = match self.resolve_flow_session_identity(origin.lane, &binding) {
            Ok(account) => account,
            Err(reason) => {
                self.flow_session_error(reason, cx);
                return;
            }
        };
        if self
            .lane_for(origin.lane)
            .is_none_or(|lane| {
                !lane.path.is_dir() || !matches!(
                    (daruda_core::path::canonicalize(&binding.cwd), daruda_core::path::canonicalize(&lane.path)),
                    (Ok(cwd), Ok(root)) if daruda_core::path::is_within(&cwd, &root)
                )
            })
            || !binding.cwd.is_dir()
        {
            self.flow_session_error(s::flow::session_worktree_missing(), cx);
            return;
        }
        let session = RestorableSession {
            session_id: binding.session_id,
            agent_id: binding.agent,
            account,
            lane_ref: origin.lane,
            title: Some(format!("{} · {}", origin.node, origin.evidence).into()),
            prompt_preview: None,
            git_branch: None,
            cwd: binding.cwd,
            last_active: std::time::SystemTime::now(),
        };
        self.close_page(cx);
        self.restore_session_with_flow_origin(session, Some(origin), window, cx);
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
                        run.execution.selection =
                            Some(ExecutionSelection::Attempt(origin.node, origin.evidence));
                    }
                });
                ws.show_page(crate::workspace::pages::Page::Flows, cx);
            },
        );
    }
}

#[cfg(test)]
mod tests;
