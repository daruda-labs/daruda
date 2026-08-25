//! A node that runs somewhere other than the run's own directory.
//!
//! The rule these all rest on is that a node's `cwd` stays *inside* the
//! run's. That is what lets the run keep one lock: it holds the directory
//! it was given, and every node works under it. A lock per subdirectory
//! would be worse than none — a run holding the root and a run holding
//! `sub/` would not exclude each other, and both would write to `sub/`.

use super::*;
use crate::error::{FlowError, ValidationKind};

/// One command node in a subdirectory, one in the run's own directory.
const TWO_TREES: &str = "\
version: 1
nodes:
  - id: here
    kind: command
    run: \"true\"
  - id: there
    kind: command
    deps: [here]
    cwd: sub
    run: \"true\"
";

/// A node's `cwd` reaches the runner. Without this the field parses,
/// validates, is written to `run.yaml` — and changes nothing about where
/// anything runs.
#[test]
fn a_node_runs_in_the_directory_it_names() {
    struct Watcher(FakeRunner, std::cell::RefCell<Vec<PathBuf>>);

    impl NodeRunner for Watcher {
        fn run_agent<'a>(
            &'a self,
            ctx: &'a RunContext<'a>,
            agent: &'a crate::model::AgentSpec,
            prompt: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
            self.1.borrow_mut().push(ctx.cwd.to_path_buf());
            self.0.run_agent(ctx, agent, prompt)
        }

        fn run_command<'a>(
            &'a self,
            ctx: &'a RunContext<'a>,
            run: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
            self.1.borrow_mut().push(ctx.cwd.to_path_buf());
            self.0.run_command(ctx, run)
        }
    }

    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("sub")).expect("mkdir");
    let watcher = Watcher(FakeRunner::new(), std::cell::RefCell::new(Vec::new()));
    execute(
        &request_for(TWO_TREES, dir.path()),
        &watcher,
        &CancelToken::default(),
    );

    let seen = watcher.1.into_inner();
    assert_eq!(
        seen,
        vec![dir.path().to_path_buf(), dir.path().join("sub")],
        "a node did not run where it said it would"
    );
}

/// A `cwd` that climbs out is refused. This is the rule the single lock
/// rests on, so it is a correctness check and not a tidiness one: a node
/// working above the run's directory works where nobody holds a lock.
#[test]
fn a_cwd_that_climbs_out_of_the_run_is_refused() {
    for escape in ["../elsewhere", "/tmp", "sub/../../up"] {
        let text = format!(
            "version: 1\nnodes:\n  - id: a\n    kind: command\n    cwd: {escape}\n    run: \"true\"\n"
        );
        let err = load(&text, None).expect_err("an escaping cwd is not a flow");
        let FlowError::Validate(issues) = err else {
            panic!("{escape}: refused, but not as a validation issue");
        };
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::CwdEscapesRunCwd)),
            "{escape}: {issues:?}"
        );
    }
}

/// A directory that is not there is caught before the run takes the lock,
/// not by the node failing halfway in. Only the request knows what a
/// relative `cwd` is relative to, so this is a request-level rule.
#[test]
fn a_cwd_that_is_not_there_is_caught_at_submission() {
    let dir = tempfile::tempdir().expect("tempdir");
    // `sub` deliberately not created.
    let issues = crate::request::validate_request(&request_for(TWO_TREES, dir.path()));
    assert!(
        issues.iter().any(
            |i| matches!(&i.kind, ValidationKind::CwdMissing { path } if path.ends_with("sub"))
        ),
        "{issues:?}"
    );
}

/// The record says where each attempt ran, by asking about that tree and
/// not the run's. A flow whose nodes work in different directories would
/// otherwise report the same status for all of them.
#[test]
fn the_working_tree_note_is_taken_where_the_node_ran() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("sub")).expect("mkdir");
    let mut request = request_for(TWO_TREES, dir.path());
    request.git_status = Some(Box::new(|at: &std::path::Path| {
        Some(format!(
            " M {}",
            at.file_name().unwrap_or_default().to_string_lossy()
        ))
    }));

    let report = execute(&request, &FakeRunner::new(), &CancelToken::default());
    let notes: Vec<Option<String>> = report
        .nodes
        .iter()
        .flat_map(|n| n.attempts.iter().map(|a| a.git_status.clone()))
        .collect();
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(
        notes[1].as_deref().is_some_and(|n| n.ends_with("sub")),
        "the second node's tree was reported as the run's: {notes:?}"
    );
}

/// Readiness is asked, not assumed.
///
/// While one node runs at a time the answer is always yes — the worklist
/// walks a topological order, so a node's dependencies are behind it. The
/// check exists for the moment two can run at once, and until then nothing
/// else would notice it always returning true.
#[test]
fn a_node_waits_for_what_it_depends_on() {
    use crate::schedule::ready::deps_are_done;

    let loaded = load(CHAIN, None).expect("valid flow");
    let flow = loaded.flow();
    let ids: Vec<NodeId> = flow.nodes.iter().map(|node| node.id.clone()).collect();
    let mut done: HashSet<NodeId> = HashSet::new();

    // The first depends on nothing, so it is ready from the start; each of
    // the rest is not, until the one before it is done.
    assert!(deps_are_done(flow, &ids[0], &done));
    for pair in ids.windows(2) {
        assert!(
            !deps_are_done(flow, &pair[1], &done),
            "`{}` was ready before `{}` finished",
            pair[1],
            pair[0]
        );
        done.insert(pair[0].clone());
        assert!(deps_are_done(flow, &pair[1], &done));
    }
}
