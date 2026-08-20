//! What a run leaves on disk and what it says while it runs: the
//! resolved spec, the record, the `.gitignore`, and the event stream.
//! All driven through `execute`, which is the only entry point that
//! does the filesystem ceremony around the drive loop.

use super::*;

/// The flow whose `defaults` block is the thing `run.yaml` has to pay back:
/// one timeout and one agent, inherited by nodes that never name either.
/// Every axis `to_flow_file` has to carry, in one flow.
///
/// The round trip below is the only thing standing between the wire types
/// and the model — a field added to one and forgotten in the conversion
/// disappears from `run.yaml` silently. A fixture that names half the axes
/// guards half of them, so this one names all of them: both prompt shapes,
/// both `on_fail` shapes and everything inside them, and every agent axis
/// including the two `defaults` fills in.
const INHERITING: &str = "\
version: 1
defaults:
  agent: { id: claude, mode: bypassPermissions }
  timeout: 4m
nodes:
  - id: design
    kind: agent
    agent:
      id: claude
      mode: bypassPermissions
      model: opus
      effort: high
      permission: allow_once
    output: design.md
    prompt: write it
    on_fail:
      retry:
        hint: try harder after {{failure}}
        max_attempts: 3
        wait: 2s
  - id: gate
    kind: command
    deps: [design]
    run: \"true\"
    on_fail:
      repair:
        fix: fix it from {{attempts}}
        rerun: [design]
        max_attempts: 4
        wait: 1s
";

/// Run a flow and hand back what landed in its `run.yaml`.
fn run_yaml_of(text: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let runner = FakeRunner::new();
    let report = execute(
        &request_for(text, dir.path()),
        &runner,
        &CancelToken::default(),
    );
    std::fs::read_to_string(report.run_dir.join("run.yaml")).expect("run.yaml is written")
}

/// `defaults` is gone by the time a run starts, so the record has to show
/// what each node actually resolved to. Asserting that the file *is* a flow
/// file, and one that loads back to the same thing, is what makes that
/// checkable — a substring assertion would pass on a verbatim copy of the
/// source, which is precisely the un-resolved form.
#[test]
fn run_yaml_reloads_as_the_same_resolved_flow() {
    let loaded = load(INHERITING, None).expect("valid flow");
    let text = run_yaml_of(INHERITING);
    let reloaded = load(&text, None).expect("run.yaml is itself a flow file");
    assert_eq!(reloaded.flow(), loaded.flow(), "{text}");
}

/// Reloading is not enough on its own: the parser also accepts YAML the flow
/// file's grammar never uses. `yaml_serde` spells an externally tagged enum
/// as `on_fail: !repair`, and a resolved `Duration` as `secs`/`nanos` — both
/// round-trip and neither is what anyone writes, which is the whole reason
/// this file is built from the parse model rather than the resolved one.
#[test]
fn run_yaml_is_written_in_the_flow_files_own_vocabulary() {
    let text = run_yaml_of(GATED);
    assert!(text.contains("repair:"), "the policy keeps its key: {text}");
    assert!(!text.contains('!'), "a YAML tag is not the grammar: {text}");
    assert!(text.contains("timeout: 10m"), "{text}");
    assert!(!text.contains("secs"), "{text}");
}

/// The round trip alone would also pass on a verbatim copy of the source,
/// because the source resolves to the same flow. What separates them is that
/// nothing is left to inherit: every node states its own settings.
///
/// `defaults` does not vanish, though. `Flow::default_agent` is the agent a
/// repair's `fix` runs as, and no node can say that for itself — dropping it
/// would resolve to `None` on reload for any flow whose nodes disagree, so
/// the file keeps it and the assertions below name it as the one exception.
#[test]
fn run_yaml_leaves_nothing_for_a_node_to_inherit() {
    let text = run_yaml_of(INHERITING);
    let file = crate::parse::parse_flow_file(&text).expect("run.yaml parses");

    // `4m` came from `defaults.timeout`; both nodes must now say it
    // themselves, in the units a human wrote.
    assert_eq!(file.defaults.timeout, None, "{text}");
    assert_eq!(text.matches("4m").count(), 2, "{text}");
    assert!(
        file.nodes
            .iter()
            .all(|n| n.timeout == Some(std::time::Duration::from_secs(240))),
        "{text}"
    );

    // The agent axis the same way: the node names every part of it, and the
    // copy left in `defaults` is only there for a repair session.
    let crate::parse::NodeKindFile::Agent { agent, .. } = &file.nodes[0].kind else {
        panic!("expected an agent node: {text}");
    };
    let agent = agent.as_ref().expect("the node names its own agent");
    assert_eq!(agent.id.as_deref(), Some("claude"), "{text}");
    assert_eq!(agent.mode.as_deref(), Some("bypassPermissions"), "{text}");
    assert_eq!(
        agent.permission,
        Some(crate::parse::PermissionPolicyFile::AllowOnce),
        "{text}"
    );
    assert_eq!(
        file.defaults.agent.as_ref().and_then(|a| a.id.as_deref()),
        Some("claude"),
        "the repair agent is the one thing a node cannot state: {text}"
    );
}

/// Written at the start, not the end: a run that was killed mid-way is
/// exactly the one whose settings someone needs to look up.
///
/// Asserting after `execute` returns proves nothing — a `run.yaml` written at
/// the very end passes that too. The only witness to "before the first node"
/// is a runner that looks while it is being called.
#[test]
fn run_yaml_is_on_disk_before_the_first_node_runs() {
    /// Records whether `run.yaml` existed at each call, then delegates.
    struct Peeker(FakeRunner, std::cell::RefCell<Vec<bool>>);

    impl Peeker {
        fn peek(&self, ctx: &RunContext<'_>) {
            self.1
                .borrow_mut()
                .push(ctx.run_dir.join("run.yaml").is_file());
        }
    }

    impl NodeRunner for Peeker {
        fn run_agent<'a>(
            &'a self,
            ctx: &'a RunContext<'a>,
            agent: &'a crate::model::AgentSpec,
            prompt: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
            self.peek(ctx);
            self.0.run_agent(ctx, agent, prompt)
        }

        fn run_command<'a>(
            &'a self,
            ctx: &'a RunContext<'a>,
            run: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
            self.peek(ctx);
            self.0.run_command(ctx, run)
        }
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let peeker = Peeker(FakeRunner::new(), std::cell::RefCell::new(Vec::new()));
    let report = execute(
        &request_for(CHAIN, dir.path()),
        &peeker,
        &CancelToken::default(),
    );
    assert!(matches!(report.outcome, RunOutcome::Done), "{report:?}");
    assert_eq!(
        peeker.1.into_inner(),
        vec![true, true, true],
        "the spec has to be on disk before anything can fail"
    );
}

/// The event's variant, which is what a sequence assertion is about — the
/// payloads are checked separately, where they matter.
fn name_of(event: &FlowEvent) -> &'static str {
    match event {
        FlowEvent::RunStarted { .. } => "RunStarted",
        FlowEvent::NodeStarted { .. } => "NodeStarted",
        FlowEvent::NodePassed { .. } => "NodePassed",
        FlowEvent::NodeFailed { .. } => "NodeFailed",
        FlowEvent::FixStarted { .. } => "FixStarted",
        FlowEvent::FixEnded { .. } => "FixEnded",
        FlowEvent::Rerunning { .. } => "Rerunning",
        FlowEvent::RunEnded { .. } => "RunEnded",
    }
}

/// Run through `execute` with a subscriber attached, and hand back the report
/// alongside everything the stream said.
fn run_watched(
    text: &str,
    runner: &dyn NodeRunner,
    budget: Budget,
) -> (RunReport, Vec<FlowEvent>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let (tx, rx) = smol::channel::unbounded();
    let mut request = request_for(text, dir.path());
    request.budget = budget;
    request.events = Some(tx);
    let report = execute(&request, runner, &CancelToken::default());
    // Drained rather than awaited: the run is over and the sender is still
    // alive inside `request`, so there is no close to wait for.
    let events = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    (report, events, dir)
}

fn last_run_end(events: &[FlowEvent]) -> RunEnd {
    events
        .iter()
        .rev()
        .find_map(|event| match event {
            FlowEvent::RunEnded { end } => Some(end.clone()),
            _ => None,
        })
        .expect("the run ended")
}

/// The stream has to explain the repair path on its own — a host that only
/// saw NodeStarted/NodeFailed would show `review` passing, then silently
/// starting again with no reason given.
#[test]
fn the_event_stream_narrates_a_repair() {
    let runner = FakeRunner::new().script(
        "gate",
        vec![
            Step::fail(NodeFailure::Exit { code: Some(1) }),
            Step::Ok { writes: None },
        ],
    );
    let (report, events, _dir) = run_watched(GATED, &runner, Budget::unlimited());
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );

    let shape: Vec<&str> = events.iter().map(name_of).collect();
    assert_eq!(
        shape,
        vec![
            "RunStarted",
            "NodeStarted",
            "NodePassed", // implement
            "NodeStarted",
            "NodePassed", // review
            "NodeStarted",
            "NodeFailed", // gate, attempt 1
            // The fix is a real agent session and can take minutes. With no
            // event for it the UI sits on `NodeFailed` and looks hung.
            "FixStarted",
            "FixEnded",
            "Rerunning",
            "NodeStarted",
            "NodePassed", // review, re-derived
            "NodeStarted",
            "NodePassed", // gate, attempt 2
            "RunEnded",
        ],
        "{events:#?}"
    );

    // The list a host draws, in the order it draws it — the fix is not in it,
    // which is why it gets events of its own.
    match &events[0] {
        FlowEvent::RunStarted { run_dir, nodes } => {
            assert_eq!(run_dir, &report.run_dir);
            assert_eq!(nodes, &["implement", "review", "gate"]);
        }
        other => panic!("expected RunStarted, got {other:?}"),
    }
    // Naming the gate is what makes the fix renderable at all.
    assert!(
        matches!(&events[7], FlowEvent::FixStarted { gate } if gate == "gate"),
        "{:?}",
        events[7]
    );
    assert!(
        matches!(&events[8], FlowEvent::FixEnded { gate, failure } if gate == "gate" && failure.is_none()),
        "{:?}",
        events[8]
    );
    // The one transition a host cannot infer: these are pending again.
    match &events[9] {
        FlowEvent::Rerunning { gate, members } => {
            assert_eq!(gate, "gate");
            assert_eq!(members, &["review"]);
        }
        other => panic!("expected Rerunning, got {other:?}"),
    }
    // The gate's second attempt is numbered, so a host can tell a re-derived
    // node from a repeated one.
    assert!(
        matches!(
            &events[12],
            FlowEvent::NodeStarted { node, attempt: 2 } if node == "gate"
        ),
        "{:?}",
        events[12]
    );
}

/// A host that stops listening must not stop the run. Dropping the receiver
/// is the normal way a UI closes, not an error.
#[test]
fn a_dropped_receiver_does_not_disturb_the_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (tx, rx) = smol::channel::unbounded();
    drop(rx);
    let mut request = request_for(CHAIN, dir.path());
    request.events = Some(tx);

    let runner = FakeRunner::new();
    let report = execute(&request, &runner, &CancelToken::default());
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert!(report.warnings().is_empty(), "{:?}", report.warnings());
    assert_eq!(runner.ids(), vec!["design", "test", "review"]);
}

/// A flow with a file-backed prompt. The file exists at submission — a
/// missing one never reaches the scheduler, because `execute` validates the
/// request first — so the only way left to an `Io` is one that vanishes
/// between then and the node, which `Vanisher` below does.
const FILE_PROMPT: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: setup
    kind: command
    run: \"true\"
  - id: design
    kind: agent
    deps: [setup]
    output: design.md
    prompt_file: prompt.md
";

/// Deletes the next node's prompt while an earlier one runs. The scheduler
/// reads a prompt *before* calling the runner, so a file that vanishes has
/// to vanish during someone else's turn — which is also how it would really
/// happen.
struct Vanisher(std::path::PathBuf, FakeRunner);

impl NodeRunner for Vanisher {
    fn run_agent<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        agent: &'a crate::model::AgentSpec,
        prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        self.1.run_agent(ctx, agent, prompt)
    }

    fn run_command<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        run: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        let _ = std::fs::remove_file(&self.0);
        self.1.run_command(ctx, run)
    }
}

/// The three endings the marker cannot tell apart. A subscriber that only got
/// `Failed` would still have to open the directory to find out which, which is
/// the polling this channel exists to remove.
#[test]
fn run_ended_distinguishes_what_the_marker_folds_together() {
    /// One way of reaching `FAILED`, and what the stream must say instead.
    struct Case {
        label: &'static str,
        flow: &'static str,
        runner: FakeRunner,
        budget: Budget,
        ends_as: fn(&RunEnd) -> bool,
    }

    let cases = [
        Case {
            label: "a refused node",
            flow: CHAIN,
            runner: FakeRunner::new().script("design", vec![Step::fail(NodeFailure::Refused)]),
            budget: Budget::unlimited(),
            ends_as: |end| {
                matches!(end, RunEnd::Failed { node, failure }
                    if node == "design" && *failure == NodeFailure::Refused)
            },
        },
        Case {
            label: "an exhausted budget",
            flow: CHAIN,
            runner: FakeRunner::new(),
            budget: Budget {
                max_node_runs: Some(2),
                ..Budget::unlimited()
            },
            ends_as: |end| {
                matches!(
                    end,
                    RunEnd::BudgetExhausted {
                        limit: BudgetLimit::NodeRuns
                    }
                )
            },
        },
    ];

    for Case {
        label,
        flow,
        runner,
        budget,
        ends_as,
    } in cases
    {
        let (report, events, _dir) = run_watched(flow, &runner, budget);
        // All three write the same marker…
        assert_eq!(
            crate::marker::run_status(&report.run_dir, &|_| true),
            crate::marker::RunStatus::Failed,
            "{label}"
        );
        // …and the stream still tells them apart.
        let end = last_run_end(&events);
        assert!(ends_as(&end), "{label}: {end:?}");
    }
}

/// A run that never took the directory started nothing, so it narrates
/// nothing — a `RunEnded` here would tell a host a run it never saw start had
/// finished.
#[test]
fn a_run_that_loses_the_lock_narrates_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    // The lock lives in the runs directory, beside the run dirs — the one
    // place `.gitignore` covers, so it stays out of the user's `git status`.
    let runs_dir = dir.path().join(".daruda/flow-runs");
    std::fs::create_dir_all(&runs_dir).expect("mkdir");
    let held = RunLock::acquire(&runs_dir, "other", &|_| true).expect("free");
    let (tx, rx) = smol::channel::unbounded();
    let mut request = request_for(CHAIN, dir.path());
    request.events = Some(tx);

    let runner = FakeRunner::new();
    let report = execute(&request, &runner, &CancelToken::default());
    assert!(
        matches!(report.outcome, RunOutcome::LockHeld { .. }),
        "{:?}",
        report.outcome
    );
    let events: Vec<FlowEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert!(events.is_empty(), "{events:?}");
    held.release().expect("the original holder still owns it");
}

/// The account sits beside the marker, and a run that failed is exactly the
/// one someone opens it for — so it is written on that path too, and a write
/// that could not happen is a warning rather than a different outcome.
#[test]
fn a_failed_run_still_leaves_its_own_account() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runner = FakeRunner::new().script(
        "design",
        vec![Step::fail(NodeFailure::Exit { code: Some(2) })],
    );
    let report = execute(
        &request_for(CHAIN, dir.path()),
        &runner,
        &CancelToken::default(),
    );
    let text = std::fs::read_to_string(report.run_dir.join("run.md")).expect("run.md is written");
    assert!(text.contains("design"), "{text}");
    assert!(text.contains("exited with status 2"), "{text}");
    assert!(report.run_dir.join("FAILED").is_file());
    assert!(report.warnings().is_empty(), "{:?}", report.warnings());
}

/// Design §10 asks `run.md` to name the rerun set. Without it the record
/// shows a re-derivation only by implication — a node appearing twice at
/// attempt 1 with different evidence ids — leaving the reader to guess
/// which failure caused it.
#[test]
fn a_gate_failure_records_the_set_it_invalidated() {
    let runner = FakeRunner::new().script(
        "gate",
        vec![
            Step::fail(NodeFailure::Exit { code: Some(1) }),
            Step::Ok { writes: None },
        ],
    );
    let (report, _dir) = run(GATED, &runner);

    let gate = report.node("gate").expect("gate ran");
    let mut invalidated = gate.attempts[0].invalidated.nodes.clone();
    invalidated.sort();
    assert_eq!(
        invalidated,
        vec![NodeId::from("gate"), NodeId::from("review")],
        "the declared root and the gate itself"
    );
    // A passing attempt invalidates nothing, so the line stays off it.
    assert!(gate.attempts[1].invalidated.nodes.is_empty());

    let md = crate::record::render_run_md(&report);
    assert!(md.contains("re-derived"), "{md}");
    assert!(md.contains("`review`"), "{md}");
}

/// The `.gitignore` covers the runs directory, so anything the engine
/// leaves outside it lands in the user's `git status`. The lock is the one
/// candidate — it is taken before the run directory exists — and it has to
/// be caught *during* the run: `execute` removes it on the way out, so the
/// end state looks clean either way.
#[test]
fn nothing_the_engine_makes_sits_outside_the_directory_it_hides() {
    /// Lists the working directory on each call, minus what the engine is
    /// allowed to create there.
    struct Watcher(FakeRunner, std::cell::RefCell<Vec<String>>);

    impl Watcher {
        fn look(&self, ctx: &RunContext<'_>) {
            let stray = std::fs::read_dir(ctx.cwd)
                .expect("readable")
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name != ".daruda");
            self.1.borrow_mut().extend(stray);
        }
    }

    impl NodeRunner for Watcher {
        fn run_agent<'a>(
            &'a self,
            ctx: &'a RunContext<'a>,
            agent: &'a crate::model::AgentSpec,
            prompt: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
            self.look(ctx);
            self.0.run_agent(ctx, agent, prompt)
        }

        fn run_command<'a>(
            &'a self,
            ctx: &'a RunContext<'a>,
            run: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
            self.look(ctx);
            self.0.run_command(ctx, run)
        }
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let watcher = Watcher(FakeRunner::new(), std::cell::RefCell::new(Vec::new()));
    let report = execute(
        &request_for(CHAIN, dir.path()),
        &watcher,
        &CancelToken::default(),
    );
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );

    let stray = watcher.1.into_inner();
    assert!(
        stray.is_empty(),
        "left outside the hidden directory while running: {stray:?}"
    );
}

/// The status a host asks for while a run is going. Every other status
/// test builds its own directory layout; this one asks about a run that
/// `execute` actually set up, which is what catches `run_status` and the
/// lock disagreeing about where the lock lives.
#[test]
fn a_run_in_flight_reads_as_running_in_the_layout_execute_builds() {
    /// Asks the question mid-run, when there is no marker yet and the lock
    /// is the only evidence.
    struct Asker(
        FakeRunner,
        std::cell::RefCell<Vec<crate::marker::RunStatus>>,
    );

    impl Asker {
        fn ask(&self, ctx: &RunContext<'_>) {
            self.1
                .borrow_mut()
                .push(crate::marker::run_status(ctx.run_dir, &|_| true));
        }
    }

    impl NodeRunner for Asker {
        fn run_agent<'a>(
            &'a self,
            ctx: &'a RunContext<'a>,
            agent: &'a crate::model::AgentSpec,
            prompt: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
            self.ask(ctx);
            self.0.run_agent(ctx, agent, prompt)
        }

        fn run_command<'a>(
            &'a self,
            ctx: &'a RunContext<'a>,
            run: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
            self.ask(ctx);
            self.0.run_command(ctx, run)
        }
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let asker = Asker(FakeRunner::new(), std::cell::RefCell::new(Vec::new()));
    let report = execute(
        &request_for(CHAIN, dir.path()),
        &asker,
        &CancelToken::default(),
    );
    assert!(matches!(report.outcome, RunOutcome::Done));

    let seen = asker.1.into_inner();
    assert_eq!(
        seen,
        vec![crate::marker::RunStatus::Running; 3],
        "a live run reads as Unknown when the lock is looked for elsewhere"
    );
}

/// A `prompt_file` path is wrong twice over in a record: it resolves
/// against the flow file's directory, not the run directory this lands in,
/// and the file it names can change afterwards. The text the agent was
/// actually handed is what the audit is for.
#[test]
fn run_yaml_inlines_a_file_prompt_instead_of_naming_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("design.md"), "write the design").expect("write");
    let request = request_for(
        "\
version: 1
defaults: { agent: { id: claude, mode: bypassPermissions } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt_file: design.md
",
        dir.path(),
    );
    let runner = FakeRunner::new();
    let report = execute(&request, &runner, &CancelToken::default());
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );

    let text = std::fs::read_to_string(report.run_dir.join("run.yaml")).expect("written");
    assert!(text.contains("write the design"), "{text}");
    assert!(!text.contains("prompt_file"), "{text}");

    // And it survives the source moving on, which a path never would.
    std::fs::write(dir.path().join("design.md"), "something else").expect("rewrite");
    let after = std::fs::read_to_string(report.run_dir.join("run.yaml")).expect("still there");
    assert_eq!(after, text);
}

/// The third ending the marker folds into `FAILED`, split out because it
/// needs a runner that reaches into the flow's own directory.
///
/// A prompt file missing at submission never gets this far — `execute`
/// validates first — so the run has to lose the file while it is going.
#[test]
fn an_io_failure_writes_the_same_marker_and_still_says_what_it_was() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prompt = dir.path().join("prompt.md");
    std::fs::write(&prompt, "write it").expect("seeded");

    let (tx, rx) = smol::channel::unbounded();
    let mut request = request_for(FILE_PROMPT, dir.path());
    request.events = Some(tx);
    let report = execute(
        &request,
        &Vanisher(prompt, FakeRunner::new()),
        &CancelToken::default(),
    );
    let events: Vec<FlowEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect();

    assert_eq!(
        crate::marker::run_status(&report.run_dir, &|_| true),
        crate::marker::RunStatus::Failed
    );
    let end = last_run_end(&events);
    assert!(
        matches!(&end, RunEnd::Io { path, .. } if path.ends_with("prompt.md")),
        "{end:?}"
    );
}

/// A flow with two ways to run it. `run.yaml` records the settings, but
/// only `run.md` records *which named way* produced them.
const PROFILED: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
    model: sonnet
profiles:
  cheap:
    agent:
      model: haiku
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write it
";

/// Which profile a run used is not in `run.yaml` — that file states the
/// resolved settings and deliberately carries no profile list — so `run.md`
/// is the only place it is written down. Two runs of one flow under two
/// profiles are otherwise indistinguishable afterwards.
#[test]
fn the_record_says_which_profile_the_run_used() {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = execute(
        &request_for_profile(PROFILED, Some("cheap"), dir.path()),
        &FakeRunner::new(),
        &CancelToken::default(),
    );
    assert_eq!(report.provenance.profile.as_deref(), Some("cheap"));
    let rendered = crate::record::render_run_md(&report);
    assert!(
        rendered.contains("**Profile** — `cheap`"),
        "the report knows, the record does not say: {rendered}"
    );
}

/// A run with no profile says nothing about one, rather than inventing a
/// name for the file as written.
#[test]
fn a_run_without_a_profile_leaves_the_line_out() {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = execute(
        &request_for(PROFILED, dir.path()),
        &FakeRunner::new(),
        &CancelToken::default(),
    );
    assert_eq!(report.provenance.profile, None);
    assert!(!crate::record::render_run_md(&report).contains("**Profile**"));
}

/// A run writes down what it finished **as it goes**. Checked through a
/// real `execute` rather than against the journal module, because the
/// question is whether the scheduler reaches it at all — a journal nothing
/// calls is a resume that always starts over.
#[test]
fn a_run_leaves_a_journal_of_what_it_finished() {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = execute(
        &request_for_profile(PROFILED, Some("cheap"), dir.path()),
        &FakeRunner::new(),
        &CancelToken::default(),
    );

    let replay = crate::journal::read(&report.run_dir);
    assert_eq!(replay.passed, vec![NodeId::from("design")]);
    assert_eq!(replay.profile.as_deref(), Some("cheap"));
    assert_eq!(replay.spent.node_runs, report.node_runs);
    assert!(!replay.torn);
}

/// One node that fails its first attempt and passes its second, so the
/// journal has a settled failure to carry as well as a pass.
const RETRYING: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write it
    on_fail:
      retry:
        hint: try again after {{failure}}
        max_attempts: 2
";

/// Every attempt, not just the passing ones — a resumed run's `run.md` has
/// to show the failures the earlier process saw, or the account of the run
/// starts at the resume.
#[test]
fn the_journal_keeps_the_attempts_that_failed_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runner = FakeRunner::new().script(
        "design",
        vec![
            Step::fail(NodeFailure::TurnFailed("the adapter gave up".to_string())),
            Step::Ok {
                writes: Some("done\n".to_string()),
            },
        ],
    );
    let report = execute(
        &request_for(RETRYING, dir.path()),
        &runner,
        &CancelToken::default(),
    );

    let replay = crate::journal::read(&report.run_dir);
    let design = replay
        .records
        .iter()
        .find(|r| r.id == "design")
        .expect("the node has a history");
    assert_eq!(design.attempts.len(), 2, "{:?}", design.attempts);
    assert!(
        matches!(
            design.attempts[0].outcome,
            crate::record::AttemptOutcome::Reported(_)
        ),
        "the first attempt's failure did not survive: {:?}",
        design.attempts[0].outcome
    );
}
