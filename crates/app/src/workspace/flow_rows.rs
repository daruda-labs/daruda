//! Display-only projections of a live run: the row handed to every
//! surface that draws one.
//!
//! Kept out of [`super::flow_ops`] because a row changes when a surface
//! does — a column, an order, a word — while starting and stopping a run
//! changes for reasons of its own.

use super::Workspace;
use super::flow_runs::RunStage;

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
    /// [`super::lane_ops::lane_label`] — a run in another lane is the case
    /// the chip exists for, so the row has to say which lane it is.
    pub lane_label: gpui::SharedString,
    /// What that run is doing, already worded (see `RunStage`).
    pub doing: gpui::SharedString,
    /// The question this run is waiting on, projected for display. The
    /// reply channel stays behind in [`super::flow_runs`] — the snapshot is compared
    /// field by field (`RightDockSnapshot::content_differs`) and a channel
    /// has no equality, so it could not travel here even if it should.
    pub asking: Option<AskRowData>,
    /// How many more are behind the one being shown. Zero for a run with a
    /// single question, which is every serial run.
    pub also_waiting: usize,
}

/// A parked question in the shape a surface draws, plus the id an answer
/// has to quote back.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::workspace) struct AskRowData {
    pub ask_id: u64,
    pub tool: gpui::SharedString,
    pub detail: Option<gpui::SharedString>,
    pub options: Vec<daruda_acp::PermissionChoice>,
}

impl Workspace {
    /// Every run this window started, in the shape the status bar draws.
    /// Across lanes on purpose — a run the user cannot currently see is
    /// exactly what the chip is for.
    pub(in crate::workspace) fn flow_status_rows(&self) -> Vec<FlowRunRow> {
        self.flow_rows_matching(|_| true)
    }

    /// The runs in the lane the right dock is showing. The panel answers
    /// questions and opens run directories, and both of those belong to one
    /// lane — `flow-runs/` is per working directory, so a panel spanning
    /// lanes could not show a coherent history beside them.
    pub(in crate::workspace) fn flow_rows_for_active_lane(&self) -> Vec<FlowRunRow> {
        let active = self.active;
        self.flow_rows_matching(|lane| lane == active)
    }

    /// The one place a run becomes a row, so the two surfaces cannot drift
    /// in how they word a stage or order themselves.
    pub(in crate::workspace) fn flow_rows_matching(
        &self,
        keep: impl Fn(daruda_store::project::LaneRef) -> bool,
    ) -> Vec<FlowRunRow> {
        let mut rows: Vec<FlowRunRow> = self
            .runs
            .iter()
            .filter(|(lane, _)| keep(*lane))
            .map(|(lane, handle)| FlowRunRow {
                lane,
                lane_label: self.lane_label_for(lane).into(),
                doing: handle.doing.describe().into(),
                asking: match &handle.doing {
                    RunStage::Asking { question, .. } => Some(AskRowData {
                        ask_id: question.ask_id,
                        tool: question.tool.clone().into(),
                        detail: question.detail.clone().map(Into::into),
                        options: question.options.clone(),
                    }),
                    _ => None,
                },
                also_waiting: match &handle.doing {
                    RunStage::Asking { queued, .. } => queued.len(),
                    _ => 0,
                },
            })
            .collect();
        // A `HashMap` has no order, and a surface that reshuffles every
        // repaint is unreadable.
        rows.sort_by(|a, b| a.lane_label.cmp(&b.lane_label));
        rows
    }
}
