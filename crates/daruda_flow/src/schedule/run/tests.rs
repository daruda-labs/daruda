use super::super::tests::{CHAIN, request_for};
use super::*;
use crate::marker::DEFAULT_KEEP_RUNS;
use crate::testing::FakeRunner;
use std::cell::RefCell;

/// One agent node named `a`, so a pin has somewhere to land.
const ONE_AGENT: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
";

/// `review` overrides the flow's agent, so one run needs two runtimes.
const TWO_AGENTS: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
  - id: review
    kind: agent
    deps: [design]
    agent: { id: codex, mode: auto }
    output: review.md
    prompt: write
";

/// No node names an agent, and yet a repair's `fix` would open a session
/// as `defaults.agent`. The repair is the whole fixture: without one
/// nothing here could ever open a session, and provisioning would be
/// paying to download a runtime for work the flow cannot ask for.
const COMMAND_ONLY_WITH_REPAIR: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: gate
    kind: command
    run: \"true\"
    on_fail:
      repair:
        fix: fix it from {{attempts}}
        max_attempts: 2
";

/// The same, minus the repair — so nothing in it can open a session.
const COMMAND_ONLY: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: gate
    kind: command
    run: \"true\"
";

fn spec() -> daruda_acp::LaunchSpec {
    daruda_acp::LaunchSpec {
        command: "x".to_string(),
        strip_env: Vec::new(),
    }
}

fn runs_dir_of(run_dir: &Path) -> &Path {
    run_dir.parent().expect("a run directory has a parent")
}

fn finished_run_in(dir: &Path, run_id: &str) -> std::path::PathBuf {
    let run_dir = dir.join(run_id);
    std::fs::create_dir_all(&run_dir).expect("mkdir");
    std::fs::write(run_dir.join("DONE"), "").expect("marker");
    run_dir
}

/// Without this every run's artifacts show up in the user's `git status`
/// — the repo's own `.gitignore` has no `.daruda` entry.
#[test]
fn the_first_run_hides_its_output_from_git() {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = execute(
        &request_for(CHAIN, dir.path()),
        &FakeRunner::new(),
        &CancelToken::default(),
    );

    let text =
        std::fs::read_to_string(runs_dir_of(&report.run_dir).join(".gitignore")).expect("written");
    assert!(text.contains('*'), "{text}");
}

/// The user may have edited it. Rewriting on every run would undo that.
#[test]
fn a_later_run_does_not_rewrite_an_existing_gitignore() {
    let dir = tempfile::tempdir().expect("tempdir");
    let request = request_for(CHAIN, dir.path());
    let runs_dir = runs_dir_of(&request.run_dir).to_path_buf();
    std::fs::create_dir_all(&runs_dir).expect("mkdir");
    std::fs::write(runs_dir.join(".gitignore"), "*\n!keep-me.md\n").expect("write");

    let _ = execute(&request, &FakeRunner::new(), &CancelToken::default());

    assert_eq!(
        std::fs::read_to_string(runs_dir.join(".gitignore")).expect("still there"),
        "*\n!keep-me.md\n"
    );
}

/// Retention only matters if a run actually performs it. Invisible to
/// git is not invisible to the disk.
#[test]
fn a_run_sweeps_the_runs_directory_it_starts_in() {
    let dir = tempfile::tempdir().expect("tempdir");
    let request = request_for(CHAIN, dir.path());
    let runs_dir = runs_dir_of(&request.run_dir).to_path_buf();
    std::fs::create_dir_all(&runs_dir).expect("mkdir");
    let old: Vec<_> = (0..DEFAULT_KEEP_RUNS + 5)
        .map(|i| finished_run_in(&runs_dir, &format!("01A{i:02}")))
        .collect();

    let report = execute(&request, &FakeRunner::new(), &CancelToken::default());

    assert!(report.warnings().is_empty(), "{:?}", report.warnings());
    for gone in &old[..5] {
        assert!(!gone.exists(), "{} survived the sweep", gone.display());
    }
    assert!(old[5].is_dir(), "the newest 20 must stay");
    assert!(report.run_dir.join("DONE").is_file());
}

/// The reason this is not lazy: a first-run download inside a node's turn
/// eats that node's timeout, and the node that pays is whichever happened
/// to be first.
#[test]
fn every_distinct_agent_is_provisioned_before_the_first_node() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut request = request_for(TWO_AGENTS, dir.path());
    request.agents.insert("codex".to_string(), spec());
    let runner = FakeRunner::new();
    let provisioned = RefCell::new(Vec::new());

    let report = execute_with(&request, &runner, &CancelToken::default(), &|id, _| {
        assert!(
            runner.calls().is_empty(),
            "`{id}` was prepared after a node had already run"
        );
        provisioned.borrow_mut().push(id.to_string());
        Ok(Vec::new())
    });

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    let mut prepared = provisioned.into_inner();
    prepared.sort();
    assert_eq!(prepared, vec!["claude", "codex"]);
}

#[test]
fn cancellation_during_preparation_is_not_an_install_failure() {
    let dir = tempfile::tempdir().unwrap();
    let request = request_for(CHAIN, dir.path());
    let runner = FakeRunner::new();
    let cancel = CancelToken::default();
    let report = execute_with(&request, &runner, &cancel, &|_, _| {
        cancel.cancel();
        Err("preparation canceled".to_owned())
    });
    assert!(matches!(report.outcome, RunOutcome::Canceled { .. }));
    assert!(runner.calls().is_empty());
}

#[test]
fn cached_fallback_notices_are_written_to_the_run_record() {
    let dir = tempfile::tempdir().unwrap();
    let request = request_for(CHAIN, dir.path());
    let report = execute_with(
        &request,
        &FakeRunner::new(),
        &CancelToken::default(),
        &|_, _| Ok(vec!["using verified cached adapter".to_owned()]),
    );
    let record = std::fs::read_to_string(report.run_dir.join(RUN_MD)).unwrap();
    assert!(record.contains("using verified cached adapter"));
}

/// Two nodes naming the same agent must not provision twice.
#[test]
fn one_agent_named_by_many_nodes_is_provisioned_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runner = FakeRunner::new();
    let provisioned = RefCell::new(Vec::new());

    let report = execute_with(
        &request_for(CHAIN, dir.path()),
        &runner,
        &CancelToken::default(),
        &|id, _| {
            provisioned.borrow_mut().push(id.to_string());
            Ok(Vec::new())
        },
    );

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert_eq!(provisioned.into_inner(), vec!["claude"]);
}

/// Provisioning downloads a runtime, so a selection that stops short of a
/// node must not pay for that node's agent — minutes of download for work
/// the run then throws away.
#[test]
fn a_selection_does_not_provision_an_agent_it_will_not_reach() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provisioned = RefCell::new(Vec::new());
    let mut request = request_for(TWO_AGENTS, dir.path());
    request.until = Some(crate::NodeId::from("design"));

    let report = execute_with(
        &request,
        &FakeRunner::new(),
        &CancelToken::default(),
        &|id, _| {
            provisioned.borrow_mut().push(id.to_string());
            Ok(Vec::new())
        },
    );

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert_eq!(
        provisioned.into_inner(),
        vec!["claude"],
        "`codex` belongs to `review`, which this run stops before"
    );
}

/// The same claim, with the selection where a continuation keeps it. A
/// resumed run is handed `until: None` and reads the axis back off the
/// journal, so a gate asking the field instead of `effective_until` sees
/// no selection at all — and this flow's `review` names an agent the
/// catalog lacks, which refused the whole resume over a node it stops
/// before.
#[test]
fn a_resumed_selection_narrows_the_same_way_a_fresh_one_does() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provisioned = RefCell::new(Vec::new());
    let mut request = request_for(TWO_AGENTS, dir.path());
    request.until = None;
    request.resume = Some(crate::journal::Replay {
        passed: Vec::new(),
        next_seq: 1,
        records: Vec::new(),
        spent: Default::default(),
        profile: None,
        until: Some(crate::NodeId::from("design")),
        pinned: Vec::new(),
        torn: Default::default(),
    });

    let report = execute_with(
        &request,
        &FakeRunner::new(),
        &CancelToken::default(),
        &|id, _| {
            provisioned.borrow_mut().push(id.to_string());
            Ok(Vec::new())
        },
    );

    assert!(
        !matches!(report.outcome, RunOutcome::Invalid { .. }),
        "refused over a node it stops before: {:?}",
        report.outcome
    );
    assert_eq!(provisioned.into_inner(), vec!["claude"]);
}

/// A repair's `fix` opens a real session as `defaults.agent`, so that
/// agent needs a runtime too — in a flow where no node names one, nothing
/// else would ever ask for it.
#[test]
fn the_repair_agent_is_provisioned_even_when_no_node_names_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provisioned = RefCell::new(Vec::new());

    let report = execute_with(
        &request_for(COMMAND_ONLY_WITH_REPAIR, dir.path()),
        &FakeRunner::new(),
        &CancelToken::default(),
        &|id, _| {
            provisioned.borrow_mut().push(id.to_string());
            Ok(Vec::new())
        },
    );

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert_eq!(provisioned.into_inner(), vec!["claude"]);
}

/// And the other direction: a flow with no repair has nothing that can
/// open an agent session, so the agent its `defaults` names is a runtime
/// nobody will ask for. Downloading one is minutes spent on nothing.
#[test]
fn a_flow_that_cannot_open_a_session_provisions_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provisioned = RefCell::new(Vec::new());

    let report = execute_with(
        &request_for(COMMAND_ONLY, dir.path()),
        &FakeRunner::new(),
        &CancelToken::default(),
        &|id, _| {
            provisioned.borrow_mut().push(id.to_string());
            Ok(Vec::new())
        },
    );

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert!(provisioned.into_inner().is_empty());
}

/// Provisioning is what makes a run possible at all, so its failure is the
/// run's — not a node's, since no node has run yet.
#[test]
fn a_provisioning_failure_stops_the_run_before_any_node() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runner = FakeRunner::new();

    let report = execute_with(
        &request_for(CHAIN, dir.path()),
        &runner,
        &CancelToken::default(),
        &|id, _| Err(format!("no runtime for `{id}`")),
    );

    match &report.outcome {
        RunOutcome::Unprovisioned { agent, message } => {
            assert_eq!(agent, "claude");
            assert!(message.contains("no runtime"), "{message}");
        }
        other => panic!("expected an unprovisioned run, got {other:?}"),
    }
    assert!(runner.calls().is_empty(), "no node may have run");
    // This run took the directory, so it still ends the way every started
    // run does — a reader acting on the marker needs the account beside it.
    assert!(report.run_dir.join("FAILED").is_file());
    assert!(report.run_dir.join(RUN_MD).is_file());
}

/// The copy is what makes a run directory self-contained: `archive` and
/// `{{node.<id>.output}}` both assume the output is inside it.
#[test]
fn a_pinned_output_is_copied_to_where_the_node_owes_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("earlier.md");
    std::fs::write(&source, "reused\n").expect("source");
    let mut request = request_for(ONE_AGENT, dir.path());
    request.pinned = vec![crate::request::PinnedOutput {
        node: "a".into(),
        from: source,
    }];

    let (took, warnings) = copy_pinned_outputs(&request);

    assert_eq!(took.iter().map(|n| n.as_str()).collect::<Vec<_>>(), ["a"]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        std::fs::read_to_string(request.run_dir.join("a.md")).expect("copied"),
        "reused\n"
    );
}

/// A pin on a node the run stops before belongs to a later run, not this
/// one. Copying it put a file in the directory that nothing here produced
/// and named the node reused — in a run whose report says it stopped
/// upstream of it — and the journal counts a pinned node as passed, so a
/// resume would then skip it on the strength of an output it never saw
/// made.
#[test]
fn a_pin_beyond_the_selection_is_not_copied_or_counted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("earlier.md");
    std::fs::write(&source, "from a run before\n").expect("source");
    let mut request = request_for(TWO_AGENTS, dir.path());
    request.until = Some(crate::NodeId::from("design"));
    request.pinned = vec![crate::request::PinnedOutput {
        node: "review".into(),
        from: source,
    }];

    let (took, warnings) = copy_pinned_outputs(&request);

    assert!(took.is_empty(), "{took:?}");
    assert!(
        warnings.is_empty(),
        "not a failure, just not this run: {warnings:?}"
    );
    assert!(
        !request.run_dir.join("review.md").exists(),
        "no node here writes it"
    );
}

/// A copy that fails un-pins the node instead of refusing the run: the
/// node then runs and is paid for, which is what no pin at all would do.
#[test]
fn a_pin_whose_copy_fails_is_a_warning_and_not_a_refusal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut request = request_for(ONE_AGENT, dir.path());
    request.pinned = vec![crate::request::PinnedOutput {
        node: "a".into(),
        from: dir.path().join("was-never-there.md"),
    }];

    let (took, warnings) = copy_pinned_outputs(&request);

    assert!(took.is_empty(), "{took:?}");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("will be run"), "{}", warnings[0]);
}
