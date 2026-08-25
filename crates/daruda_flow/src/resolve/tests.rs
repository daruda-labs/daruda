//! Merging asked from both sides: a file resolved into a flow, and a flow
//! written back out. A field added to one direction and forgotten in the
//! other is what these fixtures exist to catch.

use crate::error::FlowError;
use crate::load::load;

/// The trap a real run walked into: `permission: ask` with no `mode`.
///
/// Which modes prompt is the adapter's business, so nothing here can
/// say "bypassPermissions never asks". What it can say is that the axis
/// deciding whether anyone is *ever* asked was left unset — and the
/// mode daruda itself defaults to is one that never prompts, so the
/// silent outcome is "you are not asked, and nothing tells you".
#[test]
fn asking_without_a_mode_is_refused() {
    let issues = load(
        "\
version: 1
defaults: { agent: { id: claude, permission: ask } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
",
        None,
    )
    .expect_err("a flow that asks with no mode must not resolve");
    let FlowError::Validate(issues) = issues else {
        panic!("expected validation issues, got {issues:?}");
    };
    assert!(
        issues
            .iter()
            .any(|i| i.kind == ValidationKind::AskWithoutMode),
        "{issues:?}"
    );
}

/// Naming the mode is the whole of the fix.
#[test]
fn asking_with_a_mode_resolves() {
    load(
        "\
version: 1
defaults: { agent: { id: claude, mode: default, permission: ask } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
",
        None,
    )
    .expect("naming the mode is all it takes");
}

/// A repair's `fix` runs as `defaults.agent` and inherits its policy,
/// so a flow of nothing but command nodes can still reach `ask`.
#[test]
fn a_repair_that_could_ask_needs_the_mode_too() {
    let err = load(
        "\
version: 1
defaults: { agent: { id: claude, permission: ask } }
nodes:
  - id: gate
    kind: command
    run: \"true\"
    on_fail:
      repair:
        fix: fix it, see {{attempts}}
        max_attempts: 2
        wait: 0s
",
        None,
    )
    .expect_err("the repair agent can ask, so the mode is required");
    let FlowError::Validate(issues) = err else {
        panic!("expected validation issues");
    };
    assert!(
        issues
            .iter()
            .any(|i| i.kind == ValidationKind::AskWithoutMode),
        "{issues:?}"
    );
}

/// A flow that never asks is unaffected — the rule must not become a
/// reason every flow has to pin a mode.
#[test]
fn a_flow_that_never_asks_needs_no_mode() {
    load(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
",
        None,
    )
    .expect("a flow with no `ask` keeps its freedom to omit the mode");
}
use super::*;
use crate::error::ValidationKind;
use crate::model::{NodeKind, PermissionPolicy};
use crate::parse::parse_flow_file;
use std::time::Duration;

const FLOW: &str = "\
version: 1
defaults:
  timeout: 10m
  agent:
    id: claude
    mode: bypassPermissions
    permission: deny
nodes:
  - id: design
    kind: agent
    agent: { effort: high }
    output: design.md
    prompt: write it
  - id: test
    kind: command
    deps: [design]
    timeout: 30s
    run: cargo test
";

#[test]
fn node_overrides_merge_field_by_field_against_defaults() {
    let flow = resolve(parse_flow_file(FLOW).unwrap(), None).expect("valid flow resolves");

    let design = &flow.nodes[0];
    assert_eq!(
        design.timeout,
        Duration::from_secs(600),
        "inherited from defaults"
    );
    match &design.kind {
        NodeKind::Agent { agent, .. } => {
            assert_eq!(agent.effort.as_deref(), Some("high"), "the node's own axis");
            assert_eq!(agent.id, "claude", "every unnamed axis comes from defaults");
            assert_eq!(agent.mode.as_deref(), Some("bypassPermissions"));
            assert_eq!(agent.permission, PermissionPolicy::Deny);
        }
        NodeKind::Command { .. } => panic!("expected an agent node"),
    }

    assert_eq!(
        flow.nodes[1].timeout,
        Duration::from_secs(30),
        "the node's own wins"
    );
}

#[test]
fn absent_timeout_falls_back_to_the_engine_default() {
    let flow = resolve(
        parse_flow_file("version: 1\nnodes:\n  - id: t\n    kind: command\n    run: \"true\"\n")
            .unwrap(),
        None,
    )
    .expect("valid flow resolves");
    assert_eq!(flow.nodes[0].timeout, DEFAULT_TIMEOUT);
}

#[test]
fn max_attempts_is_clamped_and_wait_defaults() {
    let flow = resolve(
        parse_flow_file(
            "\
version: 1
nodes:
  - id: t
    kind: command
    run: \"true\"
    on_fail:
      repair:
        fix: \"{{failure}}\"
        max_attempts: 99
",
        )
        .unwrap(),
        None,
    )
    .expect("valid flow resolves");
    match &flow.nodes[0].kind {
        NodeKind::Command { on_fail, .. } => match on_fail {
            crate::model::GateFail::Repair {
                max_attempts, wait, ..
            } => {
                assert_eq!(
                    *max_attempts, MAX_ATTEMPTS_RANGE.1,
                    "clamped to the ceiling"
                );
                assert_eq!(*wait, DEFAULT_WAIT);
            }
            crate::model::GateFail::Halt => panic!("expected repair"),
        },
        NodeKind::Agent { .. } => panic!("expected a command node"),
    }
}

#[test]
fn an_agent_node_without_any_agent_source_is_reported() {
    let issues = resolve(
        parse_flow_file(
            "\
version: 1
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write it
",
        )
        .unwrap(),
        None,
    )
    .expect_err("no defaults.agent and no node override");
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].node, Some(crate::NodeId::from("design")));
    assert!(matches!(issues[0].kind, ValidationKind::MissingAgent));
}

/// A flow with no `defaults.agent` still has an agent for a repair to
/// run as, so long as the agent nodes name the same resolved spec.
#[test]
fn the_default_agent_falls_back_when_agent_nodes_agree() {
    let flow = resolve(
        parse_flow_file(
            "\
version: 1
nodes:
  - id: a
    kind: agent
    agent: { id: codex, mode: auto }
    output: a.md
    prompt: write
  - id: b
    kind: agent
    agent: { id: codex, mode: auto }
    output: b.md
    prompt: write
",
        )
        .unwrap(),
        None,
    )
    .expect("valid flow");
    assert_eq!(flow.default_agent.map(|a| a.id), Some("codex".to_string()));
}

#[test]
fn a_command_only_flow_has_no_default_agent() {
    let flow = resolve(
        parse_flow_file("version: 1\nnodes:\n  - id: t\n    kind: command\n    run: \"true\"\n")
            .unwrap(),
        None,
    )
    .expect("valid flow");
    assert!(flow.default_agent.is_none());
}

#[test]
fn mixed_agent_nodes_have_no_implicit_repair_agent() {
    let flow = resolve(
        parse_flow_file(
            "\
version: 1
nodes:
  - id: a
    kind: agent
    agent: { id: claude, mode: bypassPermissions }
    output: a.md
    prompt: write
  - id: b
    kind: agent
    agent: { id: codex, mode: auto }
    output: b.md
    prompt: write
",
        )
        .unwrap(),
        None,
    )
    .expect("valid flow");
    assert!(flow.default_agent.is_none());
}

/// The two shapes `run.yaml`'s round trip depends on and the flagship
/// flow does not reach. Both are about `default_agent`, which is not a
/// copy of `defaults.agent`: it also falls back to the nodes, so a file
/// that dropped `defaults` would resolve to a different flow.
#[test]
fn a_regenerated_file_resolves_to_the_flow_it_came_from() {
    for (label, text) in [
        // Nodes disagree, so the node fallback yields `None` — only the
        // `defaults.agent` copy carries the repair agent across.
        (
            "a node that switches agents",
            "\
version: 1
defaults:
  agent: { id: claude, mode: bypassPermissions }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
  - id: b
    kind: agent
    agent: { id: codex, mode: auto }
    output: b.md
    prompt: write
",
        ),
        // A spec with no mode cannot state its own id — naming one
        // without a mode is a validation error — so the id has to stay
        // inherited from `defaults`.
        (
            "an agent with no mode",
            "\
version: 1
defaults:
  agent: { id: claude }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
",
        ),
    ] {
        let flow = resolve(parse_flow_file(text).unwrap(), None).expect("valid flow");
        let regenerated = yaml_serde::to_string(&to_flow_file(&flow, std::path::Path::new("")))
            .expect("a flow file serializes");
        let reloaded = resolve(parse_flow_file(&regenerated).expect("parses"), None)
            .unwrap_or_else(|issues| panic!("{label}: {issues:?}\n{regenerated}"));
        assert_eq!(reloaded, flow, "{label}: {regenerated}");
    }
}

/// Mode ids differ per agent, so a node that switches agents while
/// inheriting `defaults.agent.mode` would silently ask for a mode the
/// new agent never advertises. This is checked here rather than in
/// `crate::validate` because only the file's shape — still visible at
/// this point — says whether the node named the id itself.
#[test]
fn naming_an_agent_id_without_its_mode_is_reported() {
    let issues = resolve(
        parse_flow_file(
            "\
version: 1
defaults:
  agent: { id: claude, mode: bypassPermissions }
nodes:
  - id: a
    kind: agent
    agent: { id: codex }
    output: a.md
    prompt: write
",
        )
        .unwrap(),
        None,
    )
    .expect_err("the node names an id but no mode");
    assert!(
        issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::AgentIdWithoutMode))
    );
}
