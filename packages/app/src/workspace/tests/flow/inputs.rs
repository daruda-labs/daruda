//! A flow's declared inputs, from each way a run is started.

use super::*;
use crate::control::result::{ControlError, FlowInputKind, FlowInputProblem, FlowInputProblemKind};
use crate::workspace::flow_inputs_modal::PendingFlowRun;
use crate::workspace::flow_request::FlowSelection;

/// A command that writes the values it was rendered with into the lane.
const WITH_INPUTS: &str = "\
version: 1
inputs:
  pr:
    type: number
    description: The pull request
    required: true
  focus:
    type: choice
    choices: [speed, clarity]
    default: clarity
nodes:
  - id: check
    kind: command
    run: \"echo {{input.pr}} {{input.focus}} > seen.txt\"
";

/// What the run's command wrote, once it has.
async fn seen(cx: &mut TestAppContext, lane: &std::path::Path) -> String {
    let file = lane.join("seen.txt");
    assert!(
        wait_for_run(cx, |_| std::fs::read_to_string(&file)
            .is_ok_and(|text| text.ends_with('\n')))
        .await,
        "the run wrote its values"
    );
    let text = std::fs::read_to_string(&file).unwrap();
    // Let the run settle before the test ends with its thread.
    assert!(wait_for_run(cx, |cx| !run_is_held(cx, lane)).await);
    text
}

/// A desktop start asks for the values before anything runs.
#[gpui::test]
async fn a_desktop_start_asks_for_the_inputs_first(cx: &mut TestAppContext) {
    let (lane, ws, flow_path, wh) = workspace_with_a_flow(cx, WITH_INPUTS);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        assert!(ws.run_flow_at(
            ws.active,
            &flow_path,
            crate::workspace::command::flow_picker::FlowPurpose::Run,
            FlowSelection::default(),
            window,
            cx,
        ));
    });
    vcx.run_until_parked();
    assert!(has_dialog(&mut vcx), "the inputs form is up");
    assert!(
        !run_is_held(&mut vcx, lane.path()),
        "nothing runs until it is answered"
    );
}

/// The values the form hands on are the ones the run renders.
#[gpui::test]
async fn the_given_values_reach_the_run(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (lane, ws, flow_path, wh) = workspace_with_a_flow(cx, WITH_INPUTS);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let pending = PendingFlowRun {
                lane: ws.active,
                path: flow_path.clone(),
                profile: None,
                selection: FlowSelection {
                    inputs: Some(
                        [(
                            "pr".to_string(),
                            daruda_flow::input::InputValue::Text("1234".into()),
                        )]
                        .into(),
                    ),
                    ..FlowSelection::default()
                },
            };
            ws.run_flow_with_inputs(pending, window, cx);
        });
    })
    .expect("window is live");
    assert_eq!(seen(cx, lane.path()).await, "1234 clarity\n");
}

/// The listing names each input the way a caller needs it to give a value.
#[gpui::test]
async fn a_listed_flow_names_its_inputs(cx: &mut TestAppContext) {
    let (_lane, ws, _path, _wh) = workspace_with_a_flow(cx, WITH_INPUTS);
    ws.read_with(cx, |ws, _| {
        let flows = ws.control_flow_list();
        let inputs = &flows.iter().find(|e| e.name == "ship.yaml").unwrap().inputs;
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[0].name, "pr");
        assert_eq!(inputs[0].kind, FlowInputKind::Number);
        assert!(inputs[0].required);
        assert_eq!(inputs[0].description.as_deref(), Some("The pull request"));
        assert_eq!(inputs[1].kind, FlowInputKind::Choice);
        assert_eq!(inputs[1].default.as_deref(), Some("clarity"));
        assert_eq!(inputs[1].choices, ["speed", "clarity"]);
    });
}

/// A control start is refused, each problem named, until its values satisfy
/// the declarations; then it runs with them.
#[gpui::test]
async fn a_control_start_names_what_its_values_lack(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (lane, ws, _path, wh) = workspace_with_a_flow(cx, WITH_INPUTS);
    let start = |inputs: &[(&str, &str)], cx: &mut TestAppContext| {
        let inputs = inputs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        cx.update_window(wh.into(), |_, _, cx| {
            ws.update(cx, |ws, cx| {
                ws.control_flow_run(
                    ws.active,
                    "ship",
                    None,
                    &inputs,
                    (daruda_flow::request::StartedBy::Orchestrator, true),
                    cx,
                )
                .map(|started| started.run_id)
            })
        })
        .expect("window is live")
    };
    assert_eq!(
        start(&[("focus", "taste"), ("branch", "main")], cx),
        Err(ControlError::FlowInputsRefused {
            name: "ship.yaml".into(),
            problems: vec![
                FlowInputProblem {
                    input: "branch".into(),
                    problem: FlowInputProblemKind::NotDeclared,
                },
                FlowInputProblem {
                    input: "pr".into(),
                    problem: FlowInputProblemKind::Missing,
                },
                FlowInputProblem {
                    input: "focus".into(),
                    problem: FlowInputProblemKind::Invalid,
                },
            ],
        })
    );
    assert!(!run_is_held(cx, lane.path()));
    start(&[("pr", "77"), ("focus", "speed")], cx).expect("the values satisfy the flow");
    assert_eq!(seen(cx, lane.path()).await, "77 speed\n");
}
