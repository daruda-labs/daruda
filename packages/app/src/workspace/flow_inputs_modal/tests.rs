use daruda_flow::input::{InputValue, InputValues};
use gpui::TestAppContext;

use super::{FlowInputsModal, PendingFlowRun};
use crate::workspace::flow_request::FlowSelection;

const FLOW: &str = "\
version: 1
inputs:
  pr:
    type: number
    required: true
  focus:
    type: choice
    choices: [speed, clarity]
    default: clarity
  fix:
    type: boolean
nodes:
  - id: check
    kind: command
    run: \"true\"
";

fn open(cx: &mut TestAppContext) -> (gpui::Entity<FlowInputsModal>, gpui::VisualTestContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("review.yaml");
    std::fs::write(&path, FLOW).unwrap();
    let decls = daruda_flow::parse::parse_flow_file(FLOW).unwrap().inputs;
    let (window, workspace) =
        crate::workspace::tests::build_workspace_with(cx, &daruda_config::Config::default(), None);
    let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
    let pending = PendingFlowRun {
        lane: workspace.read_with(&vcx, |ws, _| ws.active),
        path,
        profile: None,
        selection: FlowSelection::default(),
    };
    let modal = vcx.update(|window, cx| {
        FlowInputsModal::open(workspace.downgrade(), pending, decls, window, cx)
    });
    vcx.run_until_parked();
    (modal, vcx)
}

fn values(modal: &gpui::Entity<FlowInputsModal>, vcx: &mut gpui::VisualTestContext) -> InputValues {
    modal.read_with(vcx, |modal, cx| modal.values(cx))
}

/// Each control starts at its input's default, and reads back as the value
/// its type takes.
#[gpui::test]
fn the_form_starts_from_the_defaults_and_reads_back_typed_values(cx: &mut TestAppContext) {
    let (modal, mut vcx) = open(cx);
    let start = values(&modal, &mut vcx);
    assert_eq!(start["pr"], InputValue::Text(String::new()));
    assert_eq!(start["focus"], InputValue::Text("clarity".into()));
    assert_eq!(start["fix"], InputValue::Bool(false));

    modal.update_in(&mut vcx, |modal, window, cx| {
        let super::Control::Text(pr) = &modal.fields[0].control else {
            panic!("a number is typed");
        };
        pr.update(cx, |state, cx| state.set_value("1234", window, cx));
        modal.set_toggle(2, true, cx);
    });
    let typed = values(&modal, &mut vcx);
    assert_eq!(typed["pr"], InputValue::Text("1234".into()));
    assert_eq!(typed["fix"], InputValue::Bool(true));
}

/// A refused value is named beside its own field, and the form stays up so
/// it can be corrected.
#[gpui::test]
fn a_missing_required_value_is_named_beside_its_field(cx: &mut TestAppContext) {
    use crate::ui::WindowExt as _;
    let (modal, mut vcx) = open(cx);
    modal.update_in(&mut vcx, |modal, window, cx| modal.submit(window, cx));
    vcx.run_until_parked();
    modal.read_with(&vcx, |modal, _| {
        assert!(modal.fields[0].error.is_some(), "pr is required");
        assert!(modal.fields[1].error.is_none());
        assert!(modal.fields[2].error.is_none());
    });
    assert!(vcx.update(|window, cx| window.has_active_dialog(cx)));
}
