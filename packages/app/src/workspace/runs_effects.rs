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
        RunsHost::install_effects(std::rc::Rc::new(WorkspaceEffects::default()), cx);
    }
}

/// The run can be in any window: the one that asked first, then the rest.
#[derive(Default)]
struct WorkspaceEffects {
    unlocated: std::cell::RefCell<Unlocated>,
}

/// How long a run id no open lane holds is trusted to stay absent. A wait
/// refreshes every few seconds; probing every lane's disk each time is waste.
const UNLOCATED_RETRY: std::time::Duration = std::time::Duration::from_secs(30);

/// Run ids recently probed and not found, with the open lanes they were probed in.
#[derive(Default)]
struct Unlocated(std::collections::HashMap<String, (u64, std::time::Instant)>);

impl Unlocated {
    /// Probe again once the open lanes change or the miss has aged out.
    fn should_probe(&mut self, id: &str, lanes: u64, now: std::time::Instant) -> bool {
        self.0
            .retain(|_, (_, at)| now.saturating_duration_since(*at) < UNLOCATED_RETRY);
        self.0.get(id).is_none_or(|(seen, _)| *seen != lanes)
    }

    fn record(&mut self, id: &str, lanes: u64, now: std::time::Instant, found: bool) {
        if found {
            self.0.remove(id);
        } else {
            self.0.insert(id.to_owned(), (lanes, now));
        }
    }
}

fn fingerprint(lanes: &[(std::path::PathBuf, std::path::PathBuf)]) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    lanes.hash(&mut hasher);
    hasher.finish()
}

impl RunsEffects for WorkspaceEffects {
    fn run_event(&self, event: crate::runs::watch::RunEvent, cx: &mut App) {
        crate::control::notices::enqueue(event, cx);
    }

    fn locate_flow_run(
        &self,
        run_id: &str,
        cx: &mut App,
    ) -> Option<crate::runs::result::FlowRunLocation> {
        let mut lanes = Vec::new();
        WindowRegistry::for_each_workspace(cx, |ws, _window, _cx| {
            lanes.extend(
                ws.projects
                    .lanes()
                    .map(|(_, _, lane)| (lane.path.clone(), ws.lock_root.clone())),
            );
        });
        let now = std::time::Instant::now();
        let seen = fingerprint(&lanes);
        if !self.unlocated.borrow_mut().should_probe(run_id, seen, now) {
            return None;
        }
        let found = lanes.into_iter().find_map(|(path, lock_root)| {
            let run_dir = super::flow_paths::runs_dir(&path).join(run_id);
            run_dir
                .exists()
                .then(|| crate::runs::result::FlowRunLocation {
                    run_dir,
                    lock_dir: super::flow_paths::lane_lock_dir(&lock_root, &path),
                })
        });
        self.unlocated
            .borrow_mut()
            .record(run_id, seen, now, found.is_some());
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

    #[test]
    fn an_unlocated_run_is_probed_again_only_after_lanes_change_or_time_passes() {
        let mut unlocated = Unlocated::default();
        let start = std::time::Instant::now();
        assert!(unlocated.should_probe("run", 1, start));
        unlocated.record("run", 1, start, false);
        assert!(!unlocated.should_probe("run", 1, start + std::time::Duration::from_secs(2)));
        assert!(
            unlocated.should_probe("run", 2, start),
            "a lane opened or closed"
        );
        assert!(unlocated.should_probe("other", 1, start));
        assert!(unlocated.should_probe("run", 1, start + UNLOCATED_RETRY));
        assert!(unlocated.0.is_empty(), "aged misses are forgotten");
        unlocated.record("run", 1, start, false);
        unlocated.record("run", 1, start, true);
        assert!(unlocated.should_probe("run", 1, start));
    }

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
            WorkspaceEffects::default().flow_changed(&change, cx);
        });
    }
}
