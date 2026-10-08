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
        let dir = dir.to_path_buf();
        self.leave_page_detail_then(Page::Flows, window, cx, move |ws, _, cx| {
            ws.mutate_durable(cx, |ws, cx| ws.install_run_detail(lane, dir, cx));
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
        let id = FlowDetailId(self.alloc_id());
        let started = self
            .flow_history_of(lane)
            .and_then(|history| {
                history
                    .runs()
                    .iter()
                    .find(|run| run.dir == dir)
                    .map(|run| run.started.clone())
            })
            .unwrap_or_default();
        let report = dir.join(daruda_flow::record::RUN_REPORT_FILE);
        self.pages.flows.detail = Some(FlowDetail {
            id,
            lane,
            body: FlowDetailBody::Run(RunDetail {
                dir,
                started,
                report: RunReport::Loading,
            }),
        });
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || std::fs::read_to_string(report).ok(),
            move |ws, text, cx| ws.land_run_report(id, text, cx),
        )
        .detach();
        cx.notify();
    }

    /// The report read for run detail `id`. Dropped when the detail has
    /// changed since the read began.
    fn land_run_report(&mut self, id: FlowDetailId, text: Option<String>, cx: &mut Context<Self>) {
        let Some(FlowDetail {
            body: FlowDetailBody::Run(run),
            ..
        }) = self.pages.flows.detail.as_mut().filter(|d| d.id == id)
        else {
            return;
        };
        run.report = match text {
            Some(text) => RunReport::Loaded(text.into()),
            None => RunReport::Missing,
        };
        cx.notify();
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

    /// A past run's report as the page's detail — the `--screenshot-scenario
    /// flow-run-detail` entry point. Written to a temp directory, so a
    /// capture leaves nothing in the lane.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_run_detail_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        const REPORT: &str = "\
# Run — release

**Done** in 4m 12s.

| Node | Result | Attempts |
|---|---|---|
| design | passed | 1 |
| build | passed | 2 |
| review | passed | 1 |

## build

The first attempt failed the type check; the retry hint named the missing
import and the second attempt passed.

```text
cargo check -p daruda
    Finished `dev` profile in 41.3s
```
";
        let dir = std::env::temp_dir().join("daruda-shot-run-detail");
        if std::fs::create_dir_all(&dir).is_err()
            || std::fs::write(dir.join(daruda_flow::record::RUN_REPORT_FILE), REPORT).is_err()
        {
            return;
        }
        self.open_page(Page::Flows, window, cx);
        self.open_run_detail(self.active, &dir, window, cx);
    }
}
