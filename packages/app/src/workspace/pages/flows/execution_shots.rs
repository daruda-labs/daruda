//! Execution-history screenshots use isolated, real run-log fixtures.

use super::execution_fixture::{ExecutionShot, seed};
use crate::workspace::Workspace;
use gpui::{Context, Window};

impl Workspace {
    pub(in crate::workspace) fn open_run_detail_for_shot(
        &mut self,
        shot: ExecutionShot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = tempfile::Builder::new()
            .prefix("daruda-execution-shot-")
            .tempdir()
            .expect("screenshot directory")
            .keep();
        let dir = seed(&root, shot);
        let mut project =
            crate::project::Project::bootstrap_placeholder(self.next_project_id, root);
        project.name = "Flow history preview".into();
        self.next_project_id += 1;
        let lane = project.first_lane_ref().expect("fixture worktree");
        self.projects.push(project);
        self.set_active(lane);
        self.open_page(crate::workspace::pages::Page::Flows, window, cx);
        self.open_run_detail(self.active, &dir, window, cx);
        if shot == ExecutionShot::Docked {
            self.docks.left.update(cx, |dock, cx| {
                dock.open();
                cx.notify();
            });
            self.docks.right.update(cx, |dock, cx| {
                dock.open();
                cx.notify();
            });
        }
        if shot == ExecutionShot::Pending
            && let Some(super::detail::FlowDetail {
                body: super::detail::FlowDetailBody::Run(run),
                ..
            }) = &mut self.pages.flows.detail
        {
            run.execution.inspector.restore_selection(
                super::execution_selection::ExecutionSelection::Node("review".into()),
            );
        }
        if let Some(detail) = &self.pages.flows.detail {
            let id = detail.id;
            let tab = match shot {
                ExecutionShot::Output => super::execution_detail::EvidenceTab::Output,
                ExecutionShot::Metadata => super::execution_detail::EvidenceTab::Metadata,
                _ => super::execution_detail::EvidenceTab::Conversation,
            };
            self.select_execution_tab(id, tab, cx);
            if shot == ExecutionShot::Report {
                self.show_execution_report(id, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {}
