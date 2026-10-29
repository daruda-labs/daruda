//! Inspector lifecycle across reloads, canvas edits, refusals, and file loss.
use super::authoring_edits::set_output;
use super::*;
use crate::workspace::pages::flows::graph::inspector::notes::FormField;

fn open(
    flow: &str,
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    gpui::Entity<crate::workspace::Workspace>,
    std::path::PathBuf,
    gpui::Entity<crate::workspace::pages::flows::graph::FlowGraphView>,
    gpui::VisualTestContext,
) {
    let (lane, ws, path, wh) = workspace_with_a_flow(cx, flow);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_flow_graph(ws.active, &path, window, cx)
    });
    vcx.run_until_parked();
    let view = ws.read_with(&vcx, |ws, _| ws.open_graph().unwrap().2);
    (lane, ws, path, view, vcx)
}

#[gpui::test]
async fn the_discarded_notice_can_be_dismissed(cx: &mut TestAppContext) {
    let (_lane, _ws, path, view, mut vcx) = open(TWO_NODE_CHAIN, cx);
    view.update_in(&mut vcx, |v, window, cx| {
        set_output(v, "design", "mine.md", window, cx);
        v.select_node_for_test(&"build".into(), window, cx);
    });
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("design", "drawing")).unwrap();
    view.update_in(&mut vcx, |v, window, cx| v.reload(window, cx));
    assert!(!view.read_with(&vcx, |v, _| v.discarded_drafts().is_empty()));
    view.update_in(&mut vcx, |v, _, cx| v.dismiss_discarded_drafts(cx));
    assert!(view.read_with(&vcx, |v, _| v.discarded_drafts().is_empty()));
}

const EMPTY_RUN: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write a line
  - id: check
    kind: command
    run: \"\"
";

#[gpui::test]
async fn an_untouched_refused_form_does_not_block_saving_others(cx: &mut TestAppContext) {
    let (_lane, ws, path, view, mut vcx) = open(EMPTY_RUN, cx);
    view.update_in(&mut vcx, |v, window, cx| {
        set_output(v, "design", "mine.md", window, cx);
        v.select_node_for_test(&"check".into(), window, cx);
    });
    assert!(view.read_with(&vcx, |v, cx| v.form().unwrap().refusal(cx).is_some()));
    assert!(ws.update_in(&mut vcx, |ws, window, cx| ws.save_flow_editor(
        &path,
        view.clone(),
        window,
        cx
    )));
    assert!(std::fs::read_to_string(&path).unwrap().contains("mine.md"));
}

#[gpui::test]
async fn an_unrelated_canvas_edit_keeps_the_live_form(cx: &mut TestAppContext) {
    let (_lane, ws, path, view, mut vcx) = open(LONG_CHAIN, cx);
    view.update_in(&mut vcx, |v, window, cx| {
        set_output(v, "n1", "mine.md", window, cx)
    });
    let before = view.read_with(&vcx, |v, cx| {
        v.form().unwrap().body_states(cx).output.entity_id()
    });
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.disconnect_nodes(&path, view.clone(), &"n2".into(), &"n3".into(), window, cx)
    });
    view.read_with(&vcx, |v, cx| {
        let form = v.form().unwrap();
        assert_eq!(form.body_states(cx).output.entity_id(), before);
        assert_eq!(form.fields(cx).body.output, "mine.md");
        assert!(!v.draft_conflict());
    });
}

#[gpui::test]
async fn a_lost_file_keeps_the_selected_input(cx: &mut TestAppContext) {
    let (_lane, _ws, path, view, mut vcx) = open(TWO_NODE_CHAIN, cx);
    view.update_in(&mut vcx, |v, window, cx| {
        set_output(v, "design", "mine.md", window, cx)
    });
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    view.update_in(&mut vcx, |v, _, cx| v.report_file_gone(cx));
    assert!(view.read_with(&vcx, |v, cx| v.has_unsaved_form(cx)));
    std::fs::write(&path, text).unwrap();
    // The selection comes back with the file, so the kept input is visible
    // rather than pending behind a node nobody has clicked.
    view.update_in(&mut vcx, |v, window, cx| v.reload(window, cx));
    assert_eq!(
        view.read_with(&vcx, |v, cx| v.form().unwrap().fields(cx).body.output),
        "mine.md"
    );
}

#[gpui::test]
async fn a_rename_refusal_stays_on_the_renamed_node(cx: &mut TestAppContext) {
    let (_lane, ws, path, view, mut vcx) = open(TWO_NODE_CHAIN, cx);
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test(&"design".into(), window, cx);
        v.form()
            .unwrap()
            .id_state()
            .clone()
            .update(cx, |input, cx| input.set_value("build", window, cx));
    });
    assert!(
        !ws.update_in(&mut vcx, |ws, window, cx| ws.save_flow_editor(
            &path,
            view.clone(),
            window,
            cx
        ))
    );
    view.read_with(&vcx, |v, _| {
        let form = v.form().unwrap();
        assert_eq!(form.node, daruda_flow::NodeId::from("design"));
        assert!(form.note_for_test(FormField::Id).is_some());
    });
}

#[gpui::test]
async fn deleting_a_node_clears_it_from_a_kept_form(cx: &mut TestAppContext) {
    let (_lane, ws, path, view, mut vcx) = open(LONG_CHAIN, cx);
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test(&"n1".into(), window, cx);
        v.form()
            .unwrap()
            .deps_state()
            .clone()
            .update(cx, |input, cx| input.set_value("n3", window, cx));
    });
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.delete_nodes(&path, view.clone(), vec!["n3".into()], window, cx)
    });
    view.read_with(&vcx, |v, cx| {
        let form = v.form().unwrap();
        assert_eq!(form.node, daruda_flow::NodeId::from("n1"));
        assert!(form.fields(cx).deps.is_empty());
    });
}

#[gpui::test]
async fn inactive_typing_does_not_revive_an_untouched_refusal(cx: &mut TestAppContext) {
    let (_lane, ws, path, view, mut vcx) = open(EMPTY_RUN, cx);
    view.update_in(&mut vcx, |v, window, cx| {
        set_output(v, "design", "mine.md", window, cx);
        v.select_node_for_test(&"check".into(), window, cx);
        v.form()
            .unwrap()
            .body_states(cx)
            .prompt
            .inline
            .clone()
            .update(cx, |input, cx| {
                input.set_value("not for a command", window, cx)
            });
    });
    assert!(ws.update_in(&mut vcx, |ws, window, cx| ws.save_flow_editor(
        &path,
        view.clone(),
        window,
        cx
    )));
    assert!(std::fs::read_to_string(&path).unwrap().contains("mine.md"));
}

const MULTILINE_RUN: &str = "\
version: 1
nodes:
  - id: check
    kind: command
    run: |
      cargo test
      cargo build
";

fn saved_run(path: &std::path::Path) -> String {
    let file =
        daruda_flow::parse::parse_flow_file(&std::fs::read_to_string(path).unwrap()).unwrap();
    let daruda_flow::parse::NodeKindFile::Command { run, .. } = &file.nodes[0].kind else {
        panic!("a command");
    };
    run.trim_end().to_string()
}

#[gpui::test]
async fn a_multiline_command_is_shown_and_kept_whole(cx: &mut TestAppContext) {
    let (_lane, ws, path, view, mut vcx) = open(MULTILINE_RUN, cx);
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test(&"check".into(), window, cx)
    });
    vcx.run_until_parked();
    assert!(!view.read_with(&vcx, |v, cx| v.has_unsaved_form(cx)));
    let run = view.read_with(&vcx, |v, cx| v.form().unwrap().body_states(cx).run.clone());
    assert_eq!(
        run.read_with(&vcx, |input, _| input.value().to_string())
            .trim_end(),
        "cargo test\ncargo build"
    );

    // Another field's save leaves the block untouched.
    view.update_in(&mut vcx, |v, window, cx| {
        v.form()
            .unwrap()
            .timeout_state()
            .clone()
            .update(cx, |input, cx| input.set_value("10m", window, cx));
    });
    vcx.run_until_parked();
    let save = |ws: &gpui::Entity<crate::workspace::Workspace>,
                vcx: &mut gpui::VisualTestContext| {
        ws.update_in(vcx, |ws, window, cx| {
            ws.save_flow_editor(&path, view.clone(), window, cx)
        })
    };
    assert!(save(&ws, &mut vcx));
    assert_eq!(saved_run(&path), "cargo test\ncargo build");

    // Editing the command keeps the lines the person typed. A save rebinds
    // the form, so the box is the new binding's.
    let run = view.read_with(&vcx, |v, cx| v.form().unwrap().body_states(cx).run.clone());
    run.update_in(&mut vcx, |input, window, cx| {
        input.set_value("cargo test\ncargo clippy", window, cx)
    });
    vcx.run_until_parked();
    assert!(save(&ws, &mut vcx));
    assert_eq!(saved_run(&path), "cargo test\ncargo clippy");
}

#[gpui::test]
async fn editing_the_title_clears_a_refusal_from_the_form(cx: &mut TestAppContext) {
    let (_lane, ws, path, view, mut vcx) = open(TWO_NODE_CHAIN, cx);
    view.update_in(&mut vcx, |v, window, cx| {
        v.select_node_for_test(&"design".into(), window, cx);
        v.form()
            .unwrap()
            .id_state()
            .clone()
            .update(cx, |input, cx| input.set_value("build", window, cx));
    });
    assert!(
        !ws.update_in(&mut vcx, |ws, window, cx| ws.save_flow_editor(
            &path,
            view.clone(),
            window,
            cx
        ))
    );
    assert!(view.read_with(&vcx, |v, _| v.form().unwrap().banner().is_some()));
    view.update_in(&mut vcx, |v, window, cx| {
        v.name_input_for_test()
            .update(cx, |input, cx| input.set_value("Renamed", window, cx))
    });
    vcx.run_until_parked();
    view.read_with(&vcx, |v, _| {
        assert!(v.form().unwrap().banner().is_none());
        assert!(v.save_error_for_test().is_none());
    });
}
