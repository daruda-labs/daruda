//! Proposing task events from a window, and the effects ordered after them.
//!
//! A user's command is reported when the log refuses it; a fact the app
//! observed goes through `RunsHost::propose_fact`, which only logs. Effects
//! on panes run after the commit, so a refused command touches nothing.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::tasks::ExecutionRef;
use futures::FutureExt as _;
use gpui::{Context, WeakEntity};

use crate::runs::{ReadOnlyCause, RunsHost, TaskCommitError, TaskIntent};
use crate::workspace::Workspace;

/// What the desktop shows for a command the task log did not take.
pub(in crate::workspace) fn task_commit_report(e: &TaskCommitError) -> ErrorReport {
    use crate::surface::strings::error;
    let (title, severity, dedup) = match e {
        TaskCommitError::Rejected(_) => (
            error::task_command_refused(),
            ErrorSeverity::Info,
            "tasks.command_refused",
        ),
        TaskCommitError::ReadOnly(cause) => (
            match cause {
                ReadOnlyCause::UnfinishedReconciliation => error::tasks_read_only(),
                ReadOnlyCause::SettleFailed => error::tasks_settle_failed(),
                ReadOnlyCause::SettleMarkerStuck => error::tasks_settle_marker_stuck(),
            },
            ErrorSeverity::Warning,
            "tasks.read_only",
        ),
        TaskCommitError::Unavailable => (
            error::tasks_unavailable(),
            ErrorSeverity::Warning,
            "tasks.unavailable",
        ),
        TaskCommitError::Failed(_) => (
            error::tasks_save_failed(),
            ErrorSeverity::Error,
            "tasks.save",
        ),
    };
    ErrorReport::new(title)
        .severity(severity)
        .message(e.to_string())
        .at(file!(), line!())
        .dedup(dedup)
        .build()
}

impl Workspace {
    /// Propose a user's task command; a refusal or failure is reported here.
    /// The returned task may be dropped: the commit happens either way.
    pub(in crate::workspace) fn propose_task_command(
        &mut self,
        intent: TaskIntent,
        cx: &mut Context<Self>,
    ) -> gpui::Task<Result<u64, TaskCommitError>> {
        let mut commit = RunsHost::task(intent, cx);
        if let Some(result) = (&mut commit).now_or_never() {
            if let Err(error) = &result {
                self.report_error(task_commit_report(error), cx);
            }
            return gpui::Task::ready(result);
        }
        cx.spawn(async move |ws, cx| {
            let result = commit.await;
            if let Err(e) = &result {
                report_commit_error(&ws, e, cx);
            }
            result
        })
    }

    /// Cancel a run under way: commit `CancelRequested` first, then stop the
    /// run's pane in whichever window holds it.
    pub(in crate::workspace) fn cancel_execution(
        &mut self,
        exec: ExecutionRef,
        cx: &mut Context<Self>,
    ) -> gpui::Task<Result<u64, TaskCommitError>> {
        self.propose_task_command(TaskIntent::Cancel { exec }, cx)
    }

    /// Stop the turn and drop the queue of the chat pane running `exec`;
    /// `false` when this window holds no such pane.
    pub(in crate::workspace) fn stop_task_execution(
        &mut self,
        exec: &ExecutionRef,
        cx: &mut Context<Self>,
    ) -> bool {
        let pane_id = self
            .main_area
            .runtimes
            .values()
            .flat_map(|runtime| runtime.panes.iter())
            .find_map(|pane| {
                (RunsHost::pane_execution(self.run_pane(pane.id), cx) == Some(exec))
                    .then_some(pane.id)
            });
        let Some(pane_id) = pane_id else {
            return false;
        };
        if let Some(view) = self
            .main_area
            .pane(pane_id)
            .and_then(|pane| pane.terminal_view())
            .cloned()
        {
            view.update(cx, |view, _| {
                view.send_input(&[daruda_terminal::ansi::INTERRUPT])
            });
            return true;
        }
        self.cancel_agent_turn(pane_id, cx);
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |view, cx| view.clear_queue(cx));
        }
        true
    }
}

fn report_commit_error(ws: &WeakEntity<Workspace>, e: &TaskCommitError, cx: &mut gpui::AsyncApp) {
    let report = task_commit_report(e);
    let shown = ws.update(cx, |ws, cx| ws.report_error(report.clone(), cx));
    if shown.is_err() {
        LogWriter::log(report);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::strings::error;

    /// Each read-only cause has its own recovery, so each names it: a
    /// marker only a person can delete is never blamed on a restart, and
    /// a restart that would recover is never sent to hunt for a marker.
    #[test]
    fn a_read_only_task_log_names_its_own_cause() {
        let title = |cause| task_commit_report(&TaskCommitError::ReadOnly(cause)).title;
        assert_eq!(
            title(ReadOnlyCause::UnfinishedReconciliation),
            error::tasks_read_only()
        );
        assert_eq!(
            title(ReadOnlyCause::SettleFailed),
            error::tasks_settle_failed()
        );
        assert_eq!(
            title(ReadOnlyCause::SettleMarkerStuck),
            error::tasks_settle_marker_stuck()
        );
    }
}
