//! The window operations [`crate::runs::RunsHost`] asks for, installed once
//! as its [`RunsEffects`]: stopping a task's pane, a CLI tracker event, and a
//! flow run's change, each handed to whichever windows it concerns.

use daruda_store::tasks::ExecutionRef;
use gpui::App;

use crate::runs::{RunsEffects, RunsHost};
use crate::window_registry::WindowRegistry;
use crate::workspace::Workspace;

impl Workspace {
    pub(crate) fn install_task_effects(cx: &mut App) {
        RunsHost::install_effects(std::rc::Rc::new(WorkspaceEffects), cx);
    }
}

/// The run can be in any window: the one that asked first, then the rest.
struct WorkspaceEffects;

impl RunsEffects for WorkspaceEffects {
    fn run_event(&self, event: crate::runs::watch::RunEvent, cx: &mut App) {
        crate::control::notices::enqueue(event, cx);
    }

    fn locate_flow_run(
        &self,
        run_id: &str,
        cx: &mut App,
    ) -> Option<crate::runs::result::FlowRunLocation> {
        let mut found = None;
        WindowRegistry::for_each_workspace(cx, |ws, _window, _cx| {
            if found.is_none() {
                found = ws.projects.lanes().find_map(|(_, _, lane)| {
                    let run_dir = super::flow_paths::runs_dir(&lane.path).join(run_id);
                    run_dir
                        .exists()
                        .then(|| crate::runs::result::FlowRunLocation {
                            run_dir,
                            lock_dir: super::flow_paths::lane_lock_dir(&ws.lock_root, &lane.path),
                        })
                });
            }
        });
        found
    }

    fn flow_changed(&self, change: &crate::runs::FlowChange, cx: &mut App) {
        crate::workspace::flow_events::deliver_flow_change(change, cx);
    }

    fn cli_event(
        &self,
        pane: Option<crate::runs::RunPane>,
        event: daruda_agent::pty_tracker::PtyTrackerEvent,
        cx: &mut App,
    ) {
        WindowRegistry::for_each_workspace(cx, |ws, _window, cx| {
            use daruda_agent::pty_tracker::PtyTrackerEvent;
            let event = match &event {
                PtyTrackerEvent::BindingChanged { binding, .. } => {
                    let Some(pane) = pane.filter(|pane| pane.workspace == ws.uuid()) else {
                        return;
                    };
                    PtyTrackerEvent::BindingChanged {
                        pane_id: pane.pane,
                        binding: binding.clone(),
                    }
                }
                event @ PtyTrackerEvent::SessionProcessExited { .. } => event.clone(),
            };
            ws.apply_pty_tracker_event(event, cx);
        });
    }

    fn stop_execution(&self, exec: &ExecutionRef, cx: &mut App) {
        let mut stopped = false;
        WindowRegistry::for_each_workspace(cx, |ws, _window, cx| {
            if !stopped {
                stopped = ws.stop_task_execution(exec, cx);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gpui::TestAppContext;

    use super::*;

    /// A flow change reaches the effects with no window open, which is
    /// the state a run can end in.
    #[gpui::test]
    fn a_flow_change_with_no_window_open_is_delivered_to_none(cx: &mut TestAppContext) {
        cx.update(|cx| {
            RunsHost::ensure_installed(cx);
            Workspace::install_task_effects(cx);
            let dir = PathBuf::from("/nonexistent-daruda-flow-tree");
            let change = crate::runs::FlowChange {
                worktree: daruda_flow::lock::CanonicalTree::unchecked(dir),
                what: crate::runs::FlowChanged::Repainted,
            };
            WorkspaceEffects.flow_changed(&change, cx);
        });
    }
}
