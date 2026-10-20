//! Starting a task inside a lane that already exists — the
//! `TaskRunIn::ExistingLane` arm of `start_task`.
//!
//! Nothing touches git here: the lane is already checked out, so the task
//! only needs a new tab in it, rooted at the lane. An agent chat opens the
//! way a user's own does, so it keeps the lane's session host.

use std::path::Path;

use daruda_store::project::{LaneKind, LaneRef};
use daruda_store::tasks::{ExecutionRef, TaskAgentSurface, TaskRecord};
use gpui::{Context, Window};

use super::task_start::{TaskStartError, TaskStarted};
use crate::lane::availability::LaneAvailability;
use crate::runs::GlobalTasks;
use crate::workspace::Workspace;
use crate::workspace::lane_ops::LaneReveal;
use crate::workspace::main_area::pane::TabEntry;
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};

/// An existing lane a start was checked against, before it is recorded.
pub(super) struct ExistingLaneStart {
    lane: LaneRef,
    root: std::path::PathBuf,
    branch: String,
    reveal: LaneReveal,
}

/// What the prompt header names for a lane: its branch, else its label
/// (a plain directory or a detached HEAD has no branch to name).
pub(super) fn lane_branch_label(lane: &crate::lane::Lane) -> String {
    match &lane.kind {
        LaneKind::Git {
            branch: Some(branch),
            ..
        } => branch.clone(),
        _ => lane.display_name(),
    }
}

impl Workspace {
    /// The registered lane at `path`, in any project — tasks are shared by
    /// every window and project, so the owner need not be the active one.
    pub(in crate::workspace) fn lane_ref_at(&self, path: &Path) -> Option<LaneRef> {
        self.projects
            .lanes()
            .find(|(_, _, lane)| daruda_core::path::same_path(&lane.path, path))
            .map(|(target, _, _)| target)
    }

    /// The lane a task being run again should stay in: the one its earlier
    /// run created, while still registered. Starting it as a new worktree
    /// would ask git for a branch that lane already has checked out.
    pub(super) fn own_lane_for_rerun(&self, task: &TaskRecord) -> Option<std::path::PathBuf> {
        let path = task.lifecycle.worktree()?;
        (self.lane_ref_at(path).is_some() && path.is_dir()).then(|| path.to_path_buf())
    }

    /// Check the lane at `path` can host the task, before the start is
    /// recorded: a lane that is gone or cannot host a pane leaves the task
    /// in Backlog, so the user can pick another one and start again.
    /// Checked in place — a refused start does not switch lanes.
    pub(super) fn prepare_existing_lane_start(
        &mut self,
        path: &Path,
        reveal: LaneReveal,
    ) -> Result<ExistingLaneStart, TaskStartError> {
        let missing = || TaskStartError::LaneMissing {
            path: path.to_path_buf(),
        };
        let found = self
            .lane_ref_at(path)
            .and_then(|lane| Some((lane, self.lane_for(lane)?)))
            .filter(|_| path.is_dir())
            .map(|(lane, found)| (lane, found.path.clone(), lane_branch_label(found)));
        let (lane, root, branch) = found.ok_or_else(missing)?;
        // Against the live filesystem, as activating the lane would.
        self.recompute_availability_for(lane);
        if self
            .lane_for(lane)
            .is_none_or(|lane| lane.availability != LaneAvailability::Present)
        {
            return Err(missing());
        }
        let agent = self.open_agent_id(None);
        let launch = self.agent_launch_for(&agent);
        if self.lane_for(lane).is_some_and(|lane| {
            lane.remote_cwd.is_some()
                || lane
                    .session_host
                    .as_ref()
                    .is_some_and(daruda_store::project::LaneSessionHost::is_remote)
                || launch.as_ref().is_some_and(|launch| {
                    lane.effective_session_host(
                        &launch.launch,
                        &self.mirrors.session_hosts,
                        &self.mirrors.session_host_tombstones,
                    )
                    .is_ok_and(|host| host.is_remote())
                })
        }) {
            return Err(TaskStartError::RemoteLane);
        }
        Ok(ExistingLaneStart {
            lane,
            root,
            branch,
            reveal,
        })
    }

    /// Open a tab for the recorded run in its lane and hand it the prompt.
    pub(super) fn start_task_in_existing_lane(
        &mut self,
        exec: &ExecutionRef,
        start: ExistingLaneStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(TaskStarted, gpui::Task<Result<u64, TaskStartError>>), TaskStartError> {
        let ExistingLaneStart {
            lane,
            root,
            branch,
            reveal,
        } = start;
        let surface = cx
            .global::<GlobalTasks>()
            .get(&exec.task_id)
            .and_then(|task| match &task.lifecycle {
                daruda_store::tasks::Lifecycle::Preparing { source, .. } => Some(
                    if matches!(source, daruda_store::tasks::TaskExecutionSource::AgentChat) {
                        TaskAgentSurface::AgentChat
                    } else {
                        TaskAgentSurface::Terminal
                    },
                ),
                _ => None,
            })
            .ok_or(TaskStartError::NotFound)?;
        // The lane may have closed while the start was recorded.
        if self.lane_for(lane).is_none() {
            return Err(TaskStartError::LaneMissing { path: root });
        }
        // `None` past the check above is a spawn failure, already reported.
        let pane = match reveal {
            LaneReveal::Foreground => {
                self.activate_lane(lane, window, cx);
                self.open_task_pane_in_active_lane(surface, root.clone(), window, cx)
            }
            LaneReveal::Background => {
                self.open_task_pane_behind(lane, surface, root.clone(), window, cx)
            }
        }
        .ok_or(TaskStartError::PaneUnavailable)?;
        let dispatched = self.dispatch_claude_for_task(exec, &root, &branch, pane, window, cx)?;
        Ok((
            TaskStarted {
                exec: exec.clone(),
                worktree: root,
                pane,
                surface,
            },
            dispatched,
        ))
    }

    fn open_task_pane_in_active_lane(
        &mut self,
        surface: TaskAgentSurface,
        root: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        match surface {
            TaskAgentSurface::Terminal => self.insert_terminal_tab(Some(root), window, cx),
            TaskAgentSurface::AgentChat => {
                let agent_id = self.open_agent_id(None);
                let cwds = self.active_lane_cwds();
                let pane_id = self.insert_agent_chat_pane(agent_id, cwds, window, cx)?;
                self.reveal_new_agent_chat_pane(pane_id, window, cx);
                Some(pane_id)
            }
        }
    }

    /// Open the task's pane as a new tab of `lane` without showing it: that
    /// lane's selected tab and focus stay, and so does the window's active
    /// lane — which may be `lane` itself, with the orchestrator's tab shown.
    /// An Agent chat connects here, since no focus will connect it.
    fn open_task_pane_behind(
        &mut self,
        lane: LaneRef,
        surface: TaskAgentSurface,
        root: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        let pane = match surface {
            TaskAgentSurface::Terminal => match self.create_pane_at(Some(root), window, cx) {
                Ok(pane) => pane,
                Err(e) => {
                    self.report_pane_error(
                        &crate::surface::strings::error::pane_context_new_tab(),
                        e,
                        cx,
                    );
                    return None;
                }
            },
            TaskAgentSurface::AgentChat => {
                let agent_id = self.open_agent_id(None);
                let (local, remote, host) = self.lane_cwds(lane);
                self.create_new_agent_chat_pane(agent_id, local, remote, host, window, cx)
            }
        };
        let pane_id = pane.id;
        let tab_id = self.alloc_id();
        let rt = self.main_area.runtimes.entry(lane).or_default();
        let first = rt.tabs.is_empty();
        rt.panes.push(pane);
        rt.tabs.push(TabEntry {
            id: tab_id,
            layout: PaneLayout::Pane(pane_id),
            last_focused_pane: pane_id,
            user_label: None,
        });
        // A lane with nothing open shows this tab when the user goes there.
        if first {
            rt.active_tab_index = 0;
            rt.focused_pane_id = pane_id;
        }
        self.bump_activity(pane_id);
        if surface == TaskAgentSurface::AgentChat {
            self.maybe_connect_agent_chat(pane_id, cx);
        }
        self.mutate_durable(cx, |_, _| {});
        cx.notify();
        Some(pane_id)
    }
}

#[cfg(test)]
#[path = "task_existing_lane_tests.rs"]
mod tests;
