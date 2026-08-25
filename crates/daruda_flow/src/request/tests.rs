//! What a request has to carry before a run can start, asked of a real
//! filesystem — the paths, the catalog, and the two axes that say which
//! part of the flow to spend money on.

use super::*;

/// `execute` blocks, so a host runs it on a thread of its own and the
/// request has to cross. The two callbacks are the only thing that could
/// stop it, which is why they are declared `+ Send`.
#[test]
fn a_request_and_its_report_can_cross_a_thread() {
    fn assert_send<T: Send>() {}
    assert_send::<RunRequest>();
    assert_send::<crate::schedule::RunReport>();
}

use crate::error::ValidationKind;
use crate::load::load;

fn spec(command: &str) -> daruda_acp::LaunchSpec {
    daruda_acp::LaunchSpec {
        command: command.to_string(),
        strip_env: Vec::new(),
    }
}

fn request_for(text: &str, agents: &[&str], dir: &std::path::Path) -> RunRequest {
    let loaded = load(text, None).expect("valid flow");
    RunRequest {
        loaded,
        until: None,
        pinned: Vec::new(),
        cwd: dir.to_path_buf(),
        run_dir: dir.join("run"),
        flow_dir: dir.to_path_buf(),
        agents: agents.iter().map(|a| (a.to_string(), spec("x"))).collect(),
        node_install_dir: dir.to_path_buf(),
        budget: Budget::unlimited(),
        is_alive: Box::new(|_| true),
        git_status: None,
        events: None,
        ask: None,
        resume: None,
    }
}

const AGENT_FLOW: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
";

#[test]
fn an_agent_id_absent_from_the_catalog_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let req = request_for(AGENT_FLOW, &["codex"], dir.path());
    let issues = validate_request(&req);
    assert!(
        issues.iter().any(|i| matches!(&i.kind,
                ValidationKind::UnknownAgent { id } if id == "claude")),
        "{issues:?}"
    );
}

#[test]
fn a_known_agent_id_passes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let req = request_for(AGENT_FLOW, &["claude"], dir.path());
    assert_eq!(validate_request(&req), Vec::new());
}

/// `prompt_file` is relative to the flow file, not to the working
/// directory — a flow kept in `.daruda/flows/` names its prompts
/// beside itself.
#[test]
fn a_missing_prompt_file_is_rejected_and_a_present_one_is_not() {
    let dir = tempfile::tempdir().expect("tempdir");
    let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt_file: ./prompts/a.md
";
    let req = request_for(flow, &["claude"], dir.path());
    assert!(
        validate_request(&req)
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::MissingPromptFile { .. })),
        "the file does not exist yet"
    );

    std::fs::create_dir_all(dir.path().join("prompts")).expect("mkdir");
    std::fs::write(dir.path().join("prompts/a.md"), "hi").expect("write");
    assert_eq!(validate_request(&req), Vec::new());
}

/// File-backed prompts are relative to the flow file and must stay
/// under that directory. An absolute path that happens to exist is
/// still not a valid flow-local prompt.
#[test]
fn an_absolute_prompt_file_is_rejected_even_if_it_exists() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outside_dir = tempfile::tempdir().expect("outside tempdir");
    let outside = outside_dir.path().join("outside.md");
    std::fs::write(&outside, "hi").expect("write outside");
    let flow = format!(
        "\
version: 1
defaults: {{ agent: {{ id: claude }} }}
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt_file: {}
",
        outside.display()
    );
    let req = request_for(&flow, &["claude"], dir.path());
    assert!(
        validate_request(&req)
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::PromptFileOutsideFlowDir { .. })),
        "absolute paths must not be accepted just because they exist"
    );

    let parent_flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt_file: ../outside.md
";
    let req = request_for(parent_flow, &["claude"], dir.path());
    assert!(
        validate_request(&req)
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::PromptFileOutsideFlowDir { .. })),
        "`..` must be rejected before existence is checked"
    );
}

#[cfg(unix)]
#[test]
fn a_prompt_file_symlink_escaping_the_flow_dir_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outside_dir = tempfile::tempdir().expect("outside tempdir");
    let outside = outside_dir.path().join("outside.md");
    std::fs::write(&outside, "hi").expect("write outside");
    std::fs::create_dir_all(dir.path().join("prompts")).expect("mkdir");
    std::os::unix::fs::symlink(&outside, dir.path().join("prompts/link.md")).expect("symlink");

    let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt_file: ./prompts/link.md
";
    let req = request_for(flow, &["claude"], dir.path());
    assert!(
        validate_request(&req)
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::PromptFileOutsideFlowDir { .. })),
        "canonical containment must be checked after existence"
    );
}

/// A repair fix runs as `flow.default_agent`. A command-only flow can
/// therefore need an agent even though no node has kind `agent`.
#[test]
fn a_repair_default_agent_absent_from_the_catalog_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let flow = "\
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
    let req = request_for(flow, &["codex"], dir.path());
    assert!(
        validate_request(&req).iter().any(|i| matches!(&i.kind,
                ValidationKind::UnknownAgent { id } if id == "claude")),
        "the repair session's default agent must be checked"
    );
}

/// The inline form of this is rejected by `validate`; the file form
/// reached the runner untouched and rendered to an empty string, so the
/// flow silently ran with a hole in its prompt.
#[test]
fn an_unreachable_output_ref_inside_a_prompt_file_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("design.md"),
        "continue from {{node.review.output}}",
    )
    .expect("write");
    let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt_file: design.md
  - id: review
    kind: agent
    deps: [design]
    output: review.md
    prompt: read {{node.design.output}}
";
    let req = request_for(flow, &["claude"], dir.path());
    assert!(
        validate_request(&req).iter().any(|i| matches!(&i.kind,
                ValidationKind::UnreachableOutputRef { referenced } if referenced == "review")),
        "`review` is downstream of `design`, so its output cannot exist yet"
    );
}

/// A retry's `hint_file` is the same kind of reference and must be
/// checked the same way — it is read only on a failure path, which is
/// exactly when a missing file is most expensive to discover.
#[test]
fn a_missing_hint_file_is_rejected_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
    on_fail:
      retry:
        max_attempts: 2
        hint_file: ./missing-hint.md
";
    let req = request_for(flow, &["claude"], dir.path());
    assert!(
        validate_request(&req)
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::MissingPromptFile { .. }))
    );
}

const ASKS: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    agent: { id: claude, mode: default, permission: ask }
    output: a.md
    prompt: write
";

/// Only the *repair* can ask: no node names `ask`, but the fix session
/// runs as `defaults.agent` and inherits its policy. Checking nodes
/// alone lets this flow start and park with nobody to release it.
const REPAIR_ASKS: &str = "\
version: 1
defaults: { agent: { id: claude, mode: default, permission: ask } }
nodes:
  - id: gate
    kind: command
    run: \"true\"
    on_fail:
      repair:
        fix: fix it, see {{attempts}}
        max_attempts: 2
        wait: 0s
";

/// **A channel is not a capability.** `examples/run_flow.rs` hands over
/// an `events` sender and only *prints* what arrives — so keying this
/// check on `events` would let an `ask` flow start there and park until
/// somebody noticed. The port a host wires only to answer questions is
/// the one thing that means it can.
#[test]
fn an_ask_flow_is_refused_when_only_the_narration_channel_is_wired() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut req = request_for(ASKS, &["claude"], dir.path());
    let (tx, _rx) = smol::channel::unbounded();
    req.events = Some(tx);

    let kinds: Vec<_> = validate_request(&req).into_iter().map(|i| i.kind).collect();
    assert!(
        kinds.contains(&ValidationKind::NobodyToAsk),
        "an ask flow was accepted with nowhere to ask: {kinds:?}"
    );
}

/// The same refusal for a flow only a repair can make ask.
#[test]
fn a_flow_whose_only_asker_is_its_repair_is_refused_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let req = request_for(REPAIR_ASKS, &["claude"], dir.path());
    let kinds: Vec<_> = validate_request(&req).into_iter().map(|i| i.kind).collect();
    assert!(kinds.contains(&ValidationKind::NobodyToAsk), "{kinds:?}");
}

/// And a host that did wire the port runs.
#[test]
fn an_ask_flow_with_somewhere_to_ask_is_accepted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut req = request_for(ASKS, &["claude"], dir.path());
    let (tx, _rx) = smol::channel::unbounded();
    req.ask = Some(tx);

    let kinds: Vec<_> = validate_request(&req).into_iter().map(|i| i.kind).collect();
    assert!(!kinds.contains(&ValidationKind::NobodyToAsk), "{kinds:?}");
}

/// A flow nobody would ever ask about is unaffected — the check must
/// not become a reason every host needs an answering surface.
#[test]
fn a_flow_that_never_asks_needs_no_answering_surface() {
    let dir = tempfile::tempdir().expect("tempdir");
    let req = request_for(AGENT_FLOW, &["claude"], dir.path());
    let kinds: Vec<_> = validate_request(&req).into_iter().map(|i| i.kind).collect();
    assert!(!kinds.contains(&ValidationKind::NobodyToAsk), "{kinds:?}");
}

/// A pin is a promise that this output is valid. A promise about a file
/// that is gone would starve everything downstream, so it is refused
/// before the run rather than warned about during it.
#[test]
fn a_pin_whose_source_is_not_there_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut req = request_for(AGENT_FLOW, &["claude"], dir.path());
    req.pinned = vec![PinnedOutput {
        node: NodeId::from("a"),
        from: dir.path().join("gone.md"),
    }];
    let kinds: Vec<_> = validate_request(&req).into_iter().map(|i| i.kind).collect();
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, ValidationKind::PinnedSourceMissing { .. })),
        "{kinds:?}"
    );
}

#[test]
fn a_pin_naming_a_node_the_flow_lacks_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("there.md");
    std::fs::write(&source, "x").expect("write");
    let mut req = request_for(AGENT_FLOW, &["claude"], dir.path());
    req.pinned = vec![PinnedOutput {
        node: NodeId::from("nope"),
        from: source,
    }];
    let kinds: Vec<_> = validate_request(&req).into_iter().map(|i| i.kind).collect();
    assert!(
        kinds.contains(&ValidationKind::UnknownPin {
            node: NodeId::from("nope")
        }),
        "{kinds:?}"
    );
}

/// Silently running everything would spend the money the request was
/// trying not to spend, so a target that is not in the flow is refused.
#[test]
fn an_until_naming_a_node_the_flow_lacks_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut req = request_for(AGENT_FLOW, &["claude"], dir.path());
    req.until = Some(NodeId::from("nope"));
    let kinds: Vec<_> = validate_request(&req).into_iter().map(|i| i.kind).collect();
    assert!(
        kinds.contains(&ValidationKind::UnknownUntil {
            node: NodeId::from("nope")
        }),
        "{kinds:?}"
    );
}

#[test]
fn an_until_naming_a_real_node_is_accepted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut req = request_for(AGENT_FLOW, &["claude"], dir.path());
    req.until = Some(NodeId::from("a"));
    let kinds: Vec<_> = validate_request(&req).into_iter().map(|i| i.kind).collect();
    assert!(
        !kinds
            .iter()
            .any(|k| matches!(k, ValidationKind::UnknownUntil { .. })),
        "{kinds:?}"
    );
}

/// Iterating on a flow's head while its tail is half-written is the normal
/// state, and refusing the whole run over a node the selection stops before
/// is exactly what `until` exists to get past. The second half is what makes
/// this a claim about the selection rather than about the flow.
#[test]
fn a_defect_in_a_node_the_selection_skips_does_not_refuse_the_run() {
    const HEAD_THEN_A_BLANK_TAIL: &str = "\
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
    output: review.md
    prompt: \"\"
";
    let dir = tempfile::tempdir().expect("tempdir");
    let mut req = request_for(HEAD_THEN_A_BLANK_TAIL, &["claude"], dir.path());

    assert!(
        !validate_request(&req).is_empty(),
        "the whole flow does reach the blank prompt"
    );

    req.until = Some(NodeId::from("design"));
    assert_eq!(
        validate_request(&req),
        Vec::new(),
        "the blank prompt belongs to a node this run stops before"
    );
}

/// The other direction: a selection is not a licence. A defect in a node the
/// run *does* reach is still refused, target included.
#[test]
fn a_defect_in_the_selected_node_is_still_refused() {
    const BLANK_HEAD: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: \"\"
  - id: review
    kind: agent
    deps: [design]
    output: review.md
    prompt: write
";
    let dir = tempfile::tempdir().expect("tempdir");
    let mut req = request_for(BLANK_HEAD, &["claude"], dir.path());
    req.until = Some(NodeId::from("design"));
    assert!(!validate_request(&req).is_empty());
}

/// A pin and a gate's `rerun` naming the same node ask for opposite things.
/// The scheduler honours the pin (a pinned node never enters `executed`, so
/// `rerun_members` drops it), which left the repair paying for a fix session
/// per attempt and re-testing an input nothing had touched — silently. The
/// choice belongs to the author, so the run is refused while it is still free.
#[test]
fn a_pin_that_would_make_a_repair_futile_is_refused() {
    const GATED: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
  - id: gate
    kind: command
    deps: [design]
    run: \"true\"
    on_fail:
      repair:
        fix: fix it from {{attempts}}
        rerun: [design]
        max_attempts: 2
";
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("design.md");
    std::fs::write(&source, "an earlier run's design\n").expect("write");

    let mut req = request_for(GATED, &["claude"], dir.path());
    assert_eq!(validate_request(&req), Vec::new(), "no pin, nothing to say");

    req.pinned = vec![PinnedOutput {
        node: NodeId::from("design"),
        from: source,
    }];
    let kinds: Vec<_> = validate_request(&req).into_iter().map(|i| i.kind).collect();
    assert_eq!(
        kinds,
        vec![ValidationKind::PinnedNodeInRerun {
            node: NodeId::from("design"),
            gate: NodeId::from("gate"),
        }],
        "{kinds:?}"
    );
}

/// The ask gate is a per-node question too, and it sat outside the loop that
/// follows the selection — so a run was refused for having no answering
/// surface because of a node it stops before, needing no surface at all.
#[test]
fn a_node_that_asks_beyond_the_selection_does_not_demand_an_answering_surface() {
    const TAIL_ASKS: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
  - id: risky
    kind: agent
    deps: [design]
    agent: { permission: ask, mode: default }
    output: risky.md
    prompt: write
";
    let dir = tempfile::tempdir().expect("tempdir");
    let mut req = request_for(TAIL_ASKS, &["claude"], dir.path());
    assert!(req.ask.is_none(), "the fixture has nowhere to ask");

    let asks = |req: &RunRequest| {
        validate_request(req)
            .into_iter()
            .any(|i| i.kind == ValidationKind::NobodyToAsk)
    };
    assert!(asks(&req), "the whole flow does reach the asking node");

    req.until = Some(NodeId::from("design"));
    assert!(!asks(&req), "`risky` is not in this run");
}

/// The money hole this closes: a node the graph editor made and nobody
/// typed into opens a paid session and asks it nothing.
#[test]
fn a_blank_prompt_is_refused_before_a_session_is_paid_for() {
    const BLANK: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: '   '
";
    let dir = tempfile::tempdir().expect("tempdir");
    let req = request_for(BLANK, &["claude"], dir.path());
    let kinds: Vec<_> = validate_request(&req).into_iter().map(|i| i.kind).collect();
    assert!(kinds.contains(&ValidationKind::EmptyPrompt), "{kinds:?}");
}

/// And a prompt that says something is left alone, so the rule cannot
/// become a reason a real flow is refused.
#[test]
fn a_prompt_with_words_in_it_is_not_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let req = request_for(AGENT_FLOW, &["claude"], dir.path());
    let kinds: Vec<_> = validate_request(&req).into_iter().map(|i| i.kind).collect();
    assert!(!kinds.contains(&ValidationKind::EmptyPrompt), "{kinds:?}");
}
