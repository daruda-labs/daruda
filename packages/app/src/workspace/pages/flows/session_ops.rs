//! Continue an exact recorded session in the existing Workspace chat host.

use crate::workspace::flow_navigation::RunOrigin;

use super::{
    detail::{FlowDetail, FlowDetailBody, FlowDetailId},
    execution_detail::Inspection,
    execution_selection::ExecutionSelection,
};
use crate::{
    surface::strings as s,
    workspace::{Workspace, layout::snap::RestorableSession},
};
use daruda_store::accounts::{AccountId, AccountSelection};
use gpui::{Context, Window};

mod identity;
mod navigation;

struct ContinuationRequest {
    origin: RunOrigin,
    binding: daruda_flow::session::SessionBinding,
}

impl ContinuationRequest {
    fn revalidate(self) -> Option<daruda_flow::session::SessionBinding> {
        // The engine may have resumed since the user opened the execution detail.
        daruda_flow::history::RunHistory::read(&self.origin.dir)
            .is_ok_and(|history| {
                history.can_continue()
                    && history.binding(&self.binding.node, self.binding.evidence)
                        == Some(&self.binding)
            })
            .then_some(self.binding)
    }
}

impl Workspace {
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
            self.continue_flow_session(id, window, cx);
        } else {
            self.flow_session_error(s::flow::run_select_node(), cx);
        }
    }

    pub(in crate::workspace) fn continue_flow_session(
        &mut self,
        id: FlowDetailId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let request = match self.prepare_flow_session(id) {
            Ok(request) => request,
            Err(reason) => {
                self.flow_session_error(reason, cx);
                return;
            }
        };
        let origin = request.origin.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || request.revalidate(),
            move |ws, binding, cx| ws.land_flow_session(id, origin, binding, cx),
        )
        .detach();
    }

    fn land_flow_session(
        &mut self,
        id: FlowDetailId,
        origin: RunOrigin,
        binding: Option<daruda_flow::session::SessionBinding>,
        cx: &mut Context<Self>,
    ) {
        let handle = self.window_runtime.handle;
        let entity = cx.entity();
        // Background completion has no live window; enter only after this update ends.
        cx.defer(move |cx| {
            crate::windows::try_update_workspace_window(
                handle,
                cx,
                "flow.session.continue",
                move |window, cx| {
                    entity.update(cx, |ws, cx| {
                        ws.complete_flow_session(id, origin, binding, window, cx)
                    });
                },
            )
        });
    }

    fn complete_flow_session(
        &mut self,
        id: FlowDetailId,
        origin: RunOrigin,
        binding: Option<daruda_flow::session::SessionBinding>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_page() != Some(crate::workspace::pages::Page::Flows) {
            return;
        }
        let Some(run) = self.pages.flows.run(id) else {
            return;
        };
        if run.execution.inspector.selection()
            != Some(&ExecutionSelection::Attempt(
                origin.node.clone(),
                origin.evidence,
            ))
        {
            return;
        }
        match binding {
            Some(binding) => self.attach_flow_session(origin, binding, window, cx),
            None => self.flow_session_error(s::flow::session_not_ended(), cx),
        }
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

    pub(in crate::workspace) fn flow_session_problem(&self, id: FlowDetailId) -> Option<String> {
        self.prepare_flow_session(id).err()
    }

    fn prepare_flow_session(&self, id: FlowDetailId) -> Result<ContinuationRequest, String> {
        let detail = self
            .pages
            .flows
            .detail
            .as_ref()
            .filter(|d| d.id == id)
            .ok_or_else(s::flow::run_select_node)?;
        let FlowDetailBody::Run(run) = &detail.body else {
            return Err(s::flow::run_select_node());
        };
        let (Inspection::Ready(history), Some((node, evidence))) = (
            &run.execution.inspection,
            run.execution
                .inspector
                .selection()
                .and_then(ExecutionSelection::attempt),
        ) else {
            return Err(s::common::loading());
        };
        if !history.can_continue() {
            return Err(s::flow::session_not_ended());
        }
        let binding = history
            .binding(node, *evidence)
            .ok_or_else(s::flow::session_legacy)?;
        self.resolve_flow_session_identity(detail.lane, binding)?;
        Ok(ContinuationRequest {
            origin: RunOrigin {
                lane: detail.lane,
                dir: run.dir.clone(),
                node: node.clone(),
                evidence: *evidence,
            },
            binding: binding.clone(),
        })
    }
}

#[cfg(test)]
mod tests;
