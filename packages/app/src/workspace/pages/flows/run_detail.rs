//! A past run as the Flows page's detail: its report, read where the run
//! list is, rather than in a lane tab that pulls the person off the page.

use std::path::{Path, PathBuf};

use daruda_store::project::LaneRef;
use gpui::{Context, Window};

use super::detail::{FlowDetail, FlowDetailBody, FlowDetailId, RunDetail, RunReport};
use crate::workspace::Workspace;
use crate::workspace::pages::Page;

impl Workspace {
    /// Show `lane`'s past run `dir` as the Flows page's detail. The same run
    /// already open is shown as it is; anything else replaces the detail,
    /// asking first if that holds edits.
    pub(in crate::workspace) fn open_run_detail(
        &mut self,
        lane: LaneRef,
        dir: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = self.pages.flows.detail.as_ref().is_some_and(|detail| {
            detail.lane == lane
                && matches!(&detail.body, FlowDetailBody::Run(run) if run.dir == dir)
        });
        if open {
            self.show_page(Page::Flows, cx);
            return;
        }
        let source_only = self.pages.flows.detail.as_ref().is_some_and(
            |d| matches!(&d.body, FlowDetailBody::Run(run) if run.execution.source_only),
        );
        let filter = self
            .pages
            .flows
            .detail
            .as_ref()
            .and_then(|d| match &d.body {
                FlowDetailBody::Run(run) => Some(run.execution.filter),
                _ => None,
            })
            .unwrap_or_default();
        let dir = dir.to_path_buf();
        self.leave_page_detail_then(Page::Flows, window, cx, move |ws, _, cx| {
            ws.mutate_durable(cx, |ws, cx| {
                ws.install_run_detail(lane, dir, cx);
                if let Some(FlowDetail {
                    body: FlowDetailBody::Run(run),
                    ..
                }) = ws.pages.flows.detail.as_mut()
                {
                    run.execution.source_only = source_only;
                    run.execution.filter = filter;
                }
            });
            ws.show_page(Page::Flows, cx);
        });
    }

    /// Make run `dir` the page's detail and read its report. Shared by opening
    /// and restoring; the caller has left the old one and saves if this is news.
    pub(in crate::workspace) fn install_run_detail(
        &mut self,
        lane: LaneRef,
        dir: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.pages.flows.history_request = None;
        let id = FlowDetailId(self.alloc_id());
        let started = crate::workspace::flow_history::run_started_label(&dir);
        let execution_dir = dir.clone();
        self.pages.flows.detail = Some(FlowDetail {
            id,
            lane,
            body: FlowDetailBody::Run(RunDetail {
                dir,
                started,
                report: RunReport::Loading,
                execution: Default::default(),
            }),
        });
        self.read_run_execution(id, execution_dir, cx);
        self.read_run_report(id, cx);
        cx.notify();
    }

    /// Only the latest read may replace this detail's report.
    fn land_run_report(
        &mut self,
        id: FlowDetailId,
        epoch: u64,
        text: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_mut().filter(|d| d.id == id)
        else {
            return;
        };
        if run.execution.report_epoch != epoch {
            return;
        }
        run.report = match text {
            Some(text) => RunReport::Loaded(text.into()),
            None => RunReport::Missing,
        };
        cx.notify();
    }

    pub(in crate::workspace) fn show_execution_report(
        &mut self,
        id: FlowDetailId,
        cx: &mut Context<Self>,
    ) {
        let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_mut().filter(|d| d.id == id)
        else {
            return;
        };
        run.execution.content = super::execution_detail::ExecutionContent::Report;
        self.read_run_report(id, cx);
    }

    fn read_run_report(&mut self, id: FlowDetailId, cx: &mut Context<Self>) {
        let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_mut().filter(|d| d.id == id)
        else {
            return;
        };
        run.execution.report_epoch += 1;
        let epoch = run.execution.report_epoch;
        run.report = RunReport::Loading;
        let path = run.dir.join(daruda_flow::record::RUN_REPORT_FILE);
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || std::fs::read_to_string(path).ok(),
            move |ws, text, cx| ws.land_run_report(id, epoch, text, cx),
        )
        .detach();
        cx.notify();
    }

    pub(in crate::workspace) fn back_from_execution(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = &mut self.pages.flows.detail
            && run.execution.content == super::execution_detail::ExecutionContent::Report
        {
            run.execution.content = super::execution_detail::ExecutionContent::Attempts;
            cx.notify();
            return;
        }
        self.back_to_flow_list(window, cx);
    }

    /// Open run detail `id`'s report as a file, in the worktree holding it —
    /// for what only the file viewer has, such as search.
    pub(in crate::workspace) fn open_run_report_file(
        &mut self,
        id: FlowDetailId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_ref().filter(|d| d.id == id)
        else {
            return;
        };
        let report = run.dir.join(daruda_flow::record::RUN_REPORT_FILE);
        self.open_linked_file(report, window, cx);
    }

    /// Back from the page's detail to its list, asking first if it holds
    /// edits.
    pub(in crate::workspace) fn back_to_flow_list(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.leave_page_detail_then(Page::Flows, window, cx, |_, _, _| {});
    }
}

#[cfg(test)]
mod tests;
