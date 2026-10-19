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
            handle: "flow:run-1".parse().unwrap(),
            name: "ship.yaml".into(),
            origin: FlowOriginKind::Global,
            lane: sample_lane().target,
        },
        ControlResult::TaskStarted {
            task: "task-1".into(),
            handle: "task:task-1/exec-1".parse().unwrap(),
            lane: sample_lane().target,
            chat: Some(target),
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
            answer: PaneAnswer::Interrupted {
                cause: InterruptCause::User,
            },
        },
        ControlResult::Answer {
            target,
            answer: PaneAnswer::Interrupted {
                cause: InterruptCause::ExtraUsage {
                    resets_at: Some(1_791_340_200),
                },
            },
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
        ControlError::RunMismatch,
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
        ControlError::TaskNotRecorded {
            detail: "the task log is read-only".into(),
        },
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

/// An `interrupted` written before the cause existed, or by a caller that
/// leaves it out, still reads as a person's stop; the auto-stop's shape is
/// what the MCP tool description tells the model to look for.
#[test]
fn an_interrupted_answer_names_its_cause_on_the_wire() {
    let legacy: PaneAnswer =
        serde_json::from_str(r#"{"state":"interrupted"}"#).expect("legacy shape reads");
    assert_eq!(
        legacy,
        PaneAnswer::Interrupted {
            cause: InterruptCause::User
        }
    );
    let json = serde_json::to_value(PaneAnswer::Interrupted {
        cause: InterruptCause::ExtraUsage {
            resets_at: Some(1_791_340_200),
        },
    })
    .expect("serialize");
    assert_eq!(
        json,
        serde_json::json!({
            "state": "interrupted",
            "cause": { "by": "extra_usage", "resets_at": 1_791_340_200 }
        })
    );
}

/// Every status keeps its snake_case wire name, so a phone or an agent
/// reading an older answer sees the same words.
#[test]
fn every_task_status_round_trips_under_its_wire_name() {
    let cases = [
        (TaskStatus::Backlog, "backlog"),
        (TaskStatus::Preparing, "preparing"),
        (TaskStatus::Dispatching, "dispatching"),
        (TaskStatus::Running, "running"),
        (TaskStatus::Done, "done"),
        (TaskStatus::Error, "error"),
        (TaskStatus::Cancelled, "cancelled"),
        (TaskStatus::Undelivered, "undelivered"),
        (TaskStatus::Interrupted, "interrupted"),
    ];
    for (status, wire) in cases {
        let json = serde_json::to_string(&status).expect("serialize");
        assert_eq!(json, format!("\"{wire}\""));
        let back: TaskStatus = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, status);
    }
}

/// Each lifecycle phase has its own external status.
#[test]
fn a_task_status_names_each_lifecycle() {
    use daruda_store::tasks::machine::{InterruptCause, UndeliveredReason};
    use daruda_store::tasks::{ExecutionRef, Lifecycle};
    let exec = ExecutionRef {
        task_id: "t".into(),
        execution_id: "r".into(),
    };
    let worktree = std::path::PathBuf::from("/wt");
    let cases = [
        (Lifecycle::Backlog, TaskStatus::Backlog),
        (
            Lifecycle::Preparing {
                exec: exec.clone(),
                source: Default::default(),
            },
            TaskStatus::Preparing,
        ),
        (
            Lifecycle::Dispatching {
                exec: exec.clone(),
                worktree: worktree.clone(),
            },
            TaskStatus::Dispatching,
        ),
        (
            Lifecycle::Undelivered {
                exec: exec.clone(),
                worktree: None,
                reason: UndeliveredReason::QueueFull,
            },
            TaskStatus::Undelivered,
        ),
        (
            Lifecycle::Interrupted {
                exec: Some(exec),
                worktree,
                cause: InterruptCause::UserStop,
            },
            TaskStatus::Interrupted,
        ),
    ];
    for (lifecycle, status) in cases {
        assert_eq!(TaskStatus::of(&lifecycle), status, "{lifecycle:?}");
    }
}
