//! The spine: what runs, in what order, and what counts as a node
//! having done its job — plus the two reads (prompt, hint) that can fail
//! on the way there.

use super::*;

#[test]
fn nodes_run_in_topological_order_and_a_clean_flow_finishes() {
    let runner = FakeRunner::new();
    let (report, _dir) = run(CHAIN, &runner);
    assert!(matches!(report.outcome, RunOutcome::Done));
    assert_eq!(runner.ids(), vec!["design", "test", "review"]);
}

/// `Halt` is the default, so a failing node stops the run and every
/// later node is never asked for.
#[test]
fn a_failing_node_halts_the_run_and_downstream_never_runs() {
    let runner = FakeRunner::new().script(
        "test",
        vec![Step::fail(NodeFailure::Exit { code: Some(1) })],
    );
    let (report, _dir) = run(CHAIN, &runner);
    match report.outcome {
        RunOutcome::Failed { node, ref failure } => {
            assert_eq!(node, "test");
            assert_eq!(*failure, NodeFailure::Exit { code: Some(1) });
        }
        other => panic!("expected a failure, got {other:?}"),
    }
    assert_eq!(
        runner.ids(),
        vec!["design", "test"],
        "review must never be asked for"
    );
}

/// An agent node that reports success but writes nothing has broken the
/// file contract, and the scheduler — not the runner — is what notices.
#[test]
fn an_agent_that_writes_nothing_fails_even_on_a_successful_turn() {
    let runner = FakeRunner::new().script("design", vec![Step::Ok { writes: None }]);
    let (report, _dir) = run(CHAIN, &runner);
    match report.outcome {
        RunOutcome::Failed {
            node,
            failure: NodeFailure::NoOutput { .. },
        } => {
            assert_eq!(node, "design");
        }
        other => panic!("expected NoOutput for design, got {other:?}"),
    }
}

/// The scheduler — not `render` — decides which surface a string is
/// bound for. A run directory with a space in it is what tells the two
/// apart: unquoted, the path splits into two arguments.
#[test]
fn the_scheduler_renders_each_string_for_its_own_surface() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run_dir = dir.path().join("a run with spaces");
    let loaded = load(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write {{output}}
  - id: gate
    kind: command
    deps: [design]
    run: \"grep -q x {{node.design.output}}\"
",
        None,
    )
    .expect("valid flow");
    let runner = FakeRunner::new();
    let _ = smol::block_on(run_flow(
        RunInputs {
            loaded: &loaded,
            flow_dir: dir.path(),
            cwd: dir.path(),
            run_dir: &run_dir,
            cancel: &CancelToken::default(),
            budget: &Budget::unlimited(),
            git_status: None,
            events: None,
            ask: None,
            resume: None,
        },
        &runner,
    ));

    let calls = runner.calls();
    let design = calls
        .iter()
        .find(|c| c.node == "design")
        .expect("design ran");
    assert_eq!(
        design.text,
        format!("write {}", run_dir.join("design.md").display()),
        "a prompt is prose and must not be quoted"
    );

    let gate = calls.iter().find(|c| c.node == "gate").expect("gate ran");
    assert_eq!(
        gate.text,
        format!("grep -q x '{}'", run_dir.join("design.md").display()),
        "a command must be quoted, or the space splits the argument"
    );
}

/// A prompt file that vanishes between validation and the run is the
/// cheapest way to reach an I/O failure through the real scheduler.
#[test]
fn an_io_failure_names_the_node_the_attempt_and_the_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let loaded = load(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt_file: gone.md
",
        None,
    )
    .expect("valid flow");
    let runner = FakeRunner::new();
    let report = smol::block_on(run_flow(
        RunInputs {
            loaded: &loaded,
            flow_dir: dir.path(),
            cwd: dir.path(),
            run_dir: &dir.path().join("run"),
            cancel: &CancelToken::default(),
            budget: &Budget::unlimited(),
            git_status: None,
            events: None,
            ask: None,
            resume: None,
        },
        &runner,
    ));

    match report.outcome {
        RunOutcome::Io(e) => {
            assert_eq!(
                e.site,
                IoSite::Node {
                    node: "design".to_string(),
                    attempt: 1
                }
            );
            assert!(e.path.ends_with("gone.md"), "{:?}", e.path);
            // The message is what a user or a log actually reads, so the
            // shape of it is the thing worth pinning.
            let text = e.to_string();
            assert!(text.contains("design"), "{text}");
            assert!(text.contains("gone.md"), "{text}");
        }
        other => panic!("expected an attributed I/O failure, got {other:?}"),
    }
    assert!(runner.calls().is_empty(), "the runner is never reached");
}

/// The attribution has to survive the second reader: an attempt reads the
/// node's prompt and then its hint, and naming the prompt when the hint is
/// what vanished points the reader at a file that is perfectly fine.
#[test]
fn a_missing_hint_file_is_not_reported_as_a_missing_prompt() {
    let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
    on_fail:
      retry:
        max_attempts: 2
        wait: 0s
        hint_file: gone-hint.md
";
    let runner = FakeRunner::new().script(
        "design",
        vec![Step::fail(NodeFailure::TurnFailed("boom".into()))],
    );
    let (report, _dir) = run(flow, &runner);
    match report.outcome {
        RunOutcome::Io(e) => {
            assert!(e.path.ends_with("gone-hint.md"), "{:?}", e.path);
            assert!(e.doing.contains("hint"), "{}", e.doing);
            assert_eq!(
                e.site,
                IoSite::Node {
                    node: "design".to_string(),
                    attempt: 2
                }
            );
        }
        other => panic!("expected the hint file to be named, got {other:?}"),
    }
}

/// An empty file is the same broken contract as no file.
#[test]
fn an_empty_output_file_is_also_no_output() {
    let runner = FakeRunner::new().script(
        "design",
        vec![Step::Ok {
            writes: Some(String::new()),
        }],
    );
    let (report, _dir) = run(CHAIN, &runner);
    assert!(matches!(
        report.outcome,
        RunOutcome::Failed {
            failure: NodeFailure::NoOutput { .. },
            ..
        }
    ));
}

/// The e2e fixtures are the only description of what a real run should do,
/// and nothing else compiles them. A flow that stopped parsing — or a
/// `prompt_file` that moved — would be found by a person at the keyboard,
/// which is the worst place to find it.
#[test]
fn the_shipped_example_flows_still_load() {
    let flows = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/flows");
    let mut checked = 0;
    for entry in std::fs::read_dir(&flows).expect("the example flows are shipped") {
        let path = entry.expect("readable").path();
        if path.extension().is_none_or(|e| e != "yaml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("readable");
        let loaded =
            load(&text, None).unwrap_or_else(|e| panic!("{} does not load: {e}", path.display()));
        let flow = loaded.flow();

        // A file-backed prompt names a path relative to the flow file, so a
        // fixture whose prompt moved is broken in a way `load` cannot see.
        for node in &flow.nodes {
            if let crate::model::NodeKind::Agent {
                prompt: crate::model::Prompt::File(rel),
                ..
            } = &node.kind
            {
                let resolved = flows.join(rel);
                assert!(
                    resolved.is_file(),
                    "{}: prompt_file {} is missing",
                    path.display(),
                    resolved.display()
                );
            }
        }
        checked += 1;
    }
    assert!(checked >= 5, "only {checked} example flows were checked");
}
