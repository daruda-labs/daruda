//! The words a task row, its menu and its error dialog use for where a run
//! stands and why it ended. The task log records causes; these name them.

use daruda_store::tasks::machine::{ErrorCause, InterruptCause, StartFailure, UndeliveredReason};
use daruda_store::tasks::{Lifecycle, SessionEndReason, TASK_TOOL_USE_FAILURE_THRESHOLD};
use gpui::SharedString;

use crate::surface::strings;

/// Why a run failed or never reached its agent; `None` for any other state.
pub(in crate::workspace) fn failure_text(lifecycle: &Lifecycle) -> Option<String> {
    match lifecycle {
        Lifecycle::Error { cause, .. } => Some(error_text(cause)),
        Lifecycle::Undelivered { reason, .. } => Some(undelivered_text(*reason)),
        _ => None,
    }
}

/// The row's status line.
pub(super) fn state_label(lifecycle: &Lifecycle) -> SharedString {
    SharedString::from(match lifecycle {
        Lifecycle::Backlog => strings::terminal::task_backlog(),
        Lifecycle::Preparing { .. } | Lifecycle::Dispatching { .. } => {
            strings::task::state_starting()
        }
        Lifecycle::Running { .. } => strings::terminal::task_running(),
        Lifecycle::Done { end_reason, .. } => format!(
            "{} ({})",
            strings::terminal::task_done_prefix(),
            done_flavour_label(*end_reason),
        ),
        Lifecycle::Error { cause, .. } => format!(
            "{}: {}",
            strings::terminal::task_error_prefix(),
            error_text(cause),
        ),
        Lifecycle::Cancelled { .. } => strings::terminal::task_cancelled(),
        Lifecycle::Undelivered { reason, .. } => format!(
            "{}: {}",
            strings::task::state_undelivered(),
            undelivered_text(*reason),
        ),
        Lifecycle::Interrupted { cause, .. } => format!(
            "{} ({})",
            strings::task::state_interrupted(),
            interrupt_text(*cause),
        ),
    })
}

fn done_flavour_label(reason: SessionEndReason) -> String {
    match reason {
        SessionEndReason::Stop => strings::task::done_flavour_stop(),
        SessionEndReason::PromptInputExit => strings::task::done_flavour_prompt_input_exit(),
        SessionEndReason::Logout => strings::task::done_flavour_logout(),
        SessionEndReason::Other => strings::task::done_flavour_other(),
        // `Error` settles as the `Error` state, not `Done`; reaching here
        // means an imported row — fall back to "Other".
        SessionEndReason::Error => strings::task::done_flavour_other(),
    }
}

fn error_text(cause: &ErrorCause) -> String {
    match cause {
        ErrorCause::StartFailed(failure) => start_failure_text(*failure),
        ErrorCause::SessionFailed => strings::task::error_session_failed(),
        ErrorCause::ToolFailures => {
            strings::task::error_tool_use_failure(TASK_TOOL_USE_FAILURE_THRESHOLD)
        }
        ErrorCause::Imported { message } => message.clone(),
    }
}

fn start_failure_text(failure: StartFailure) -> String {
    use strings::task as t;
    match failure {
        StartFailure::NotFound | StartFailure::NotBacklog => t::error_start_failed(),
        StartFailure::ProjectNotOpen => t::error_start_project_not_open(),
        StartFailure::NoGitRepo => t::error_start_no_git_repo(),
        StartFailure::RepoBusy => t::error_start_repo_busy(),
        StartFailure::LaneMissing => t::error_start_lane_missing(),
        StartFailure::PaneUnavailable => t::error_start_pane_unavailable(),
        StartFailure::GitAddFailed => t::error_start_git_add_failed(),
        StartFailure::FinalizeFailed => t::error_start_finalize_failed(),
        StartFailure::PromptUndelivered => t::error_prompt_undelivered(),
        StartFailure::WindowClosed => t::error_start_window_closed(),
        StartFailure::AgentMissing => t::error_start_agent_missing(),
        StartFailure::ConnectUnavailable => t::error_start_connect_failed(),
        StartFailure::AccountUnavailable => t::error_start_account_unavailable(),
    }
}

fn undelivered_text(reason: UndeliveredReason) -> String {
    use strings::task as t;
    match reason {
        UndeliveredReason::QueueFull => t::undelivered_queue_full(),
        UndeliveredReason::ConnectFailed => t::undelivered_connect_failed(),
        UndeliveredReason::SessionFailed => t::undelivered_session_failed(),
        UndeliveredReason::SessionTornDown => t::undelivered_session_ended(),
        UndeliveredReason::RemovedByUser => t::undelivered_removed(),
        UndeliveredReason::PaneClosed => t::undelivered_pane_closed(),
        UndeliveredReason::StartFailed => t::undelivered_start_failed(),
        UndeliveredReason::PtyWriteFailed => t::undelivered_pty_write_failed(),
        UndeliveredReason::AppRestart => t::undelivered_app_restart(),
        UndeliveredReason::Orphaned => t::undelivered_orphaned(),
    }
}

fn interrupt_text(cause: InterruptCause) -> String {
    use strings::task as t;
    match cause {
        InterruptCause::UserStop => t::interrupted_user_stop(),
        InterruptCause::ExtraUsage => t::interrupted_extra_usage(),
        InterruptCause::LaneGone => t::error_lane_gone(),
        InterruptCause::AppRestart => t::interrupted_app_restart(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::tasks::ExecutionRef;

    fn exec() -> ExecutionRef {
        ExecutionRef {
            task_id: "t".into(),
            execution_id: "r".into(),
        }
    }

    #[test]
    fn only_a_failed_or_undelivered_run_has_failure_text() {
        let undelivered = Lifecycle::Undelivered {
            exec: exec(),
            worktree: None,
            reason: UndeliveredReason::PaneClosed,
        };
        assert_eq!(
            failure_text(&undelivered),
            Some(strings::task::undelivered_pane_closed())
        );
        let interrupted = Lifecycle::Interrupted {
            exec: Some(exec()),
            worktree: "/wt".into(),
            cause: InterruptCause::UserStop,
        };
        assert_eq!(failure_text(&interrupted), None);
        assert!(state_label(&interrupted).contains(&strings::task::state_interrupted()));
    }

    #[test]
    fn an_imported_error_keeps_its_own_words() {
        let imported = Lifecycle::Error {
            exec: None,
            worktree: None,
            cause: ErrorCause::Imported {
                message: "old words".into(),
            },
        };
        assert_eq!(failure_text(&imported).as_deref(), Some("old words"));
    }
}
