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
        agent: "codex-acp".into(),
        agent_name: "Codex".into(),
        is_active_lane: true,
        activity: Activity::AwaitingPermission,
        health: Health::Ok,
        unread: true,
        title: Some("restore picker invariants".into()),
        last_activity: Some(1_757_000_000),
        tab_name: None,
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
            agent: "claude".into(),
        },
        ControlResult::ChatCreated {
            target,
            agent: "codex-acp".into(),
        },
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
