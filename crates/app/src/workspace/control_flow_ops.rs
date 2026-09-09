//! Running a flow from outside the app.
//!
//! A flow answers to different rules than a chat: it is addressed by worktree
//! rather than by pane, only one may run in a worktree at a time, and one that
//! would stop to ask a question has nowhere to ask it from a phone.
//!
//! Listing is the exception that takes no target — every open window
//! contributes its active worktree's set and each row says which, so there is
//! nothing for a caller to name.

use std::path::Path;

use gpui::Context;

use crate::control::result::{
    ControlError, FlowEntry, FlowOriginKind, LaneHandle, StopDisposition,
};
use crate::workspace::Workspace;
use crate::workspace::flow_paths::{FlowOrigin, FoundFlow, flow_label};
use crate::workspace::flow_request::FlowSelection;
use daruda_store::project::LaneRef;

impl Workspace {
    /// Every flow the active lane can run, in the order the flows panel shows
    /// them (by file name, whatever scope each came from).
    ///
    /// `FlowSources::list_flows` already decided which file a name resolves
    /// to — the repository's copy shadows the person's — so this list holds
    /// one entry per name and an ambiguous name is unrepresentable.
    pub(crate) fn control_flow_list(&self) -> Vec<FlowEntry> {
        self.flow_sources()
            .map(|sources| {
                sources
                    .list_flows()
                    .into_iter()
                    .map(|found| FlowEntry {
                        name: flow_label(&found.path),
                        origin: map_origin(found.origin),
                        lane: LaneHandle::new(self.uuid(), self.active),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Start `name` in `lane`.
    ///
    /// The worktree is the caller's to name — every other targeted command on
    /// this surface names its target, and a run that read the active lane
    /// instead made the answer the first place the caller learned where it
    /// landed.
    ///
    /// Every refusal is taken *before* dispatch, because a refusal after it
    /// surfaces as a desktop toast the caller never sees and would leave a
    /// phone told a run started that never did. A worktree that is gone has
    /// nowhere to put a run; one that would open a desktop dialog (an `ask`
    /// permission policy, or a profile question) would hang a caller that
    /// cannot answer it; a lane runs one flow at a time, enforced by an
    /// on-disk lock; and a flow that does not pass its own static checks would
    /// be refused on submit.
    pub(crate) fn control_flow_run(
        &mut self,
        lane: LaneRef,
        name: &str,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Result<FlowEntry, ControlError> {
        let Some(cwd) = self.lane_for(lane).map(|l| l.path.clone()) else {
            return Err(ControlError::TargetGone);
        };
        let found =
            self.resolve_flow_name_in(lane, name)
                .ok_or_else(|| ControlError::FlowNotFound {
                    name: name.to_string(),
                })?;
        let label = flow_label(&found.path);
        if needs_desktop_answer(&found.path) {
            return Err(ControlError::FlowNeedsInteraction { name: label });
        }
        // Both halves of "a run is already going": the on-disk lock catches
        // another process, and `runs` catches this one. Asking only the lock
        // leaves the guard inside `run_flow_at` to answer the second case by
        // opening its "stop it?" picker — a desktop dialog raised by a phone
        // command, which is the one thing this surface must never do.
        if self.lane_holder(&cwd).is_some() || self.runs.is_running(lane) {
            return Err(ControlError::FlowLocked { name: label });
        }
        // Everything the run would refuse *after* dispatch surfaces on the
        // desktop as a toast, which a phone never sees — so the static verdict
        // is taken here, where it can still become an answer. Same check
        // `Validate Flow…` runs: no lock, no run directory.
        let runnable = matches!(
            self.check_flow(lane, &found.path, None, cx),
            Ok(issues) if issues.is_empty()
        );
        if !runnable {
            return Err(ControlError::FlowRefused { name: label });
        }

        let entry = FlowEntry {
            name: label,
            origin: map_origin(found.origin),
            lane: LaneHandle::new(self.uuid(), lane),
        };
        // The guard reads the lock again, so a run that took it in between is
        // still caught — and answered, rather than reported as started.
        if !self.run_flow_at(
            lane,
            &found.path,
            crate::workspace::command::flow_picker::FlowPurpose::Run,
            FlowSelection::default(),
            window,
            cx,
        ) {
            return Err(ControlError::FlowLocked { name: entry.name });
        }
        // The guard passing does not mean a run exists: the request builder
        // can still refuse after it — an unusable session host, an account
        // directory it cannot prepare. Marking the run is how that is
        // detected: no run, nothing to mark.
        //
        // WORKAROUND: `submit_flow_run` discards its typed `FlowSubmitError`
        // into `report_flow_refusal`, so the reason only ever reaches the
        // screen. Surfacing it would restructure the flow host's dispatch, a
        // separate subsystem — until then the answer says what is known.
        //
        // Marked after dispatch rather than threaded into the submission: the
        // run is inserted synchronously and nothing here awaits, so the event
        // pump cannot retire it in between.
        if !self.runs.answer_telegram_on_end(lane) {
            return Err(ControlError::FlowNotStarted { name: entry.name });
        }
        Ok(entry)
    }

    /// Stop the run `lane` holds.
    ///
    /// Open, not gated: a stop is how a runaway is ended, and asking a person
    /// to approve one is the wrong way round. `AlreadyIdle` rather than an
    /// error for a worktree with nothing running — the caller asked for a
    /// state, and it is already in it.
    pub(crate) fn control_flow_stop(
        &mut self,
        lane: LaneRef,
        cx: &mut Context<Self>,
    ) -> Result<StopDisposition, ControlError> {
        if self.lane_for(lane).is_none() {
            return Err(ControlError::TargetGone);
        }
        if !self.runs.is_running(lane) {
            return Ok(StopDisposition::AlreadyIdle);
        }
        self.stop_flow_run_in(lane, cx);
        Ok(StopDisposition::Stopped)
    }

    /// This window's active worktree, if it can run `name`.
    ///
    /// For a text adapter's default target. Goes through the same resolution
    /// [`Self::control_flow_run`] uses, so the worktree a bare stem picks and
    /// the run that follows cannot disagree about which file that was.
    pub(crate) fn control_active_lane_offers(&self, name: &str) -> Option<LaneHandle> {
        self.resolve_flow_name_in(self.active, name)
            .map(|_| LaneHandle::new(self.uuid(), self.active))
    }

    /// The flow `name` points at *in `lane`*, accepting either the file name
    /// as written on disk or its stem — a phone keyboard should not have to
    /// type `.yaml`.
    fn resolve_flow_name_in(&self, lane: LaneRef, name: &str) -> Option<FoundFlow> {
        let sources = self.flow_sources_for(lane)?;
        sources.list_flows().into_iter().find(|found| {
            let file = found.path.file_name().map(|n| n.to_string_lossy());
            let stem = found.path.file_stem().map(|n| n.to_string_lossy());
            file.as_deref() == Some(name) || stem.as_deref() == Some(name)
        })
    }
}

/// The single conversion between the workspace-private flow origin and the
/// control surface's own enum, for the same reason [`map_activity`] exists.
fn map_origin(origin: FlowOrigin) -> FlowOriginKind {
    match origin {
        FlowOrigin::Repo => FlowOriginKind::Repo,
        FlowOrigin::Project => FlowOriginKind::Project,
        FlowOrigin::Global => FlowOriginKind::Global,
    }
}

/// Would running this flow put a dialog on the desktop that only a person at
/// the machine can answer? Two shapes do: an agent node (or the repair agent)
/// whose permission policy is `ask`, and a file declaring profiles, which
/// makes the picker ask which one to run.
///
/// A file that cannot be read or parsed answers `false`: the run that follows
/// fails on the same read and names it properly, and refusing here would
/// report a parse error as an interaction problem.
fn needs_desktop_answer(path: &Path) -> bool {
    use daruda_flow::model::{NodeKind, PermissionPolicy};

    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    if daruda_flow::load::profiles(&text).is_ok_and(|p| !p.is_empty()) {
        return true;
    }
    let Ok(inspected) = daruda_flow::inspect(&text, None) else {
        return false;
    };
    let flow = inspected.loaded.flow();
    let node_asks = flow.nodes.iter().any(|node| {
        matches!(&node.kind, NodeKind::Agent(body) if body.agent.permission == PermissionPolicy::Ask)
    });
    let repair_asks = flow
        .default_agent
        .as_ref()
        .is_some_and(|agent| agent.permission == PermissionPolicy::Ask);
    node_asks || repair_asks
}
