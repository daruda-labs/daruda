//! Running a flow through the phone or orchestrator control surface.
//!
//! A flow answers to different rules than a chat: it is addressed by worktree
//! rather than by pane, only one may run in a worktree at a time, and a file
//! that declares profiles runs only with one of them named. Runtime permission
//! questions are supported on desktop and eligible remote channels; the first
//! answer wins.
//!
//! Listing is the exception that takes no target — every open window
//! contributes its active worktree's set and each row says which, so there is
//! nothing for a caller to name.

use std::path::Path;

use gpui::Context;

use crate::control::result::{
    ControlError, FlowEntry, FlowIssue, FlowNotStartedReason, FlowOriginKind, LaneHandle,
    StopDisposition,
};
use crate::workspace::Workspace;
use crate::workspace::flow_ops::FlowStartRefused;
use crate::workspace::flow_paths::{FlowOrigin, FoundFlow, flow_label, project_flows_dir};
use crate::workspace::flow_request::{FlowSelection, FlowSubmitError};
use daruda_control_types::runs::RunHandle;
use daruda_store::project::LaneRef;

/// A successful dispatch and the exact run it inserted.
#[derive(Debug, PartialEq)]
pub(crate) struct FlowStarted {
    pub entry: FlowEntry,
    pub run_id: String,
}

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
                        profiles: declared_profiles(&found.path),
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
    /// Every start refusal is returned to the caller. Runtime permission
    /// questions use the shared desktop/remote answer path. A worktree that
    /// is gone has nowhere to put a run; a file that declares profiles needs
    /// one of them named, and a name it does not declare is refused rather
    /// than run under the defaults; a lane runs one flow at a time, enforced
    /// by an on-disk lock the engine takes only after dispatch; and a flow
    /// that does not pass its own static checks is refused as written.
    pub(crate) fn control_flow_run(
        &mut self,
        lane: LaneRef,
        name: &str,
        profile: Option<&str>,
        started_by: daruda_flow::request::StartedBy,
        solicited: bool,
        cx: &mut Context<Self>,
    ) -> Result<FlowStarted, ControlError> {
        let Some(cwd) = self.lane_for(lane).map(|l| l.path.clone()) else {
            return Err(ControlError::TargetGone);
        };
        let found =
            self.resolve_flow_name_in(lane, name)
                .ok_or_else(|| ControlError::FlowNotFound {
                    name: name.to_string(),
                })?;
        let label = flow_label(&found.path);
        let profiles = declared_profiles(&found.path);
        match profile {
            None if !profiles.is_empty() => {
                return Err(ControlError::FlowNeedsInteraction { name: label });
            }
            Some(profile) if !profiles.iter().any(|p| p == profile) => {
                return Err(ControlError::FlowUnknownProfile {
                    name: label,
                    profile: profile.to_string(),
                });
            }
            _ => {}
        }
        // The engine takes the on-disk lock only once the run has started, so
        // a lane another process holds is refused here, where it is still an
        // answer rather than a run that ends at once.
        if self.lane_holder(&cwd).is_some() || self.is_flow_running_in(lane, cx) {
            return Err(ControlError::FlowLocked {
                name: label,
                held_by: self.flow_holder_in(lane, &cwd, cx),
            });
        }
        // Same check `Validate Flow…` runs: no lock, no run directory.
        match self.check_flow(lane, &found.path, profile, cx) {
            Ok(issues) if issues.is_empty() => {}
            Ok(_) => return Err(ControlError::FlowRefused { name: label }),
            Err(e) => {
                let held_by = self.flow_holder_in(lane, &cwd, cx);
                return Err(refusal_answer(label, held_by, FlowStartRefused::Submit(e)));
            }
        }
        let submitted = self.submit_flow_run(
            lane,
            &found.path,
            profile,
            &FlowSelection::default(),
            (started_by, solicited),
            cx,
        );
        let run_id = match submitted {
            Ok(run_id) => run_id,
            Err(refused) => {
                let held_by = self.flow_holder_in(lane, &cwd, cx);
                return Err(refusal_answer(label, held_by, refused));
            }
        };
        let entry = FlowEntry {
            name: label,
            origin: map_origin(found.origin),
            lane: LaneHandle::new(self.uuid(), lane),
            profiles,
        };
        Ok(FlowStarted { entry, run_id })
    }

    /// Every problem `text` would have as a flow of `lane`, found without
    /// saving or running it: the same check `Validate Flow…` runs, so a
    /// clean answer means the file would start there as written.
    ///
    /// `name` names an existing flow whose directory relative prompt files
    /// resolve against; without one the text is checked where a new flow is
    /// created. A refusal that comes from the worktree rather than the text
    /// (a remote agent, a launch that cannot be built) is reported as an
    /// issue too: the text will not run there either way.
    pub(crate) fn control_flow_validate(
        &mut self,
        lane: LaneRef,
        text: &str,
        profile: Option<&str>,
        name: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Result<Vec<FlowIssue>, ControlError> {
        if self.lane_for(lane).is_none() {
            return Err(ControlError::TargetGone);
        }
        let path = match name {
            Some(name) => {
                self.resolve_flow_name_in(lane, name)
                    .ok_or_else(|| ControlError::FlowNotFound {
                        name: name.to_string(),
                    })?
                    .path
            }
            None => {
                let root = self
                    .project_for(lane.project)
                    .map(|p| p.root.clone())
                    .ok_or(ControlError::TargetGone)?;
                project_flows_dir(&self.data_dir, &root).join(DRAFT_FLOW_FILE)
            }
        };
        let issues = match self.check_flow_text(lane, text, &path, profile, cx) {
            Ok(issues) => issues,
            Err(FlowSubmitError::Load(daruda_flow::FlowError::Validate(issues))) => issues,
            Err(FlowSubmitError::Invalid(issues)) => issues,
            Err(FlowSubmitError::NoLane | FlowSubmitError::LaneUnresolvable { .. }) => {
                return Err(ControlError::TargetGone);
            }
            Err(other) => {
                return Ok(vec![FlowIssue {
                    node: None,
                    message: submit_problem(other),
                }]);
            }
        };
        Ok(issues
            .into_iter()
            .map(|issue| FlowIssue {
                node: issue.node.map(|node| node.to_string()),
                message: issue.message,
            })
            .collect())
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
        requested: Option<&daruda_control_types::runs::RunHandle>,
        cx: &mut Context<Self>,
    ) -> Result<StopDisposition, ControlError> {
        if self.lane_for(lane).is_none() {
            return Err(ControlError::TargetGone);
        }
        if let Some(requested) = requested
            && self.hosted_flow_handle_in(lane, cx).as_ref() != Some(requested)
        {
            return Err(ControlError::RunMismatch);
        }
        if !self.is_flow_running_in(lane, cx) {
            return Ok(StopDisposition::AlreadyIdle);
        }
        self.stop_flow_run_in(lane, cx);
        Ok(StopDisposition::Stopped)
    }

    /// The handle of the run this process hosts in `lane`, the same one
    /// `daruda_flow_run` answered with.
    fn hosted_flow_handle_in(&self, lane: LaneRef, cx: &Context<Self>) -> Option<RunHandle> {
        let run = self.flow_run_in(lane, cx)?;
        flow_handle(run.view().run_dir().file_name()?.to_str()?)
    }

    /// The run holding `lane`'s worktree: one this process hosts, else the
    /// live lock holder another process left. `None` when neither names one.
    fn flow_holder_in(&self, lane: LaneRef, cwd: &Path, cx: &Context<Self>) -> Option<RunHandle> {
        self.hosted_flow_handle_in(lane, cx).or_else(|| {
            self.lane_holder(cwd)
                .and_then(|holder| flow_handle(&holder.run_id))
        })
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

/// A run id as the handle a control caller passes back; an id the handle
/// grammar rejects (an unreadable lock's empty one) names nothing.
fn flow_handle(run_id: &str) -> Option<RunHandle> {
    format!("flow:{run_id}").parse().ok()
}

/// A start's refusal as the control caller's answer. A problem in the file
/// is `FlowRefused`, a held worktree `FlowLocked`; anything else names its
/// reason as a code, since this caller sees no toast and the toast's wording
/// is localized desktop text that an MCP result must not carry.
fn refusal_answer(
    name: String,
    held_by: Option<RunHandle>,
    refused: FlowStartRefused,
) -> ControlError {
    let error = match refused {
        FlowStartRefused::AlreadyRunning => return ControlError::FlowLocked { name, held_by },
        FlowStartRefused::Submit(error) => error,
    };
    let reason = match error {
        FlowSubmitError::Load(_) | FlowSubmitError::Invalid(_) => {
            return ControlError::FlowRefused { name };
        }
        FlowSubmitError::LockHeld { .. } => return ControlError::FlowLocked { name, held_by },
        FlowSubmitError::NoLane => FlowNotStartedReason::NoLane,
        FlowSubmitError::LaneUnresolvable { .. } => FlowNotStartedReason::LaneUnresolvable,
        FlowSubmitError::RemoteLane { agent } => FlowNotStartedReason::RemoteLane { agent },
        FlowSubmitError::AgentLaunchRefused { agent, .. } => {
            FlowNotStartedReason::AgentLaunchRefused { agent }
        }
        FlowSubmitError::UnusableSessionHost { agent, .. } => {
            FlowNotStartedReason::SessionHostUnusable { agent }
        }
        FlowSubmitError::Read { .. } => FlowNotStartedReason::ReadFailed,
        FlowSubmitError::Resume(_) => FlowNotStartedReason::ResumeRefused,
    };
    ControlError::FlowNotStarted { name, reason }
}

/// The file name unsaved text is checked under. Only its directory matters:
/// that is what the text's relative prompt files resolve against.
const DRAFT_FLOW_FILE: &str = "draft.yaml";

/// A check's refusal that is not a list of issues, in the fixed English an
/// MCP answer carries. Paths stay in the desktop's log.
fn submit_problem(error: FlowSubmitError) -> String {
    let reason = match error {
        FlowSubmitError::Load(error) => return error.to_string(),
        FlowSubmitError::RemoteLane { agent } => FlowNotStartedReason::RemoteLane { agent },
        FlowSubmitError::AgentLaunchRefused { agent, .. } => {
            FlowNotStartedReason::AgentLaunchRefused { agent }
        }
        FlowSubmitError::UnusableSessionHost { agent, .. } => {
            FlowNotStartedReason::SessionHostUnusable { agent }
        }
        FlowSubmitError::NoLane => FlowNotStartedReason::NoLane,
        FlowSubmitError::LaneUnresolvable { .. } => FlowNotStartedReason::LaneUnresolvable,
        FlowSubmitError::Read { .. } => FlowNotStartedReason::ReadFailed,
        FlowSubmitError::Resume(_) => FlowNotStartedReason::ResumeRefused,
        FlowSubmitError::LockHeld { .. } => return "worktree is locked by another run".into(),
        FlowSubmitError::Invalid(issues) => {
            return format!("the flow has {} validation problem(s)", issues.len());
        }
    };
    reason.to_string()
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

/// The profiles the file at `path` declares, in the order the desktop picker
/// offers them.
///
/// A file that cannot be read or parsed answers none: the run that follows
/// fails on the same read and names it properly, and refusing here would
/// report a parse error as a profile problem.
fn declared_profiles(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| daruda_flow::load::profiles(&text).ok())
        .unwrap_or_default()
}
