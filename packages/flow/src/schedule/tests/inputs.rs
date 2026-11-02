//! A run's input values: rendered into what each node is handed, recorded
//! when the run starts, and rendered the same again by a resume that is
//! given none.

use super::resume_steps::{as_if_killed, resume_request};
use super::*;
use crate::input::InputValue;
use crate::request::validate_request;

const REVIEW: &str = "\
version: 1
inputs:
  pr: { type: number, required: true }
  focus: { type: choice, choices: [security, clarity], default: clarity }
nodes:
  - id: fetch
    kind: command
    run: gh pr diff {{input.pr}}
  - id: report
    kind: command
    deps: [fetch]
    run: echo {{input.focus}} {{input.pr}}
";

#[test]
fn a_resume_renders_the_values_the_run_started_with() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut request = request_for(REVIEW, dir.path());
    request
        .inputs
        .insert("pr".into(), InputValue::Text("1234".into()));

    let runner = FakeRunner::new().cancel_at("report", 1);
    let first = execute(&request, &runner, &CancelToken::default());
    let texts: Vec<_> = runner.calls().into_iter().map(|call| call.text).collect();
    assert_eq!(texts[0], "gh pr diff 1234");
    assert_eq!(texts[1], "echo clarity 1234", "the default filled in");

    let recorded = crate::run_log::reader::read(&first.run_dir).inputs;
    assert_eq!(recorded["pr"].as_text(), "1234");
    assert_eq!(recorded["focus"].as_text(), "clarity");

    as_if_killed(&request, &first.run_dir);
    let resumed = resume_request(REVIEW, dir.path(), &first.run_dir).expect("resumable");
    assert!(resumed.inputs.is_empty(), "a resume is handed no values");
    assert!(validate_request(&resumed).is_empty());
    let runner = FakeRunner::new();
    execute(&resumed, &runner, &CancelToken::default());
    let texts: Vec<_> = runner.calls().into_iter().map(|call| call.text).collect();
    assert_eq!(texts, ["echo clarity 1234"]);
}

/// A log written before inputs existed replays as a run with none.
#[test]
fn a_log_from_before_inputs_replays_with_none() {
    let legacy = serde_json::json!({
        "type": "run_started",
        "nodes": [],
    });
    let event: crate::run_log::machine::FlowRunEvent =
        serde_json::from_value(legacy).expect("reads");
    let replay = crate::run_log::reader::replay_events([&event]);
    assert!(replay.inputs.is_empty());
}
