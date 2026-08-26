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
    AgentBody, AgentFail, AgentSpec, Flow, GateFail, Node, NodeKind, PermissionPolicy, Prompt,
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
                output_schema,
                continue_until,
                max_turns,
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
                    Some(spec) => NodeKind::Agent(Box::new(crate::model::AgentBody {
                        agent: {
                            check_ask_has_a_mode(&spec, Some(&node.id), &mut issues);
                            spec
                        },
                        prompt: resolve_prompt(prompt),
                        output,
                        // Unboxed on the way in: the wire type boxes these
                        // because almost no node declares them, and the model
                        // boxes the whole arm instead.
                        output_schema: output_schema.map(|schema| *schema),
                        continue_until: continue_until.map(|done| *done),
                        max_turns: max_turns.unwrap_or(crate::model::DEFAULT_MAX_TURNS),
                        on_fail: resolve_agent_fail(on_fail),
                    })),
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
        // Taken apart field by field rather than read through `body`: a field
        // added to `AgentBody` then fails to compile *here*, which is what
        // makes someone decide whether it survives the trip back out. One
        // dropped silently is not a cosmetic loss — `run.yaml` is read back to
        // decide whether a pin still names the same node, so the node would
        // stop matching itself and the pin would quietly go unhonoured.
        NodeKind::Agent(body) => {
            let AgentBody {
                agent,
                prompt,
                output,
                output_schema,
                continue_until,
                max_turns,
                on_fail,
            } = &**body;
            NodeKindFile::Agent {
                agent: Some(node_agent_override(agent)),
                prompt: match inline(prompt, flow_dir) {
                    Ok(text) => PromptSource::Prompt(text),
                    Err(path) => PromptSource::PromptFile(path),
                },
                output: output.clone(),
                output_schema: output_schema.clone().map(Box::new),
                continue_until: continue_until.clone().map(Box::new),
                // Written back only when it is not the default, so a flow that
                // never mentioned turns does not grow a line on a round trip.
                max_turns: (*max_turns != crate::model::DEFAULT_MAX_TURNS).then_some(*max_turns),
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
            }
        }
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
        NodeKind::Agent(body) => Some(&body.agent),
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
mod profile_tests;
#[cfg(test)]
mod reserved_profile_tests;
#[cfg(test)]
mod tests;
