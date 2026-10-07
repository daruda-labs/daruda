//! Starting a task inside a lane that already exists — the
//! `TaskRunIn::ExistingLane` arm of `start_task`.
//!
//! Nothing touches git here: the lane is already checked out, so the task
//! only needs a new tab in it, rooted at the lane. An agent chat opens the
//! way a user's own does, so it keeps the lane's session host.

use std::path::Path;

use daruda_store::project::{LaneKind, LaneRef};
use daruda_store::tasks::{Task, TaskAgentSurface};
use gpui::{Context, Window};

use super::task_start::{TaskStartError, TaskStarted};
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_host::agent_chat_ops::resolve_open_agent_id;
use crate::workspace::main_area::pane_tree::PaneId;

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
        self.projects.iter().find_map(|project| {
            project
                .lanes
                .iter()
                .find(|lane| daruda_core::path::same_path(&lane.path, path))
                .map(|lane| LaneRef {
                    project: project.id,
                    lane: lane.id,
                })
        })
    }

    /// The lane a task being run again should stay in: the one its earlier
    /// run created, while still registered. Starting it as a new worktree
    /// would ask git for a branch that lane already has checked out.
    pub(super) fn own_lane_for_rerun(&self, task: &Task) -> Option<std::path::PathBuf> {
        let path = task.state.worktree_path()?;
        (self.lane_ref_at(path).is_some() && path.is_dir()).then(|| path.clone())
    }

    /// Open a tab for `task` in the lane at `path` and hand it the prompt.
    /// A lane that is gone or cannot host a pane leaves the task in
    /// Backlog, so the user can pick another one and start again.
    pub(super) fn start_task_in_existing_lane(
        &mut self,
        task: &Task,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<TaskStarted, TaskStartError> {
        let missing = || TaskStartError::LaneMissing {
            path: path.to_path_buf(),
        };
        let found = self
            .lane_ref_at(path)
            .and_then(|lane| Some((lane, self.lane_for(lane)?)))
            .filter(|_| path.is_dir())
            .map(|(lane, found)| (lane, found.path.clone(), lane_branch_label(found)));
        let (lane, root, branch) = found.ok_or_else(missing)?;
        self.activate_lane(lane, window, cx);
        if self.active_lane_is_inaccessible() {
            return Err(missing());
        }
        // `None` past the check above is a spawn failure, already reported.
        let pane = self
            .open_task_pane_in_active_lane(task.agent_surface, root.clone(), window, cx)
            .ok_or(TaskStartError::PaneUnavailable)?;
        self.dispatch_claude_for_task(&task.id, &root, &branch, pane, window, cx)?;
        Ok(TaskStarted {
            worktree: root,
            pane,
            surface: task.agent_surface,
        })
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
                let agent_id = resolve_open_agent_id(&self.agents, self.last_agent_id.as_deref());
                let cwds = self.active_lane_cwds();
                let pane_id = self.insert_agent_chat_pane(agent_id, cwds, window, cx)?;
                self.reveal_new_agent_chat_pane(pane_id, window, cx);
                Some(pane_id)
            }
        }
    }
}

#[cfg(test)]
#[path = "task_existing_lane_tests.rs"]
mod tests;
