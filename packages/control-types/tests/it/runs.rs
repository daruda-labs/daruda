use daruda_control_types::runs::RunHandle;

#[test]
fn handles_round_trip_as_strings_including_latest_task_alias() {
    for value in [
        "flow:1929fcd-a1-5",
        "task:legacy-task/execution_1",
        "task:latest",
    ] {
        let handle: RunHandle = value.parse().expect("valid handle");
        assert_eq!(handle.to_string(), value);
        let encoded = serde_json::to_value(&handle).expect("serialize");
        assert_eq!(encoded, value);
        assert_eq!(
            serde_json::from_value::<RunHandle>(encoded).unwrap(),
            handle
        );
    }
}

#[test]
fn malformed_handles_are_rejected_including_serde() {
    for value in [
        "",
        "flow:",
        "task:",
        "task:a/",
        "task:/a",
        "task:a/b/c",
        "flow:a/b",
        "flow:..",
        "task:../b",
        "task:a/..",
        "flow:a:b",
        "task:a\\b",
        "flow: a",
        "flow:a\n",
        "task:a/\t",
        "Flow:a",
        "other:a",
    ] {
        assert!(value.parse::<RunHandle>().is_err(), "accepted {value:?}");
        assert!(serde_json::from_value::<RunHandle>(serde_json::json!(value)).is_err());
    }
}

#[test]
fn result_states_round_trip_without_losing_ending_or_pointers() {
    use daruda_control_types::runs::{EndedAs, Pointers, Progress, RunResult, RunState, WaitingOn};
    let mut states = vec![
        RunState::Unknown,
        RunState::Gone,
        RunState::Unreadable {
            reason: "corrupt log".into(),
        },
        RunState::Running {
            progress: Progress {
                elapsed_secs: 42,
                ..Default::default()
            },
            waiting_on: Some(WaitingOn::Question {
                node: "review".into(),
            }),
        },
        RunState::Running {
            progress: Progress::default(),
            waiting_on: Some(WaitingOn::Permission {
                tool: Some("shell".into()),
            }),
        },
    ];
    for ended_as in [
        EndedAs::Completed,
        EndedAs::Failed,
        EndedAs::Canceled,
        EndedAs::Stalled,
        EndedAs::Crashed,
        EndedAs::Interrupted,
        EndedAs::Undelivered,
        EndedAs::Error,
        EndedAs::BudgetExhausted,
    ] {
        states.push(RunState::Ended {
            ended_as,
            text: Some("answer".into()),
            pointers: Pointers {
                run_dir: Some("/run".into()),
                report: Some("/run/run.md".into()),
                outputs: vec!["/run/out.md".into()],
                chat: Some("chat".into()),
                transcript: Some("/chat.jsonl".into()),
            },
        });
    }
    for state in states {
        let result = RunResult {
            handle: "task:t/e".parse().unwrap(),
            what: "Task".into(),
            why: Some("Prompt".into()),
            state,
        };
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(value["handle"], "task:t/e");
        assert_eq!(serde_json::from_value::<RunResult>(value).unwrap(), result);
    }
}

#[test]
fn normal_wait_outcomes_round_trip_with_every_partition() {
    use daruda_control_types::runs::{WaitAnswer, WaitOutcome};
    for outcome in [
        WaitOutcome::Settled,
        WaitOutcome::TimedOut,
        WaitOutcome::Nothing,
    ] {
        let answer = WaitAnswer {
            outcome,
            ended: vec![],
            blocked: vec![],
            already_reported: vec![],
            not_running: vec![],
            pending: vec![],
            more: 4,
        };
        let value = serde_json::to_value(&answer).unwrap();
        assert_eq!(value["more"], 4);
        assert_eq!(serde_json::from_value::<WaitAnswer>(value).unwrap(), answer);
    }
}
