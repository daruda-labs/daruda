//! What a control command answers with.
//!
//! Carries data, never rendered sentences: an adapter for a human runs these
//! through `crate::surface::strings`, an adapter for an agent serializes them
//! as-is. A rendered string here would force one of those two to un-render it.
//!
//! GPUI-free.

use serde::{Deserialize, Serialize};

use crate::telegram::bridge::PaneRef;

/// A session title is agent-authored and unbounded. Capped here, at the one
/// place a [`ChatSummary`] is built, so the bound is a property of the type
/// rather than of one adapter's renderer.
const TITLE_MAX_CHARS: usize = 80;

/// Flatten an agent-authored title to one bounded line. Control characters
/// become spaces before the whitespace run is collapsed, so a multi-line title
/// cannot break the row it is rendered on, and no consumer has to defend
/// against one.
pub(crate) fn sanitize_title(raw: &str) -> Option<String> {
    let clean: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(TITLE_MAX_CHARS)
        .collect();
    (!clean.is_empty()).then_some(clean)
}

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
    /// [`sanitize_title`] at construction — it is agent-authored and otherwise
    /// unbounded, so every consumer, not just a phone screen, gets it capped.
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
    },
    Brief(BriefSummary),
    /// `/daruda` was accepted. The reply arrives later from the orchestrator
    /// pane.
    Accepted {
        disposition: AskDisposition,
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
        }
    }
}

pub(crate) type ControlOutcome = Result<ControlResult, ControlError>;

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::project::WorkspaceUuid;

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
                }],
            },
            ControlResult::FlowStarting {
                name: "ship.yaml".into(),
                origin: FlowOriginKind::Global,
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
    fn a_title_is_capped_and_stripped_of_control_characters() {
        let raw = format!("line one\nline two\t{}", "x".repeat(200));
        let clean = sanitize_title(&raw).expect("non-empty");
        assert_eq!(clean.chars().count(), TITLE_MAX_CHARS);
        assert!(!clean.contains('\n') && !clean.contains('\t'));
    }

    #[test]
    fn a_short_title_is_untouched_and_a_blank_one_is_absent() {
        assert_eq!(
            sanitize_title("restore invariants").as_deref(),
            Some("restore invariants")
        );
        assert_eq!(sanitize_title("   \n\t  "), None);
        assert_eq!(sanitize_title(""), None);
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
