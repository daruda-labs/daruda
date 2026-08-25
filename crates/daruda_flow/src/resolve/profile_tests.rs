//! What a profile layer replaces, and what it leaves alone.

use super::*;
use crate::parse::parse_flow_file;

/// `defaults` names the agent and the mode; the profile changes one
/// axis; one node pins another. Every layer is visible in the result.
const LAYERED: &str = "\
version: 1
defaults:
  timeout: 5m
  agent:
    id: claude
    mode: default
    model: sonnet
    permission: ask
profiles:
  cheap:
    timeout: 1m
    agent:
      model: haiku
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
  - id: b
    kind: agent
    output: b.md
    prompt: write
    agent:
      model: opus
";

fn resolved(profile: Option<&str>) -> Flow {
    resolve(parse_flow_file(LAYERED).expect("parses"), profile).expect("resolves")
}

fn model_of(flow: &Flow, id: &str) -> Option<String> {
    flow.nodes
        .iter()
        .find(|n| n.id == id)
        .and_then(|n| match &n.kind {
            NodeKind::Agent { agent, .. } => agent.model.clone(),
            NodeKind::Command { .. } => None,
        })
}

/// No profile named, nothing changes — the layer costs the flows that
/// do not use it nothing.
#[test]
fn a_run_without_a_profile_resolves_exactly_as_before() {
    let flow = resolved(None);
    assert_eq!(model_of(&flow, "a").as_deref(), Some("sonnet"));
    assert_eq!(flow.nodes[0].timeout, Duration::from_secs(300));
}

/// `defaults` ← `profile` ← `node`. The profile beats `defaults`, and
/// the node beats the profile — a profile that won over the nodes
/// would make the file unreadable on its own, because what a node does
/// would depend on which profile someone picked.
#[test]
fn a_profile_beats_defaults_and_a_node_beats_the_profile() {
    let flow = resolved(Some("cheap"));
    assert_eq!(
        model_of(&flow, "a").as_deref(),
        Some("haiku"),
        "the profile did not reach a node that pins nothing"
    );
    assert_eq!(
        model_of(&flow, "b").as_deref(),
        Some("opus"),
        "the profile overrode a node's own choice"
    );
}

/// A profile names one axis and inherits the rest. Nothing it leaves
/// out is cleared — a `model` change must not drop the mode that
/// decides whether anyone is ever asked.
#[test]
fn a_profile_only_replaces_what_it_names() {
    let flow = resolved(Some("cheap"));
    let agent = match &flow.nodes[0].kind {
        NodeKind::Agent { agent, .. } => agent.clone(),
        NodeKind::Command { .. } => panic!("agent node"),
    };
    assert_eq!(agent.id, "claude");
    assert_eq!(agent.mode.as_deref(), Some("default"));
    assert_eq!(agent.permission, PermissionPolicy::Ask);
    assert_eq!(flow.nodes[0].timeout, Duration::from_secs(60), "timeout");
}

/// A name the file does not declare is refused. Falling back to plain
/// `defaults` would run the flow with settings nobody picked — under a
/// profile named `unattended`, silently attended.
#[test]
fn a_profile_the_file_does_not_declare_is_refused_by_name() {
    let issues = resolve(parse_flow_file(LAYERED).expect("parses"), Some("nope"))
        .expect_err("an unknown profile is not a run");
    assert!(
        issues.iter().any(|i| matches!(
            &i.kind,
            ValidationKind::UnknownProfile { name } if name == "nope"
        )),
        "{issues:?}"
    );
}

/// The same rule a node follows: a layer that switches the agent has
/// to say the mode again, because the one it would inherit belongs to
/// the agent it just replaced.
#[test]
fn a_profile_that_switches_agent_without_a_mode_is_refused() {
    let file = parse_flow_file(
        "\
version: 1
defaults:
  agent:
    id: claude
    mode: default
profiles:
  other:
    agent:
      id: codex
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
",
    )
    .expect("parses");
    let issues = resolve(file, Some("other")).expect_err("refused");
    assert!(
        issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::AgentIdWithoutMode)),
        "{issues:?}"
    );
}

/// The record says what ran, so the menu of what could have run
/// instead has no place in it — every node already states the settings
/// the chosen profile produced.
#[test]
fn the_record_of_a_run_carries_no_profiles() {
    let flow = resolved(Some("cheap"));
    let written = to_flow_file(&flow, Path::new("/flow"));
    assert!(written.profiles.is_empty());
}

/// The host has to know the menu before it can offer it, which is
/// before anything can be merged.
#[test]
fn the_declared_profiles_are_readable_without_resolving() {
    assert_eq!(crate::load::profiles(LAYERED).expect("parses"), ["cheap"]);
}
