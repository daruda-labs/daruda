//! Display-only projections of a live run: the row handed to every
//! surface that draws one.
//!
//! Kept out of [`super::flow_ops`] because a row changes when a surface
//! does — a column, an order, a word — while starting and stopping a run
//! changes for reasons of its own.

use gpui::App;

use super::Workspace;
use crate::runs::{ParkedAsk, RunsHost};

/// One running flow, flattened for the render pass.
///
/// Shared by the two surfaces that draw runs — the status bar chip, which
/// lists every lane, and the Flows panel, which lists one. They differ in
/// which rows they get, not in what a row is.
///
/// `PartialEq` without `Eq`: the adapter's permission choices carry only
/// `PartialEq`, and the one thing this derive is for — the right dock's
/// notify-on-change diff — needs nothing more.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::workspace) struct FlowRunRow {
    pub lane: daruda_store::project::LaneRef,
    pub run_dir: std::path::PathBuf,
    pub source: super::flow_request::FlowSource,
    /// [`super::lane_ops::lane_label`] — a run in another lane is the case
    /// the chip exists for, so the row has to say which lane it is.
    pub lane_label: gpui::SharedString,
    /// What that run is doing, already worded (see [`super::flow_runs::describe`]).
    pub doing: gpui::SharedString,
    /// Every question the run is waiting on, oldest first, projected for
    /// display. Each node asks on its own, so each is answered on its own.
    /// The reply channels stay with the run: the snapshot is compared field
    /// by field, and a channel has no equality.
    pub asking: Vec<AskRowData>,
}

/// A parked question in the shape a surface draws, plus the id an answer
/// has to quote back.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::workspace) struct AskRowData {
    pub ask_id: u64,
    /// Who is asking, and for what — the line the question sits under.
    pub asked: gpui::SharedString,
    pub tool: gpui::SharedString,
    pub detail: Option<gpui::SharedString>,
    pub options: Vec<daruda_acp::PermissionChoice>,
}

impl AskRowData {
    pub(in crate::workspace) fn of(question: &ParkedAsk) -> Self {
        Self {
            ask_id: question.ask_id(),
            asked: super::flow_runs::describe_question(question).into(),
            tool: question.tool().to_string().into(),
            detail: question.detail().map(|detail| detail.to_string().into()),
            options: question.options().to_vec(),
        }
    }
}

impl Workspace {
    /// Every run in this window's lanes, in the shape the status bar draws.
    /// Across lanes on purpose — a run the user cannot currently see is
    /// exactly what the chip is for.
    pub(in crate::workspace) fn flow_status_rows(&self, cx: &App) -> Vec<FlowRunRow> {
        self.flow_rows_matching(|_| true, cx)
    }

    /// Active-worktree projection for lifecycle regression tests.
    #[cfg(test)]
    pub(in crate::workspace) fn flow_rows_for_active_lane(&self, cx: &App) -> Vec<FlowRunRow> {
        let active = self.active;
        self.flow_rows_matching(|lane| lane == active, cx)
    }

    /// The one place a run becomes a row, so the two surfaces cannot drift
    /// in how they word a stage or order themselves.
    pub(in crate::workspace) fn flow_rows_matching(
        &self,
        keep: impl Fn(daruda_store::project::LaneRef) -> bool,
        cx: &App,
    ) -> Vec<FlowRunRow> {
        // One row per run: a window can hold a folder under two lanes, and
        // a run listed twice would also count its questions twice. The lane
        // in view wins, so a row names the lane the person is looking at.
        let mut lanes: Vec<_> = self
            .projects
            .lanes()
            .filter(|(lane, _, _)| keep(*lane))
            .collect();
        lanes.sort_by_key(|(lane, _, _)| *lane != self.active);
        let mut seen = std::collections::HashSet::new();
        let mut rows: Vec<FlowRunRow> = lanes
            .into_iter()
            .filter_map(|(lane, _, held)| {
                let view = RunsHost::flow_run(&held.path, cx)?.view();
                if !seen.insert(view.run_dir()) {
                    return None;
                }
                Some(FlowRunRow {
                    lane,
                    run_dir: view.run_dir().to_path_buf(),
                    source: view.source().clone(),
                    lane_label: self.lane_label_for(lane).into(),
                    doing: super::flow_runs::describe(view).into(),
                    asking: view
                        .questions()
                        .filter(|ask| !ask.is_closed())
                        .map(AskRowData::of)
                        .collect(),
                })
            })
            .collect();
        // A surface that reshuffles every repaint is unreadable.
        rows.sort_by(|a, b| a.lane_label.cmp(&b.lane_label));
        rows
    }
}
