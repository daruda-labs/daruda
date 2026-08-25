//! The file contract as the scheduler enforces it, and the two ways a
//! symlink defeats a size check: a link standing where the output goes, and
//! a link on the way to it. Both are reachable by an agent — nodes run under
//! `bypassPermissions` — so both are the scheduler's to refuse.
//!
//! And the declared shape, which is the same question one layer in: the file
//! is the node's, and its contents still are not what the node promised.

use super::*;

/// One agent node, so a refusal is the run's outcome with no `on_fail`
/// policy in the way.
const ONE_NODE: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
";

/// A gate first, so an earlier node has run — and can leave something
/// behind — before the writing node's nested output is reached.
const NESTED: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: setup
    kind: command
    run: \"true\"
  - id: design
    kind: agent
    deps: [setup]
    output: reports/out.md
    prompt: write
";

/// The same node, now promising a shape as well as a file.
const SHAPED: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.json
    output_schema:
      type: object
      required: [verdict]
      properties:
        verdict: { type: string, enum: [pass, fail] }
    prompt: write
";

/// Writes nothing and puts a link where its output belongs — an agent
/// satisfying the contract by pointing at someone else's file.
struct Linker {
    inner: FakeRunner,
    target: PathBuf,
}

impl NodeRunner for Linker {
    fn run_agent<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        agent: &'a crate::model::AgentSpec,
        prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        if let Some(output) = ctx.output {
            std::os::unix::fs::symlink(&self.target, output).expect("symlink");
        }
        self.inner.run_agent(ctx, agent, prompt)
    }

    fn run_command<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        run: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        self.inner.run_command(ctx, run)
    }
}

/// Puts a link where the *next* node's output directory goes, then behaves.
/// The planting node passes — which is what makes this the case a post-run
/// check cannot cover: by then the write has already landed outside.
struct Planter {
    inner: FakeRunner,
    outside: PathBuf,
}

impl NodeRunner for Planter {
    fn run_agent<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        agent: &'a crate::model::AgentSpec,
        prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        self.inner.run_agent(ctx, agent, prompt)
    }

    fn run_command<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        run: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        std::fs::create_dir_all(&self.outside).expect("mkdir");
        std::os::unix::fs::symlink(&self.outside, ctx.run_dir.join("reports")).expect("symlink");
        self.inner.run_command(ctx, run)
    }
}

/// The hole: `metadata` follows links, so a node that wrote nothing could
/// point its output at any non-empty file and be judged as having worked.
#[test]
fn a_node_that_only_linked_its_output_does_not_pass() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("someone-elses.md");
    std::fs::write(&target, "work this node did not do\n").expect("write");
    let runner = Linker {
        inner: FakeRunner::new().script("design", vec![Step::Ok { writes: None }]),
        target: target.clone(),
    };

    let report = execute(
        &request_for(ONE_NODE, dir.path()),
        &runner,
        &CancelToken::default(),
    );

    let RunOutcome::Failed { node, failure } = &report.outcome else {
        panic!("a link must not satisfy the contract: {:?}", report.outcome);
    };
    assert_eq!(node, "design");
    assert!(
        matches!(failure, NodeFailure::OutputNotAFile { .. }),
        "{failure:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&target).expect("read"),
        "work this node did not do\n",
        "the target must be left alone"
    );
}

/// The case detection cannot cover. `create_dir_all` follows a planted link,
/// so the refusal has to land before the directory is made and before the
/// runner is called — otherwise the agent's write is already outside.
#[test]
fn a_linked_parent_stops_the_node_before_it_starts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outside = dir.path().join("outside");
    let runner = Planter {
        inner: FakeRunner::new(),
        outside: outside.clone(),
    };

    let report = execute(
        &request_for(NESTED, dir.path()),
        &runner,
        &CancelToken::default(),
    );

    let RunOutcome::Failed { node, failure } = &report.outcome else {
        panic!(
            "a linked output directory must be refused: {:?}",
            report.outcome
        );
    };
    assert_eq!(node, "design");
    assert!(
        matches!(failure, NodeFailure::OutputEscapes { .. }),
        "{failure:?}"
    );
    assert_eq!(
        runner.inner.ids(),
        vec![NodeId::from("setup")],
        "the refused node must never have been started"
    );
    assert!(
        !outside.join("out.md").exists(),
        "nothing may be written outside the run directory"
    );
}

/// The refusal is not a silent stop: `run.md` names the node and says what
/// the path resolved to, which is what makes it actionable.
#[test]
fn the_record_says_which_node_was_refused_and_where_its_output_led() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outside = dir.path().join("outside");
    let runner = Planter {
        inner: FakeRunner::new(),
        outside,
    };
    let report = execute(
        &request_for(NESTED, dir.path()),
        &runner,
        &CancelToken::default(),
    );

    let rendered = crate::record::render_run_md(&report);
    assert!(rendered.contains("failed at `design`"), "{rendered}");
    assert!(rendered.contains("resolves through a link"), "{rendered}");
}

/// One `execute` of [`SHAPED`] with the node writing `text`.
fn shaped_run(text: &str) -> (RunOutcome, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let runner = FakeRunner::new().script(
        "design",
        vec![Step::Ok {
            writes: Some(text.to_string()),
        }],
    );
    let report = execute(
        &request_for(SHAPED, dir.path()),
        &runner,
        &CancelToken::default(),
    );
    (report.outcome, dir)
}

/// The likeliest miss: the node writes the prose it would have written
/// anyway. The file is there and non-empty, so only the declared shape
/// refuses it — and the run says so by name.
#[test]
fn prose_where_a_shape_was_declared_fails_the_node() {
    let (outcome, _dir) = shaped_run("The verdict is: pass.\n");
    let RunOutcome::Failed { node, failure } = &outcome else {
        panic!("prose is not the declared shape: {outcome:?}");
    };
    assert_eq!(node, "design");
    assert!(
        matches!(failure, NodeFailure::OutputSchema { .. }),
        "{failure:?}"
    );
    assert!(
        failure
            .to_string()
            .contains("does not match its output schema"),
        "the record has to say which promise was broken: {failure}"
    );
}

/// Valid JSON of the wrong shape is the same refusal, and the failure line
/// names the path inside the value rather than only the file.
#[test]
fn json_of_the_wrong_shape_fails_the_node_and_names_the_path() {
    let (outcome, _dir) = shaped_run("{\"verdict\": 7}\n");
    let RunOutcome::Failed { failure, .. } = &outcome else {
        panic!("a number is not one of the listed verdicts: {outcome:?}");
    };
    assert!(
        failure.to_string().contains("$.verdict: expected string"),
        "{failure}"
    );
}

/// And a node that keeps its promise passes — extra properties included,
/// because the schema reaches the agent as prose and invented fields are what
/// that gets.
#[test]
fn json_matching_the_declared_shape_passes() {
    let (outcome, _dir) = shaped_run("{\"verdict\": \"pass\", \"notes\": \"invented\"}\n");
    assert!(matches!(outcome, RunOutcome::Done), "{outcome:?}");
}

/// The regression the refusals must not cost: a node that really writes its
/// nested output still passes, and the directory is still created for it.
#[test]
fn a_plain_nested_output_still_passes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runner = FakeRunner::new();
    let report = execute(
        &request_for(NESTED, dir.path()),
        &runner,
        &CancelToken::default(),
    );

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    let output = report.run_dir.join("reports/out.md");
    assert!(
        std::fs::symlink_metadata(&output)
            .expect("the output is on disk")
            .file_type()
            .is_file(),
        "a real write must still land as a plain file"
    );
    assert_eq!(runner.ids(), vec!["setup", "design"]);
}
