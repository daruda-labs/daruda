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
/// One is serial. The ceiling is not a machine limit — every node here is
/// an agent session or a subprocess, and a flow asking for dozens at once
/// has almost certainly mistaken this for a thread pool.
pub const PARALLEL_RANGE: (usize, usize) = (1, 8);
/// A retry that waits a full minute is almost certainly a typo; the ceiling
/// keeps one from parking the run.
pub const WAIT_RANGE: (Duration, Duration) = (Duration::ZERO, Duration::from_secs(60));

/// Merge `defaults` — under the chosen profile, if any — into every node
/// and produce the executable `Flow`.
pub fn resolve(file: FlowFile, profile: Option<&str>) -> Result<Flow, Vec<ValidationIssue>> {
    let mut issues = Vec::new();
    let mut nodes = Vec::with_capacity(file.nodes.len());
    if file.profiles.contains_key(RESERVED_PROFILE_NAME) {
        issues.push(ValidationIssue {
            node: None,
            kind: ValidationKind::ReservedProfileName,
            message: format!(
                "`{RESERVED_PROFILE_NAME}` is the base layer every profile is folded over, \
                 so a profile cannot also be called that"
            ),
        });
    }
    let defaults = match effective_defaults(&file, profile, &mut issues) {
        Some(defaults) => defaults,
        // Nothing below can mean anything against settings that were not
        // resolved, and every issue it found would name the wrong ones.
        None => return Err(issues),
    };

    for node in file.nodes {
        let timeout = node.timeout.or(defaults.timeout).unwrap_or(DEFAULT_TIMEOUT);
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
                match merge_agent(defaults.agent.as_ref(), override_.as_ref()) {
                    Some(spec) => NodeKind::Agent {
                        agent: {
                            check_ask_has_a_mode(&spec, Some(&node.id), &mut issues);
                            spec
                        },
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
            cwd: node.cwd,
        });
    }

    // `defaults.agent` first. Without it, fallback only when every resolved
    // agent node agrees on the exact same spec; otherwise a repair prompt
    // would run as whichever node happened to appear first, which is not a
    // stable policy.
    let default_agent =
        merge_agent(defaults.agent.as_ref(), None).or_else(|| unanimous_agent_from_nodes(&nodes));
    // A repair's `fix` runs as this one and inherits its policy, so a flow
    // whose nodes never ask can still reach `Ask` through it. Checked
    // before the verdict below, not after — an issue pushed past it would
    // be collected and then thrown away with an `Ok`.
    if let Some(agent) = default_agent.as_ref() {
        check_ask_has_a_mode(agent, None, &mut issues);
    }

    if issues.is_empty() {
        Ok(Flow {
            version: file.version,
            // Clamped rather than refused: a number outside the range is a
            // wish about speed, not a mistake about meaning, and the same
            // reasoning `max_attempts` is clamped under.
            parallel: defaults
                .parallel
                .unwrap_or(1)
                .clamp(PARALLEL_RANGE.0, PARALLEL_RANGE.1),
            profile: profile.map(str::to_string),
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
/// [`Flow::profile`] is deliberately **not** round-tripped: it is
/// provenance, not a setting, and every node below already states what the
/// profile resolved to. Which named layer produced them is recorded in
/// `run.md` instead — so a profiled `Flow` reloaded from `run.yaml` differs
/// from the original in that one field, and only that one.
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
            parallel: Some(flow.parallel),
        },
        // Every node below states what it resolved to, so the profile that
        // produced those settings has already been applied. Carrying the
        // menu into the record would invite reading it as a choice the run
        // still has.
        profiles: std::collections::BTreeMap::new(),
        nodes: flow
            .nodes
            .iter()
            .map(|node| NodeFile {
                id: node.id.clone(),
                deps: node.deps.clone(),
                timeout: Some(node.timeout),
                // Kept relative, like the model: the record is read on
                // whichever machine opens it, and a resolved path would be
                // the one this run happened to have.
                cwd: node.cwd.clone(),
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
            PermissionPolicy::Ask => PermissionPolicyFile::Ask,
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

/// The one name a profile may not take: the file already uses it for the
/// base layer every profile is folded over.
const RESERVED_PROFILE_NAME: &str = "defaults";

/// `defaults`, with the named profile folded over it. `None` means the
/// name is not one of the file's — reported, never silently ignored.
///
/// One layer, not two rankings: the order is `defaults` ← `profile` ←
/// `node`, so a node that pins an axis still wins. A profile that beat the
/// nodes would make the file unreadable on its own — what a node does
/// would depend on which profile someone picked.
fn effective_defaults(
    file: &FlowFile,
    profile: Option<&str>,
    issues: &mut Vec<ValidationIssue>,
) -> Option<Defaults> {
    let Some(name) = profile else {
        return Some(file.defaults.clone());
    };
    let Some(over) = file.profiles.get(name) else {
        issues.push(ValidationIssue {
            node: None,
            kind: ValidationKind::UnknownProfile {
                name: name.to_string(),
            },
            message: format!("this flow declares no profile named `{name}`"),
        });
        return None;
    };
    // The same rule a node follows, for the same reason: a mode carried
    // over from `defaults` belongs to the agent `defaults` named, so a
    // layer that switches the agent has to say the mode again.
    if over.agent.as_ref().and_then(|a| a.id.as_ref()).is_some()
        && over.agent.as_ref().and_then(|a| a.mode.as_ref()).is_none()
    {
        issues.push(ValidationIssue {
            node: None,
            kind: ValidationKind::AgentIdWithoutMode,
            message: format!(
                "profile `{name}` names its own `agent.id`, so it must also name \
                              `agent.mode`"
            ),
        });
    }
    Some(Defaults {
        timeout: over.timeout.or(file.defaults.timeout),
        agent: layer(file.defaults.agent.as_ref(), over.agent.as_ref()),
        parallel: over.parallel.or(file.defaults.parallel),
    })
}

/// Field by field, the later layer winning per axis. The one merge rule
/// there is — `defaults` ← `profile` and `defaults` ← `node` are the same
/// operation, and two copies of it would be two things to keep in step.
fn layer(base: Option<&AgentOverride>, over: Option<&AgentOverride>) -> Option<AgentOverride> {
    if base.is_none() {
        return over.cloned();
    }
    if over.is_none() {
        return base.cloned();
    }
    let pick =
        |f: fn(&AgentOverride) -> Option<String>| over.and_then(f).or_else(|| base.and_then(f));
    Some(AgentOverride {
        id: pick(|a| a.id.clone()),
        model: pick(|a| a.model.clone()),
        effort: pick(|a| a.effort.clone()),
        mode: pick(|a| a.mode.clone()),
        permission: over
            .and_then(|a| a.permission)
            .or_else(|| base.and_then(|a| a.permission)),
    })
}

/// The node's own layer over the resolved defaults, as the executable
/// spec. `None` only when neither source names an agent id.
fn merge_agent(
    defaults: Option<&AgentOverride>,
    node: Option<&AgentOverride>,
) -> Option<AgentSpec> {
    let merged = layer(defaults, node)?;
    Some(AgentSpec {
        id: merged.id?,
        model: merged.model,
        effort: merged.effort,
        mode: merged.mode,
        permission: merged
            .permission
            .map(resolve_permission)
            .unwrap_or(PermissionPolicy::Deny),
    })
}

/// A flow that wants a person asked has to say which mode it runs in.
///
/// Not a fallback to daruda's own configured default: a flow file is
/// committed and shared, and filling it in from local config would make
/// the same file ask on one machine and not another — with `run.yaml`
/// recording a mode the file never said. What the file says is the whole
/// of it, so this makes it say.
///
/// Names no mode itself. Which modes prompt is the adapter's to advertise
/// (§8), and this only observes that the deciding axis was left unset.
fn check_ask_has_a_mode(
    spec: &AgentSpec,
    node: Option<&crate::NodeId>,
    issues: &mut Vec<ValidationIssue>,
) {
    if spec.permission != PermissionPolicy::Ask || spec.mode.is_some() {
        return;
    }
    issues.push(ValidationIssue {
        node: node.cloned(),
        kind: ValidationKind::AskWithoutMode,
        message: "`permission: ask` needs an `agent.mode`; whether anyone is asked depends on it"
            .to_string(),
    });
}

fn resolve_permission(p: PermissionPolicyFile) -> PermissionPolicy {
    match p {
        PermissionPolicyFile::Deny => PermissionPolicy::Deny,
        PermissionPolicyFile::AllowOnce => PermissionPolicy::AllowOnce,
        PermissionPolicyFile::Ask => PermissionPolicy::Ask,
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
            parse_flow_file(
                "version: 1\nnodes:\n  - id: t\n    kind: command\n    run: \"true\"\n",
            )
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
            parse_flow_file(
                "version: 1\nnodes:\n  - id: t\n    kind: command\n    run: \"true\"\n",
            )
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
}

#[cfg(test)]
mod profile_tests {
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
}

#[cfg(test)]
mod reserved_profile_tests {
    use super::*;
    use crate::parse::parse_flow_file;

    /// The file spells the base layer `defaults:`, so a profile of that
    /// name means two things at once — and a host listing the base beside
    /// the declared ones would show two rows nobody can tell apart, with
    /// different settings behind them.
    ///
    /// Refused whether or not it is the one chosen: the ambiguity is in
    /// the file, not in the pick.
    #[test]
    fn a_profile_cannot_take_the_name_of_the_layer_it_covers() {
        let text = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
profiles:
  defaults:
    timeout: 1m
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
";
        for chosen in [None, Some("defaults")] {
            let issues = resolve(parse_flow_file(text).expect("parses"), chosen)
                .expect_err("a reserved profile name is not a flow");
            assert!(
                issues
                    .iter()
                    .any(|i| matches!(i.kind, ValidationKind::ReservedProfileName)),
                "chosen {chosen:?}: {issues:?}"
            );
        }
    }
}
