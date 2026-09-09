//! Creating a worktree on behalf of the control surface.
//!
//! Its own file because it is the one control operation that cannot answer in
//! one turn of the event loop: `git worktree add` is blocking, so this is
//! plan → background git → register, the same three steps the task workflow
//! runs. `finalize_create_lane` owns the third one, so nothing here becomes a
//! second way to make a lane.
//!
//! The orchestration lives inside `crate::workspace` so `CreateWorktreePlan`
//! does not have to leave it; the dispatcher only sees a channel.

use gpui::{Context, Window};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::{LaneRef, ProjectId};

use crate::control::result::ControlError;
use crate::telegram::bridge::PaneRef;
use crate::workspace::Workspace;

/// What a finished creation reports: the worktree, and the chat that came with
/// it, so a caller can talk to the new agent without listing again.
pub(crate) type CreatedLane = Result<(LaneRef, PaneRef), ControlError>;

impl Workspace {
    /// Create a worktree in `project` and open a chat in it.
    ///
    /// Answers on the returned channel once the git work and the registration
    /// are both done. A caller that stops listening cannot stall it — the
    /// channel is bounded at one and the send is never awaited.
    pub(crate) fn control_create_lane(
        &mut self,
        project: ProjectId,
        name: String,
        base_ref: Option<String>,
        agent: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> smol::channel::Receiver<CreatedLane> {
        let (tx, rx) = smol::channel::bounded(1);
        let plan = match self.control_lane_plan(project, &name, base_ref) {
            Ok(plan) => plan,
            Err(e) => {
                let _ = tx.try_send(Err(e));
                return rx;
            }
        };
        // The same lock the task workflow takes. Two `git worktree add` runs
        // against one repo can leave a half-created checkout, and an agent
        // asking while a task-driven create is in flight is exactly that
        // case. Note the *create form* takes no lock today, so this does not
        // serialize against the user's own dialog — and the set is
        // per-`Workspace`, so it does not serialize across windows either.
        if !self.acquire_repo_lock(&plan.repo_root) {
            let _ = tx.try_send(Err(ControlError::LaneCreateBusy));
            return rx;
        }

        let weak = cx.weak_entity();
        window
            .spawn(cx, async move |async_cx| {
                let add = {
                    let plan = plan.clone();
                    async_cx
                        .background_executor()
                        .spawn(async move {
                            crate::lane::git::add_lane(
                                &plan.repo_root,
                                &plan.new_path,
                                Some(&plan.branch),
                                plan.base_ref.as_deref(),
                            )
                            .map_err(|e| e.to_string())
                        })
                        .await
                };

                let outcome = match async_cx.update(|window, app_cx| {
                    let Some(workspace) = weak.upgrade() else {
                        return Err(ControlError::TargetGone);
                    };
                    workspace.update(app_cx, |ws, cx| {
                        // The lock is released before finalizing, matching the
                        // task path: a half-created entry must not block the
                        // user's own next attempt on the same repo.
                        ws.release_repo_lock(&plan.repo_root);
                        match &add {
                            Err(detail) => {
                                ws.report_error(
                                    ErrorReport::new(
                                        crate::surface::strings::error_lane_create_failed(),
                                    )
                                    .severity(ErrorSeverity::Error)
                                    .at(file!(), line!())
                                    .with_context("detail", detail.clone())
                                    .dedup("control.lane_create")
                                    .build(),
                                    cx,
                                );
                                // git's message rides out with the code: the
                                // caller is a model that has to fix an
                                // argument, and the log it cannot read is no
                                // help to it.
                                Err(ControlError::LaneCreateFailed {
                                    detail: detail.clone(),
                                })
                            }
                            Ok(()) => {
                                ws.control_lane_finalize(plan.clone(), project, agent, window, cx)
                            }
                        }
                    })
                }) {
                    Ok(outcome) => outcome,
                    // The window is gone, so the checkout may be on disk with
                    // nothing knowing about it — the same documented outcome
                    // the task path has, cleaned up by `git worktree prune`.
                    Err(_) => Err(ControlError::TargetGone),
                };
                let _ = tx.try_send(outcome);
            })
            .detach();
        rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, TestAppContext};

    /// A project that is not open cannot be planned against, and the caller
    /// hears so immediately rather than after a git attempt.
    #[gpui::test]
    async fn creating_in_a_missing_project_refuses_before_touching_git(cx: &mut TestAppContext) {
        let fixture = crate::test_support::workspace_with_agent_chat(cx);
        let rx = cx
            .update_window(fixture.window.into(), |_, window, cx| {
                fixture.workspace.update(cx, |ws, cx| {
                    ws.control_create_lane(9_999, "x".into(), None, None, window, cx)
                })
            })
            .expect("window is live");
        assert_eq!(
            rx.recv().await,
            Ok(Err(ControlError::TargetGone)),
            "answered without spawning anything"
        );
    }
}
