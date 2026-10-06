//! What text a session actually receives. The scheduler refuses a node
//! whose declared output was never written, so the prompt has to state that
//! rule — an agent that did the work and wrote nothing is otherwise failed
//! for breaking a contract nobody told it about.
//!
//! Which session gets the block is keyed on owing a file, not on being an
//! agent: a repair's fix session is an agent run that owes nothing.

use super::*;

/// One agent node whose prompt says nothing about writing anything — the
/// shape the graph inspector authors, and the one the defect appeared in.
const SILENT: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: make me a file
";

/// The contract arrives even though the prompt never mentions an output,
/// and it states the absolute path — a node runs in the working tree, not
/// in the run directory, so a relative one would land somewhere else.
#[test]
fn an_agent_node_is_told_the_contract_its_output_is_judged_by() {
    let runner = FakeRunner::new();
    let (report, dir) = run(SILENT, &runner);
    assert!(matches!(report.outcome, RunOutcome::Done));

    let run_dir = dir.path().join("run");
    assert_eq!(
        runner.calls()[0].text,
        format!(
            "make me a file\n\n---\n{}",
            expected_contract(&run_dir, "design.md")
        ),
        "the contract is the last channel, and states the absolute path"
    );
}

/// A command node has no `output` at all, so there is nothing to contract
/// about and the text handed to the shell must stay exactly the `run` line.
#[test]
fn a_command_node_is_told_nothing_about_an_output() {
    let runner = FakeRunner::new();
    let (report, _dir) = run(CHAIN, &runner);
    assert!(matches!(report.outcome, RunOutcome::Done));
    let calls = runner.calls();
    let gate = calls.iter().find(|c| c.node == "test").expect("test ran");
    assert_eq!(gate.text, "true");
}

/// Why the injection asks whether a file is owed rather than what kind of
/// node it is: the fix session is a real agent session that edits the tree,
/// and told to write a file it would invent one the run has no place for.
#[test]
fn a_repairs_fix_session_is_told_no_contract() {
    let runner = FakeRunner::new().script(
        "gate",
        vec![
            Step::fail(NodeFailure::Exit { code: Some(1) }),
            Step::Ok { writes: None },
        ],
    );
    let (_report, _dir) = run(GATED, &runner);
    let calls = runner.calls();
    let fix = calls
        .iter()
        .find(|c| c.node == "__fix__")
        .expect("a fix ran");
    assert!(
        !fix.text.contains("OUTPUT CONTRACT"),
        "the fix owes no file: {}",
        fix.text
    );
}

/// A file-backed prompt reaches the same composition, because the block is
/// appended to the rendered text rather than to the authored prompt.
#[test]
fn a_file_backed_prompt_gets_the_contract_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("design.prompt"), "do the design").expect("write");
    let loaded = load(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt_file: design.prompt
",
        None,
    )
    .expect("valid flow");
    let run_dir = dir.path().join("run");
    let runner = FakeRunner::new();
    let report = smol::block_on(run_flow(
        RunInputs {
            pinned: Vec::new(),
            until: None,
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
    assert!(matches!(report.outcome, RunOutcome::Done));
    assert_eq!(
        runner.calls()[0].text,
        format!(
            "do the design\n\n---\n{}",
            expected_contract(&run_dir, "design.md")
        )
    );
}
