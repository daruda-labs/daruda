//! What a control command answers with.
//!
//! Carries data, never rendered sentences: an adapter for a human runs these
//! through `crate::surface::strings`, an adapter for an agent serializes them
//! as-is. A rendered string here would force one of those two to un-render it.
//!
//! GPUI-free.

use serde::{Deserialize, Serialize};

use crate::telegram::bridge::PaneRef;

/// A pane's live agent activity.
///
/// Deliberately a separate type from `AgentChatView`'s `ActivityState`, which
/// is `pub(in crate::workspace)`. Widening that visibility would break the
/// encapsulation `scripts/lint-agent-activity.sh` protects; the single
/// conversion lives in `crate::workspace::control_ops`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Activity {
    Idle,
    Working,
    AwaitingPermission,
}

/// Whether the pane can be talked to at all. Distinct from [`Activity`] —
/// a pane with no session is neither working nor meaningfully idle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Health {
    Ok,
    Error,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ChatSummary {
    pub target: PaneRef,
    pub is_active_lane: bool,
    pub activity: Activity,
    pub health: Health,
    /// The lane this pane sits in has activity the user has not looked at.
    /// Lane-scoped because that is where daruda tracks the signal.
    pub unread: bool,
    /// The agent-authored session title, flattened to one bounded line by
    /// [`crate::control::agent_text::sanitize_title`] at construction — it is
    /// agent-authored and otherwise unbounded, so every consumer, not just a
    /// phone screen, gets it capped.
    /// `None` = a session that has not titled itself yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LaneGroup {
    pub name: String,
    pub chats: Vec<ChatSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProjectGroup {
    pub name: String,
    pub lanes: Vec<LaneGroup>,
}

/// Windows are the outermost group because the same project can be open in
/// two of them; without this axis their rows are indistinguishable by name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WindowGroup {
    pub index: u32,
    pub projects: Vec<ProjectGroup>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Listing {
    pub windows: Vec<WindowGroup>,
    /// Rows the *adapter* dropped to fit its own message budget. The executor
    /// never truncates, so it always reports `0`; the field exists so a
    /// rendered listing can say how much it left out without inventing a
    /// second channel for it.
    pub omitted: u32,
}

/// A prompt that reached a busy pane waits behind the turn in flight. Saying
/// only "sent" would read as a hang on the phone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SendDisposition {
    Delivered,
    Queued,
    /// The text was a local slash command the pane handled itself (`/clear`),
    /// so it never reached the agent. An `Ok` variant because it is a normal
    /// outcome — and a named one because `/clear` *resets the session*, which
    /// a reply saying "sent" would hide.
    HandledLocally,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopDisposition {
    Stopped,
    AlreadyIdle,
}

/// What a prompt sent with `daruda_chat_ask` came back with.
///
/// An enum rather than an `Option<String>` beside a flag: "said nothing",
/// "not finished", "went behind another turn" and "failed" are four different
/// things a caller acts on differently, and a text field plus a boolean can
/// spell combinations none of them mean.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum PaneAnswer {
    /// What the turn this prompt started ended up saying.
    Text { text: String },
    /// The turn finished without producing any text — all tool calls.
    NoAnswer,
    /// The turn ended in an error. Whatever it managed to say is in the chat;
    /// this call has no answer to give.
    Failed,
    /// Somebody stopped the turn — a person at the desk, or another surface.
    /// Its own state rather than `Text`: the transcript *does* hold whatever
    /// had been written by then, and handing that back as the reply would
    /// present a cut-off sentence as the agent's answer.
    Interrupted,
    /// The prompt went behind a turn already in flight, so this call is not
    /// the one that will see its reply. Read it later.
    Queued,
    /// The turn outlived the wait and is still going. Not a failure — read it
    /// later.
    StillWorking,
}

/// What became of a `/daruda` prompt. The answer arrives later from the
/// orchestrator pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AskDisposition {
    /// This request started the orchestrator, so the answer waits on its ACP
    /// handshake too.
    Connecting,
    /// On the wire now.
    Sent,
    /// Behind a turn the orchestrator already had in flight.
    Queued,
    /// The pane handled a local slash command, so no agent answer will follow.
    HandledLocally,
}

/// Collapse orchestrator startup and prompt delivery into one phone-facing
/// status.
pub(crate) fn ask_disposition(connecting: bool, send: SendDisposition) -> AskDisposition {
    match (send, connecting) {
        (SendDisposition::HandledLocally, _) => AskDisposition::HandledLocally,
        (_, true) => AskDisposition::Connecting,
        (SendDisposition::Delivered, false) => AskDisposition::Sent,
        (SendDisposition::Queued, false) => AskDisposition::Queued,
    }
}

/// Which of the three flow directories a name resolved to. Mirrors
/// `workspace::flow_paths::FlowOrigin`, which is `pub(in crate::workspace)`;
/// the single conversion lives in `crate::workspace::control_ops`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FlowOriginKind {
    Repo,
    Project,
    Global,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FlowEntry {
    /// The file name as it is on disk, so an answer can say which of two
    /// same-stem files (`ship.yaml` / `ship.yml`) it means.
    pub name: String,
    pub origin: FlowOriginKind,
    /// The worktree this flow belongs to.
    ///
    /// A flow is lane-scoped and every open window contributes its active
    /// worktree's set, so a bare name does not say *where* it would run. Two
    /// windows on different repositories can both offer `ship.yaml` and mean
    /// different things.
    pub lane: LaneHandle,
}

/// A worktree, addressed across the whole process.
///
/// Window-qualified for the reason [`PaneRef`] is: `ProjectId` and `LaneId`
/// are monotonic *per workspace*, so `{ project: 0, lane: 0 }` exists in every
/// open window. Without the uuid, two windows holding the same repository
/// produce listing rows a caller cannot tell apart — and a command built from
/// one of them would reach both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct LaneHandle {
    pub workspace: daruda_store::project::WorkspaceUuid,
    pub project: daruda_store::project::ProjectId,
    /// Named `worktree` on the wire, matching what daruda has always
    /// persisted and what [`daruda_store::project::LaneRef`] serializes.
    #[serde(rename = "worktree", alias = "lane")]
    pub lane: daruda_store::project::LaneId,
}

impl LaneHandle {
    /// The window-local half, for a caller that has already resolved which
    /// workspace this names.
    pub(crate) fn lane_ref(self) -> daruda_store::project::LaneRef {
        daruda_store::project::LaneRef {
            project: self.project,
            lane: self.lane,
        }
    }

    /// Qualify a window-local ref with the workspace that owns it.
    pub(crate) fn new(
        workspace: daruda_store::project::WorkspaceUuid,
        target: daruda_store::project::LaneRef,
    ) -> Self {
        Self {
            workspace,
            project: target.project,
            lane: target.lane,
        }
    }
}

/// One worktree, as `daruda_worktree_list` reports it.
///
/// Carries the handle plus enough context to choose between rows without a
/// second call: which project it belongs to, whether it is the one on screen,
/// and how many chats are already in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LaneEntry {
    pub target: LaneHandle,
    /// Display name of the owning project.
    pub project: String,
    pub name: String,
    pub is_active: bool,
    /// Open agent-chat panes in this worktree.
    pub chats: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BriefSummary {
    pub working: u32,
    pub awaiting_permission: u32,
    pub error: u32,
    /// Open agent-chat panes across every window.
    pub total: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ControlResult {
    Listing(Listing),
    /// `None` = the selection was cleared.
    Selected {
        target: Option<ChatSummary>,
    },
    Sent {
        target: PaneRef,
        disposition: SendDisposition,
    },
    Stopped {
        target: PaneRef,
        disposition: StopDisposition,
    },
    /// A struct variant, not a newtype: `#[serde(tag = "kind")]` cannot
    /// serialize a tagged newtype wrapping a sequence, and this type's whole
    /// purpose is to be serialized by an adapter that speaks to a machine.
    FlowList {
        flows: Vec<FlowEntry>,
    },
    /// The start was *requested* and nothing refused it — not that the run is
    /// under way. The engine takes its lock inside the worker thread it was
    /// just handed, so a run another process locks in between still fails, and
    /// reports that failure asynchronously. Naming this `FlowStarted` would
    /// promise something the caller cannot know yet.
    FlowStarting {
        name: String,
        origin: FlowOriginKind,
        /// Which worktree it started in. The command names no target, so this
        /// is the only place the answer says where the run landed.
        lane: LaneHandle,
    },
    /// A run the caller asked to end. `AlreadyIdle` is an `Ok`: nothing was
    /// running, which is the state a stop was asking for.
    FlowStopped {
        lane: LaneHandle,
        disposition: StopDisposition,
    },
    Brief(BriefSummary),
    /// `/daruda` was accepted. The reply arrives later from the orchestrator
    /// pane.
    Accepted {
        disposition: AskDisposition,
    },
    LaneListing {
        lanes: Vec<LaneEntry>,
    },
    /// A worktree and the chat pane that came with it — both handles, so the
    /// caller can talk to the new agent without listing again.
    LaneCreated {
        target: LaneHandle,
        chat: PaneRef,
    },
    ChatCreated {
        target: PaneRef,
    },
    /// A prompt sent with the reply waited for. Distinct from `Sent`, which
    /// reports only that a prompt went out.
    Answer {
        target: PaneRef,
        answer: PaneAnswer,
    },
    /// What one chat's agent last said, bounded by
    /// [`crate::control::agent_text::bound_agent_text`].
    ///
    /// `None` is a pane whose transcript holds no assistant text — a turn that
    /// was all tool calls, or a session that has not answered yet. A distinct
    /// answer from an empty string, which would read as "it said nothing" when
    /// the truth is "it has not said anything *yet*".
    Transcript {
        target: PaneRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
}

/// A command that cannot run. A branch the caller must take in the normal
/// course — a queued prompt, an already-idle pane — is an `Ok` variant with a
/// disposition, not an error.
///
/// A *parse* failure has no variant here on purpose. It never reached the
/// executor, so giving it one would mint a code no executor can produce; the
/// adapter renders `spec::ParseError` directly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub(crate) enum ControlError {
    OrdinalNotFound {
        ordinal: u32,
    },
    NoTargetSelected,
    TargetGone,
    FlowNotFound {
        name: String,
    },
    FlowLocked {
        name: String,
    },
    /// The flow will not run as written — the same static verdict
    /// `Validate Flow…` gives. Reported before dispatch so the phone is never
    /// told a run started that the desktop then refused in a toast.
    FlowRefused {
        name: String,
    },
    /// The flow is valid and nothing refused it up front, but no run exists
    /// afterwards — so something between the two said no. Deliberately does
    /// not guess *what*: at this point the static check has passed, so naming
    /// the file would send the caller to inspect something that is fine.
    FlowNotStarted {
        name: String,
    },
    /// The flow would open a desktop dialog the phone cannot answer — an
    /// agent node whose permission policy is `ask`, or a file declaring
    /// profiles, which asks which one to run.
    FlowNeedsInteraction {
        name: String,
    },
    NoActiveLane,
    /// `[orchestrator] enabled = false` — nothing to hand the prompt to.
    OrchestratorDisabled,
    /// Switched on, but the settings name no runnable agent. A separate code
    /// from `Disabled` because the fix is a different one.
    OrchestratorUnresolvable,
    /// Configured and resolvable, but it would not come up. Deliberately does
    /// not carry the reason: it is an internal failure the log holds, not
    /// something the caller can act on.
    OrchestratorUnavailable,
    /// The user said no to a tool that creates something.
    ApprovalRefused,
    /// The card went unanswered. Distinct from a refusal: nobody decided, so
    /// asking again later is reasonable where retrying a refusal is not.
    ApprovalTimedOut,
    /// The agent has created as many worktrees as it may. Refused before the
    /// card, since there is nothing to ask about.
    AgentLimitReached,
    /// The addressed pane already holds as many queued prompts as it may.
    QueueFull,
    /// The orchestrator addressed its own pane, which would make a turn that
    /// makes a turn.
    SelfTargetRefused,
    /// The worktree could not be created. Reported as its own code rather than
    /// as `TargetGone`, which would send the caller looking for a target that
    /// is there — the branch name, the base ref or the checkout path is the
    /// problem.
    ///
    /// `detail` carries git's own words (`LC_ALL=C`, so not the user's locale),
    /// because the caller here is usually a model whose only recovery is to
    /// change an argument and which cannot read the log. Without it, "creation
    /// failed" is indistinguishable from a refusal, and a model told only that
    /// invents reasons.
    LaneCreateFailed {
        detail: String,
    },
    /// The name is not a usable branch name. Refused by `guard_gated`, before
    /// the approval card, since a name git will reject is not worth a tap.
    ///
    /// A name that is *usable* but already taken by an existing branch is a
    /// different case and still costs a tap: recognising it needs a `git`
    /// call, which the pre-card guards are synchronous and so cannot make.
    /// That one comes back as [`Self::LaneCreateFailed`] with git's words.
    LaneNameInvalid,
    /// daruda could not ask the user — the Telegram bridge is off, unpaired,
    /// or has no token. Distinct from a refusal *and* from a timeout: nobody
    /// declined and nobody failed to answer, the question never went out.
    ApprovalUnavailable,
    /// Too many approval cards are already waiting. Retryable once the user
    /// has worked through them, unlike the refusals above.
    ApprovalsPending,
    /// Another worktree creation is already in flight for that repository.
    /// Two `git worktree add` runs against one repo can leave a half-created
    /// checkout, so the second one waits its turn rather than racing.
    LaneCreateBusy,
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Diagnostic only — the user-facing wording lives in the adapters,
        // which run these through `crate::surface::strings`.
        match self {
            Self::OrdinalNotFound { ordinal } => write!(f, "no chat at {ordinal}"),
            Self::NoTargetSelected => write!(f, "no target selected"),
            Self::TargetGone => write!(f, "target no longer exists"),
            Self::FlowNotFound { name } => write!(f, "no flow named {name}"),
            Self::FlowLocked { name } => write!(f, "flow already running: {name}"),
            Self::FlowRefused { name } => write!(f, "flow will not run as written: {name}"),
            Self::FlowNotStarted { name } => write!(f, "flow did not start: {name}"),
            Self::FlowNeedsInteraction { name } => write!(f, "flow needs desktop input: {name}"),
            Self::NoActiveLane => write!(f, "no active lane"),
            Self::OrchestratorDisabled => write!(f, "orchestrator is disabled"),
            Self::OrchestratorUnresolvable => write!(f, "orchestrator names no runnable agent"),
            Self::OrchestratorUnavailable => write!(f, "orchestrator would not start"),
            Self::ApprovalRefused => write!(f, "the user refused"),
            Self::ApprovalTimedOut => write!(f, "the approval went unanswered"),
            Self::AgentLimitReached => write!(f, "agent worktree budget spent"),
            Self::QueueFull => write!(f, "prompt queue full"),
            Self::SelfTargetRefused => write!(f, "an agent may not prompt itself"),
            Self::LaneCreateFailed { detail } => {
                write!(f, "worktree creation failed: {detail}")
            }
            Self::LaneNameInvalid => write!(f, "not a usable branch name"),
            Self::ApprovalUnavailable => write!(f, "no way to ask the user"),
            Self::ApprovalsPending => write!(f, "too many approvals already waiting"),
            Self::LaneCreateBusy => write!(f, "a worktree is already being created there"),
        }
    }
}

pub(crate) type ControlOutcome = Result<ControlResult, ControlError>;

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::project::WorkspaceUuid;

    fn sample_lane() -> LaneEntry {
        LaneEntry {
            target: LaneHandle {
                workspace: WorkspaceUuid::new(),
                project: 1,
                lane: 2,
            },
            project: "daruda".into(),
            name: "fix-picker".into(),
            is_active: true,
            chats: 2,
        }
    }

    fn sample_summary() -> ChatSummary {
        ChatSummary {
            target: PaneRef {
                workspace: WorkspaceUuid::new(),
                pane: 7,
            },
            is_active_lane: true,
            activity: Activity::AwaitingPermission,
            health: Health::Ok,
            unread: true,
            title: Some("restore picker invariants".into()),
            last_activity: Some(1_757_000_000),
        }
    }

    #[test]
    fn listing_round_trips_through_json() {
        let listing = Listing {
            windows: vec![WindowGroup {
                index: 0,
                projects: vec![ProjectGroup {
                    name: "daruda".into(),
                    lanes: vec![LaneGroup {
                        name: "fix-picker".into(),
                        chats: vec![sample_summary()],
                    }],
                }],
            }],
            omitted: 0,
        };
        let json = serde_json::to_string(&listing).expect("serialize");
        let back: Listing = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, listing);
    }

    #[test]
    fn result_round_trips_through_json() {
        let result = ControlResult::Brief(BriefSummary {
            working: 2,
            awaiting_permission: 1,
            error: 0,
            total: 5,
        });
        let json = serde_json::to_string(&result).expect("serialize");
        let back: ControlResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, result);
    }

    #[test]
    fn missing_last_activity_is_absent_not_zero() {
        let mut s = sample_summary();
        s.last_activity = None;
        let json = serde_json::to_string(&s).expect("serialize");
        assert!(
            !json.contains("last_activity"),
            "None must not serialize as 0: {json}"
        );
    }

    /// Every variant, because `#[serde(tag = "kind")]` rejects some shapes at
    /// *runtime* rather than at compile time — a tagged newtype wrapping a
    /// sequence is the one that bit us. A test covering one variant proves
    /// nothing about the others.
    #[test]
    fn every_result_variant_round_trips_through_json() {
        let target = sample_summary().target;
        let cases = [
            ControlResult::Listing(Listing {
                windows: Vec::new(),
                omitted: 3,
            }),
            ControlResult::Selected { target: None },
            ControlResult::Selected {
                target: Some(sample_summary()),
            },
            ControlResult::Sent {
                target,
                disposition: SendDisposition::Queued,
            },
            ControlResult::Sent {
                target,
                disposition: SendDisposition::HandledLocally,
            },
            ControlResult::Stopped {
                target,
                disposition: StopDisposition::Stopped,
            },
            ControlResult::FlowList {
                flows: vec![FlowEntry {
                    name: "ship.yaml".into(),
                    origin: FlowOriginKind::Repo,
                    lane: sample_lane().target,
                }],
            },
            ControlResult::FlowStarting {
                name: "ship.yaml".into(),
                origin: FlowOriginKind::Global,
                lane: sample_lane().target,
            },
            ControlResult::FlowStopped {
                lane: sample_lane().target,
                disposition: StopDisposition::Stopped,
            },
            ControlResult::FlowStopped {
                lane: sample_lane().target,
                disposition: StopDisposition::AlreadyIdle,
            },
            ControlResult::Brief(BriefSummary {
                working: 1,
                awaiting_permission: 2,
                error: 3,
                total: 6,
            }),
            ControlResult::Accepted {
                disposition: AskDisposition::Connecting,
            },
            ControlResult::Accepted {
                disposition: AskDisposition::Sent,
            },
            ControlResult::Accepted {
                disposition: AskDisposition::Queued,
            },
            ControlResult::Accepted {
                disposition: AskDisposition::HandledLocally,
            },
            ControlResult::LaneListing { lanes: Vec::new() },
            ControlResult::LaneListing {
                lanes: vec![sample_lane()],
            },
            ControlResult::LaneCreated {
                target: sample_lane().target,
                chat: target,
            },
            ControlResult::ChatCreated { target },
            ControlResult::Answer {
                target,
                answer: PaneAnswer::Text {
                    text: "the reply".into(),
                },
            },
            ControlResult::Answer {
                target,
                answer: PaneAnswer::NoAnswer,
            },
            ControlResult::Answer {
                target,
                answer: PaneAnswer::Failed,
            },
            ControlResult::Answer {
                target,
                answer: PaneAnswer::Interrupted,
            },
            ControlResult::Answer {
                target,
                answer: PaneAnswer::Queued,
            },
            ControlResult::Answer {
                target,
                answer: PaneAnswer::StillWorking,
            },
            ControlResult::Transcript { target, text: None },
            ControlResult::Transcript {
                target,
                text: Some("the answer".into()),
            },
        ];
        for case in cases {
            let json =
                serde_json::to_string(&case).unwrap_or_else(|e| panic!("serialize {case:?}: {e}"));
            let back: ControlResult =
                serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize {json}: {e}"));
            assert_eq!(back, case);
        }
    }

    /// Same reasoning as above, for the failure half of the contract.
    #[test]
    fn every_error_variant_round_trips_through_json() {
        let cases = [
            ControlError::OrdinalNotFound { ordinal: 4 },
            ControlError::NoTargetSelected,
            ControlError::TargetGone,
            ControlError::FlowNotFound {
                name: "nope".into(),
            },
            ControlError::FlowLocked {
                name: "ship.yaml".into(),
            },
            ControlError::FlowRefused {
                name: "ship.yaml".into(),
            },
            ControlError::FlowNotStarted {
                name: "ship.yaml".into(),
            },
            ControlError::FlowNeedsInteraction {
                name: "ship.yaml".into(),
            },
            ControlError::NoActiveLane,
            ControlError::OrchestratorDisabled,
            ControlError::OrchestratorUnresolvable,
            ControlError::OrchestratorUnavailable,
            ControlError::ApprovalRefused,
            ControlError::ApprovalTimedOut,
            ControlError::AgentLimitReached,
            ControlError::QueueFull,
            ControlError::SelfTargetRefused,
            ControlError::LaneCreateBusy,
            ControlError::LaneCreateFailed {
                detail: "fatal: a branch named 'main' already exists".into(),
            },
            ControlError::LaneNameInvalid,
            ControlError::ApprovalUnavailable,
            ControlError::ApprovalsPending,
        ];
        for case in cases {
            let json =
                serde_json::to_string(&case).unwrap_or_else(|e| panic!("serialize {case:?}: {e}"));
            let back: ControlError =
                serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize {json}: {e}"));
            assert_eq!(back, case);
            assert!(!case.to_string().is_empty(), "every error has a diagnostic");
        }
    }

    #[test]
    fn ask_disposition_ranks_no_answer_over_a_cold_start() {
        use AskDisposition as A;
        use SendDisposition as S;
        assert_eq!(ask_disposition(true, S::Queued), A::Connecting);
        assert_eq!(ask_disposition(true, S::Delivered), A::Connecting);
        assert_eq!(ask_disposition(true, S::HandledLocally), A::HandledLocally);
        assert_eq!(ask_disposition(false, S::Delivered), A::Sent);
        assert_eq!(ask_disposition(false, S::Queued), A::Queued);
        assert_eq!(ask_disposition(false, S::HandledLocally), A::HandledLocally);
    }

    /// The wire key stays `worktree`, and both spellings deserialize — a
    /// caller copying a listing row back must not have to translate it.
    #[test]
    fn a_lane_handle_serializes_with_the_persisted_key_name() {
        let handle = LaneHandle {
            workspace: WorkspaceUuid::new(),
            project: 1,
            lane: 2,
        };
        let json = serde_json::to_value(handle).expect("serialize");
        assert_eq!(json["worktree"], 2);
        assert!(json.get("lane").is_none());
        assert_eq!(
            serde_json::from_value::<LaneHandle>(json).expect("deserialize"),
            handle
        );

        let aliased = serde_json::json!({
            "workspace": handle.workspace,
            "project": 1,
            "lane": 2,
        });
        assert_eq!(
            serde_json::from_value::<LaneHandle>(aliased).expect("alias"),
            handle
        );
    }

    /// The whole point of the type: two windows produce distinguishable
    /// handles for the same window-local ref.
    #[test]
    fn two_windows_produce_different_handles_for_the_same_local_ref() {
        let local = daruda_store::project::LaneRef {
            project: 0,
            lane: 0,
        };
        let a = LaneHandle::new(WorkspaceUuid::new(), local);
        let b = LaneHandle::new(WorkspaceUuid::new(), local);
        assert_ne!(a, b);
        assert_eq!(a.lane_ref(), b.lane_ref(), "the local halves do match");
    }

    #[test]
    fn error_round_trips_through_json() {
        let error = ControlError::FlowNeedsInteraction {
            name: "deploy.yaml".into(),
        };
        let json = serde_json::to_string(&error).expect("serialize");
        let back: ControlError = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, error);
    }
}
