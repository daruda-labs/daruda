//! The single place `defaults` and a node's overrides are combined. Field
//! by field, not whole-value: a node naming one axis keeps the defaults for
//! every other. Sealing this in one function is what keeps the same field
//! group from being assembled differently by two call paths.
//!
//! Issues found here are returned instead of a `Flow`, so a flow that fails
//! merging is never handed to `crate::validate` — see `crate::load`, which
//! reports whichever stage failed.

use crate::error::{ValidationIssue, ValidationKind};
use crate::model::{
    AgentFail, AgentSpec, Flow, GateFail, Node, NodeKind, PermissionPolicy, Prompt,
};
use crate::parse::{
    AgentFailFile, AgentOverride, Defaults, FlowFile, GateFailFile, HintSource, NodeFile,
    NodeKindFile, PermissionPolicyFile, PromptSource,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Engine defaults for axes the file leaves unset.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
pub const DEFAULT_WAIT: Duration = Duration::from_secs(5);
/// The `max_attempts` floor is 2, not 1: one attempt is what `halt` already
/// means, so allowing 1 would give the same behaviour two spellings.
pub const MAX_ATTEMPTS_RANGE: (u32, u32) = (2, 5);
/// A retry that waits a full minute is almost certainly a typo; the ceiling
/// keeps one from parking the run.
pub const WAIT_RANGE: (Duration, Duration) = (Duration::ZERO, Duration::from_secs(60));

/// Merge `defaults` into every node and produce the executable `Flow`.
pub fn resolve(file: FlowFile) -> Result<Flow, Vec<ValidationIssue>> {
    let mut issues = Vec::new();
    let mut nodes = Vec::with_capacity(file.nodes.len());

    for node in file.nodes {
        let timeout = node
            .timeout
            .or(file.defaults.timeout)
            .unwrap_or(DEFAULT_TIMEOUT);
        let kind = match node.kind {
            NodeKindFile::Command { run, on_fail } => NodeKind::Command {
                run,
                on_fail: resolve_gate_fail(on_fail),
            },
            NodeKindFile::Agent {
                agent: override_,
                prompt,
                output,
                on_fail,
            } => {
                // Checked here, not in `crate::validate`: after merging,
                // nothing says whether the id came from the node or from
                // `defaults`, and only the former needs its own mode.
                if override_.as_ref().and_then(|a| a.id.as_ref()).is_some()
                    && override_.as_ref().and_then(|a| a.mode.as_ref()).is_none()
                {
                    issues.push(ValidationIssue {
                        node: Some(node.id.clone()),
                        kind: ValidationKind::AgentIdWithoutMode,
                        message: "a node naming its own `agent.id` must also name `agent.mode`"
                            .to_string(),
                    });
                }
                match merge_agent(file.defaults.agent.as_ref(), override_.as_ref()) {
                    Some(spec) => NodeKind::Agent {
                        agent: spec,
                        prompt: resolve_prompt(prompt),
                        output,
                        on_fail: resolve_agent_fail(on_fail),
                    },
                    None => {
                        issues.push(ValidationIssue {
                            node: Some(node.id.clone()),
                            kind: ValidationKind::MissingAgent,
                            message:
                                "an agent node needs `defaults.agent.id` or its own `agent.id`"
                                    .to_string(),
                        });
                        continue;
                    }
                }
            }
        };
        nodes.push(Node {
            id: node.id,
            deps: node.deps,
            kind,
            timeout,
        });
    }

    if issues.is_empty() {
        // `defaults.agent` first. Without it, fallback only when every
        // resolved agent node agrees on the exact same spec; otherwise a
        // repair prompt would run as whichever node happened to appear
        // first, which is not a stable policy.
        let default_agent = merge_agent(file.defaults.agent.as_ref(), None)
            .or_else(|| unanimous_agent_from_nodes(&nodes));

        Ok(Flow {
            version: file.version,
            default_agent,
            nodes,
        })
    } else {
        Err(issues)
    }
}

/// The resolved flow as a file that would produce it again: every node
/// states the settings it resolved to, so nothing is left to inherit. Lives
/// here because `resolve` owns the relationship between the two models — a
/// second module deriving it would be a second thing to keep in step.
///
/// `defaults` keeps exactly one thing: [`Flow::default_agent`], the agent a
/// repair's `fix` runs as. It is not a copy of `defaults.agent` — it also
/// falls back to the nodes when they agree — so a file that dropped it would
/// resolve to `None` for any flow whose nodes disagree.
///
/// File-backed prompts and hints are **inlined**, reading against
/// `flow_dir`. A path would be wrong twice over: it resolves against the
/// flow file's directory, not the run directory this record lands in, and
/// the file it names can change afterwards. What the run handed the agent
/// is the thing worth recording, and text does not move.
///
/// A file that cannot be read keeps its path. The run is about to fail on
/// the same read a moment later, with a `FlowIoError` that names it — so
/// the record says what it knows rather than inventing content.
pub fn to_flow_file(flow: &Flow, flow_dir: &Path) -> FlowFile {
    FlowFile {
        version: flow.version,
        defaults: Defaults {
            timeout: None,
            agent: flow.default_agent.as_ref().map(agent_override),
        },
        nodes: flow
            .nodes
            .iter()
            .map(|node| NodeFile {
                id: node.id.clone(),
                deps: node.deps.clone(),
                timeout: Some(node.timeout),
                kind: node_kind_file(&node.kind, flow_dir),
            })
            .collect(),
    }
}

/// The prompt's text, or the path back when it cannot be read.
fn inline(prompt: &Prompt, flow_dir: &Path) -> Result<String, PathBuf> {
    match prompt {
        Prompt::Inline(text) => Ok(text.clone()),
        Prompt::File(path) => {
            std::fs::read_to_string(flow_dir.join(path)).map_err(|_| path.clone())
        }
    }
}

fn node_kind_file(kind: &NodeKind, flow_dir: &Path) -> NodeKindFile {
    match kind {
        NodeKind::Agent {
            agent,
            prompt,
            output,
            on_fail,
        } => NodeKindFile::Agent {
            agent: Some(node_agent_override(agent)),
            prompt: match inline(prompt, flow_dir) {
                Ok(text) => PromptSource::Prompt(text),
                Err(path) => PromptSource::PromptFile(path),
            },
            output: output.clone(),
            on_fail: match on_fail {
                AgentFail::Halt => AgentFailFile::Halt,
                AgentFail::Retry {
                    hint,
                    max_attempts,
                    wait,
                } => AgentFailFile::Retry {
                    hint: match inline(hint, flow_dir) {
                        Ok(text) => HintSource::Hint(text),
                        Err(path) => HintSource::HintFile(path),
                    },
                    max_attempts: *max_attempts,
                    wait: Some(*wait),
                },
            },
        },
        NodeKind::Command { run, on_fail } => NodeKindFile::Command {
            run: run.clone(),
            on_fail: match on_fail {
                GateFail::Halt => GateFailFile::Halt,
                GateFail::Repair {
                    fix,
                    rerun,
                    max_attempts,
                    wait,
                } => GateFailFile::Repair {
                    fix: fix.clone(),
                    rerun: rerun.clone(),
                    max_attempts: *max_attempts,
                    wait: Some(*wait),
                },
            },
        },
    }
}

/// A node states its own `agent.id` unless the grammar forbids it: naming an
/// id without a `mode` is a validation error, and a spec that resolved to no
/// mode can only have inherited its id from `defaults` — which is where the
/// reload picks it back up.
fn node_agent_override(spec: &AgentSpec) -> AgentOverride {
    AgentOverride {
        id: spec.mode.as_ref().map(|_| spec.id.clone()),
        ..agent_override(spec)
    }
}

fn agent_override(spec: &AgentSpec) -> AgentOverride {
    AgentOverride {
        id: Some(spec.id.clone()),
        model: spec.model.clone(),
        effort: spec.effort.clone(),
        mode: spec.mode.clone(),
        permission: Some(match spec.permission {
            PermissionPolicy::Deny => PermissionPolicyFile::Deny,
            PermissionPolicy::AllowOnce => PermissionPolicyFile::AllowOnce,
        }),
    }
}

fn unanimous_agent_from_nodes(nodes: &[Node]) -> Option<AgentSpec> {
    let mut agents = nodes.iter().filter_map(|n| match &n.kind {
        NodeKind::Agent { agent, .. } => Some(agent),
        NodeKind::Command { .. } => None,
    });
    let first = agents.next()?.clone();
    if agents.all(|agent| agent == &first) {
        Some(first)
    } else {
        None
    }
}

/// Field by field: the node wins per axis, `defaults` fills the rest.
/// `None` only when neither source names an agent id.
fn merge_agent(
    defaults: Option<&AgentOverride>,
    node: Option<&AgentOverride>,
) -> Option<AgentSpec> {
    let pick =
        |f: fn(&AgentOverride) -> Option<String>| node.and_then(f).or_else(|| defaults.and_then(f));
    Some(AgentSpec {
        id: pick(|a| a.id.clone())?,
        model: pick(|a| a.model.clone()),
        effort: pick(|a| a.effort.clone()),
        mode: pick(|a| a.mode.clone()),
        permission: node
            .and_then(|a| a.permission)
            .or_else(|| defaults.and_then(|a| a.permission))
            .map(resolve_permission)
            .unwrap_or(PermissionPolicy::Deny),
    })
}

fn resolve_permission(p: PermissionPolicyFile) -> PermissionPolicy {
    match p {
        PermissionPolicyFile::Deny => PermissionPolicy::Deny,
        PermissionPolicyFile::AllowOnce => PermissionPolicy::AllowOnce,
    }
}

fn resolve_prompt(p: PromptSource) -> Prompt {
    match p {
        PromptSource::Prompt(text) => Prompt::Inline(text),
        PromptSource::PromptFile(path) => Prompt::File(path),
    }
}

fn resolve_hint(h: HintSource) -> Prompt {
    match h {
        HintSource::Hint(text) => Prompt::Inline(text),
        HintSource::HintFile(path) => Prompt::File(path),
    }
}

fn clamp_attempts(n: u32) -> u32 {
    n.clamp(MAX_ATTEMPTS_RANGE.0, MAX_ATTEMPTS_RANGE.1)
}

fn clamp_wait(w: Option<Duration>) -> Duration {
    w.unwrap_or(DEFAULT_WAIT).clamp(WAIT_RANGE.0, WAIT_RANGE.1)
}

fn resolve_agent_fail(f: AgentFailFile) -> AgentFail {
    match f {
        AgentFailFile::Halt => AgentFail::Halt,
        AgentFailFile::Retry {
            hint,
            max_attempts,
            wait,
        } => AgentFail::Retry {
            hint: resolve_hint(hint),
            max_attempts: clamp_attempts(max_attempts),
            wait: clamp_wait(wait),
        },
    }
}

fn resolve_gate_fail(f: GateFailFile) -> GateFail {
    match f {
        GateFailFile::Halt => GateFail::Halt,
        GateFailFile::Repair {
            fix,
            rerun,
            max_attempts,
            wait,
        } => GateFail::Repair {
            fix,
            rerun,
            max_attempts: clamp_attempts(max_attempts),
            wait: clamp_wait(wait),
        },
    }
}

#[cfg(test)]
mod tests {
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
        let flow = resolve(parse_flow_file(FLOW).unwrap()).expect("valid flow resolves");

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
            parse_flow_file(
                "version: 1\nnodes:\n  - id: t\n    kind: command\n    run: \"true\"\n",
            )
            .unwrap(),
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
        )
        .expect_err("no defaults.agent and no node override");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].node.as_deref(), Some("design"));
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
        )
        .expect("valid flow");
        assert_eq!(flow.default_agent.map(|a| a.id), Some("codex".to_string()));
    }

    #[test]
    fn a_command_only_flow_has_no_default_agent() {
        let flow = resolve(
            parse_flow_file(
                "version: 1\nnodes:\n  - id: t\n    kind: command\n    run: \"true\"\n",
            )
            .unwrap(),
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
            let flow = resolve(parse_flow_file(text).unwrap()).expect("valid flow");
            let regenerated = yaml_serde::to_string(&to_flow_file(&flow, std::path::Path::new("")))
                .expect("a flow file serializes");
            let reloaded = resolve(parse_flow_file(&regenerated).expect("parses"))
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
        )
        .expect_err("the node names an id but no mode");
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::AgentIdWithoutMode))
        );
    }
}
