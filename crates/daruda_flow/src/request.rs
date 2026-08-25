//! What the host hands the engine to start a run, and the checks that need
//! that context. The engine never reads the agent catalog or the config
//! itself — it is given a finished map and a directory.

use crate::NodeId;
use crate::error::{ValidationIssue, ValidationKind};
use crate::model::{AgentFail, NodeKind, PermissionPolicy, Prompt};
use daruda_acp::LaunchSpec;
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

/// Everything one run needs. The host resolves lanes, agents and paths
/// before this point; the engine only ever sees finished values.
pub struct RunRequest {
    /// The flow and its graph, which only mean anything as a pair.
    pub loaded: crate::load::LoadedFlow,
    /// The working directory every node shares.
    pub cwd: PathBuf,
    /// `<cwd>/.daruda/flow-runs/<run-id>/`.
    pub run_dir: PathBuf,
    /// Where the flow file lives — `prompt_file` and `hint_file` resolve
    /// against this, not against `cwd`.
    pub flow_dir: PathBuf,
    /// Catalog id to launch spec, already resolved for this run's host and
    /// account.
    pub agents: HashMap<String, LaunchSpec>,
    pub node_install_dir: PathBuf,
    /// The three runaway defences, already resolved from config.
    pub budget: Budget,
    /// Whether a pid is still running. The engine never asks the OS; the
    /// host already tracks processes and answers. Owned rather than
    /// `lock::IsAlive`, which is a borrow and would put a lifetime on every
    /// type that carries a request.
    pub is_alive: Box<dyn Fn(u32) -> bool + Send>,
    /// `git status --porcelain` for `cwd`, asked once per attempt. Owned for
    /// the same reason as `is_alive`, and `None` for a host that has nothing
    /// to say — the record then carries no note, and the run is unaffected.
    pub git_status: Option<AskGitStatus>,
    /// Where the run narrates itself, for a host that would otherwise poll
    /// the run directory. Unbounded and never awaited, so a subscriber that
    /// stops reading cannot slow or stop the run; `None` is a host that does
    /// not watch, which is most of them.
    pub events: Option<smol::channel::Sender<crate::event::FlowEvent>>,
    /// Where a `permission: ask` node puts its question. Only a host that
    /// built somewhere to answer wires this, so **its presence is the
    /// capability declaration** — which is why `validate_request` reads it
    /// rather than `events`.
    ///
    /// The distinction is not academic: `examples/run_flow.rs` passes an
    /// `events` channel and only prints what arrives, so keying on that
    /// would let an `ask` flow start there and park forever.
    pub ask: Option<smol::channel::Sender<crate::runner::PendingAsk>>,
    /// Run no further than this node — it and its ancestors, nothing
    /// downstream. `None` runs the whole flow.
    ///
    /// A run-scoped axis rather than a flow-file one: the file describes what
    /// should happen, and a selection says which part of that to spend money
    /// on this time. Recorded in the journal, not in `run.yaml`.
    pub until: Option<NodeId>,
    /// Outputs to reuse instead of recomputing. Each names a node and where
    /// its finished output lives now — a previous run's directory.
    ///
    /// The engine copies each into this run's directory before the first node,
    /// so every run directory stays self-contained: `archive` and
    /// `{{node.<id>.output}}` both assume the output is inside it.
    pub pinned: Vec<PinnedOutput>,
    /// What an earlier process had already finished, when this is a run
    /// being picked up rather than started.
    ///
    /// Comes from [`crate::resume::prepare`], which produces it together
    /// with the `loaded` above — they are two halves of one run directory
    /// and pairing a replay with a different flow would skip nodes by name
    /// in a graph that never had them.
    pub resume: Option<crate::journal::Replay>,
}

impl RunRequest {
    /// The selection this run actually runs under.
    ///
    /// Two sources, because a continuation is handed no selection of its own:
    /// `build_resume_request` sends `until: None` and the axis comes back off
    /// the journal instead. Every question about which nodes this run will
    /// reach — what to validate, what to provision, what to walk — has to ask
    /// this rather than the field, or a resumed run is refused over a node it
    /// was told to skip.
    pub(crate) fn effective_until(&self) -> Option<&NodeId> {
        self.until
            .as_ref()
            .or_else(|| self.resume.as_ref().and_then(|r| r.until.as_ref()))
    }

    /// Every agent this run could open a session as, repair included when a
    /// selected gate can call for one. See [`crate::graph::agents_in_play`].
    pub(crate) fn selected_agents(&self) -> Vec<&str> {
        crate::graph::agents_in_play(self.loaded.flow(), self.effective_until())
    }

    /// The nodes this run will reach.
    ///
    /// **Every per-node question at submission asks this, never
    /// `loaded.flow().nodes` directly.** The two read the same in the common
    /// case and differ exactly when a selection is set, so a site that reaches
    /// past it looks correct and refuses a run over a node it stops before.
    /// That has now happened twice on this axis, which is why the reachable
    /// set has a name here rather than a filter each caller remembers.
    pub(crate) fn selected_nodes(&self) -> impl Iterator<Item = &crate::model::Node> {
        let selected = crate::graph::Selection::of(self.loaded.flow(), self.effective_until());
        self.loaded
            .flow()
            .nodes
            .iter()
            .filter(move |node| selected.includes(&node.id))
    }
}

/// One reused output: which node owes it, and where the copy comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedOutput {
    pub node: NodeId,
    pub from: PathBuf,
}

/// A cost ceiling is only meaningful in a currency, and two currencies do
/// not add up — so the currency travels with the amount.
#[derive(Debug, Clone, PartialEq)]
pub struct CostLimit {
    pub amount: f64,
    pub currency: String,
}

/// The three runaway defences. `None` is unlimited, which the host may
/// only choose explicitly (the config default has all three on).
#[derive(Debug, Clone, Default)]
pub struct Budget {
    /// When the run must stop. The host computes it from configured
    /// duration and start time; the engine only compares.
    pub deadline: Option<std::time::Instant>,
    /// Maximum budget units: every runner call consumes one, including
    /// reruns and fix sessions; an in-session correction consumes another.
    pub max_node_runs: Option<u32>,
    /// Only enforceable while the agent reports a cost, which is why the
    /// run's report carries a warning when it never does.
    pub max_cost: Option<CostLimit>,
}

impl Budget {
    /// Every defence off. Spelled out at each call site so a test that
    /// wants one limit does not silently get the other two.
    pub fn unlimited() -> Self {
        Self::default()
    }
}

/// The two static rules that need context the flow file does not carry.
/// Collected, not short-circuited.
/// The directory a node runs in has to be there. Checked here rather than
/// in `crate::validate` because only a request knows what the node's
/// relative `cwd` is relative *to*.
///
/// Not created: a flow naming a directory that does not exist is a flow
/// describing a tree it is not looking at, and making one would hide that.
fn check_node_cwd(
    request: &RunRequest,
    node: &crate::model::Node,
    issues: &mut Vec<ValidationIssue>,
) {
    let Some(relative) = node.cwd.as_ref() else {
        return;
    };
    let resolved = request.cwd.join(relative);
    if !resolved.is_dir() {
        issues.push(ValidationIssue {
            node: Some(node.id.clone()),
            kind: ValidationKind::CwdMissing {
                path: resolved.display().to_string(),
            },
            message: format!("`{}` is not a directory", resolved.display()),
        });
    }
}

/// How a host reports a working tree's state, asked about the directory an
/// attempt ran in. Owned and `Send` because it crosses into the run's
/// thread; named because the boxed signature is otherwise repeated at every
/// site that builds one.
pub type AskGitStatus = Box<dyn Fn(&std::path::Path) -> Option<String> + Send>;

pub fn validate_request(request: &RunRequest) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    check_absolute(request, &mut issues);
    if let Some(target) = &request.until
        && !request.loaded.flow().nodes.iter().any(|n| &n.id == target)
    {
        issues.push(ValidationIssue {
            node: None,
            kind: ValidationKind::UnknownUntil {
                node: target.clone(),
            },
            message: format!("`{target}` is not a node in this flow"),
        });
    }
    let selected = crate::graph::Selection::of(request.loaded.flow(), request.effective_until());
    for pin in &request.pinned {
        if !request.loaded.flow().nodes.iter().any(|n| n.id == pin.node) {
            // Asked of every pin, selection or not: naming a node the flow
            // does not have is wrong about the flow, not about this run.
            issues.push(ValidationIssue {
                node: None,
                kind: ValidationKind::UnknownPin {
                    node: pin.node.clone(),
                },
                message: format!("`{}` is not a node in this flow", pin.node),
            });
        } else if selected.includes(&pin.node) && !pin.from.is_file() {
            // Where the source sits only matters for a pin this run will use.
            // A pin on a node it stops before is not copied, so refusing the
            // run over that file would be refusing it for a node it does not
            // run.
            issues.push(ValidationIssue {
                node: Some(pin.node.clone()),
                kind: ValidationKind::PinnedSourceMissing {
                    node: pin.node.clone(),
                    path: pin.from.display().to_string(),
                },
                message: format!(
                    "the output pinned for `{}` is not at {}",
                    pin.node,
                    pin.from.display()
                ),
            });
        }
    }
    check_no_pin_blocks_a_repair(request, &mut issues);
    let mut checked_agents = HashSet::new();
    // Per-node checks follow the selection: refusing a run over a node it was
    // told not to run is exactly what `until` exists to get past — a flow with
    // a half-written tail is the normal state while iterating on its head.
    // The flow-level checks above stay unconditional.
    for node in request.selected_nodes() {
        // Before the node kinds split: a command node runs somewhere too,
        // and a missing directory fails it just as surely.
        check_node_cwd(request, node, &mut issues);

        let NodeKind::Agent {
            agent,
            prompt,
            on_fail,
            ..
        } = &node.kind
        else {
            continue;
        };

        check_agent_id(
            &agent.id,
            Some(&node.id),
            &request.agents,
            &mut checked_agents,
            &mut issues,
        );
        // Asked here and not in `crate::validate` on purpose: `new_node`
        // seeds an empty prompt so a fresh card can be typed into, so a blank
        // one is a legitimate file to author and only an illegitimate one to
        // run. Refusing at load would blank the canvas the author needs.
        if matches!(prompt, Prompt::Inline(text) if text.trim().is_empty()) {
            issues.push(ValidationIssue {
                node: Some(node.id.clone()),
                kind: ValidationKind::EmptyPrompt,
                message:
                    "an agent node with a blank prompt would open a session and ask it nothing"
                        .to_string(),
            });
        }
        // A file-backed prompt hides its `{{node.x.output}}` references from
        // `crate::validate`, which never opens the file; this is the only
        // stage that knows `flow_dir`, so the ancestor rule is applied here.
        let ancestors: HashSet<NodeId> = request
            .loaded
            .graph()
            .ancestors(&node.id)
            .into_iter()
            .collect();
        check_file(
            &request.flow_dir,
            prompt,
            "prompt_file",
            &node.id,
            &ancestors,
            &mut issues,
        );
        if let AgentFail::Retry { hint, .. } = on_fail {
            check_file(
                &request.flow_dir,
                hint,
                "hint_file",
                &node.id,
                &ancestors,
                &mut issues,
            );
        }
    }

    // The repair agent, and only when a gate this run reaches can call for
    // one: a flow whose repairs are all downstream of where this run stops
    // never opens a `fix` session, so an agent it could not run is not this
    // run's problem.
    if let Some(agent) = repair_agent_in_play(request) {
        check_agent_id(
            &agent.id,
            None,
            &request.agents,
            &mut checked_agents,
            &mut issues,
        );
    }

    check_someone_can_answer(request, &mut issues);
    issues
}

/// Refuse a run that could put a question to a person this host cannot
/// reach. Without it the run parks and only a Stop ends it.
///
/// The default agent counts even when no node names `ask`: a repair's
/// `fix` session runs as `flow.default_agent` and inherits its policy, so
/// a flow of nothing but `deny` nodes can still ask.
/// Refuse a run whose pins would make a repair futile.
///
/// A gate's `rerun` names nodes to run again so the gate's verdict can
/// change; a pin says one of those must not run at all. Honouring the pin —
/// which is what the scheduler does, since a pinned node never enters
/// `executed` — leaves the repair paying for a fix session per attempt and
/// re-testing an input nothing touched. Silently dropping the pin instead
/// would spend money the user said not to spend. Neither is ours to pick, so
/// the run is refused while it still costs nothing and the author unpins or
/// drops the rerun.
fn check_no_pin_blocks_a_repair(request: &RunRequest, issues: &mut Vec<ValidationIssue>) {
    if request.pinned.is_empty() {
        return;
    }
    let graph = request.loaded.graph();
    for gate in request.selected_nodes() {
        let NodeKind::Command {
            on_fail: crate::model::GateFail::Repair { rerun, .. },
            ..
        } = &gate.kind
        else {
            continue;
        };
        let closure = graph.rerun_closure(rerun);
        for pin in &request.pinned {
            if closure.contains(&pin.node) {
                issues.push(ValidationIssue {
                    node: Some(pin.node.clone()),
                    kind: ValidationKind::PinnedNodeInRerun {
                        node: pin.node.clone(),
                        gate: gate.id.clone(),
                    },
                    message: format!(
                        "`{}` is pinned, but `{}`'s repair re-runs it to change its own verdict",
                        pin.node, gate.id
                    ),
                });
            }
        }
    }
}

fn check_someone_can_answer(request: &RunRequest, issues: &mut Vec<ValidationIssue>) {
    if request.ask.is_some() {
        return;
    }
    let node_asks = request.selected_nodes().find_map(|node| match &node.kind {
        NodeKind::Agent { agent, .. } if agent.permission == PermissionPolicy::Ask => {
            Some(node.id.clone())
        }
        _ => None,
    });
    let repair_asks = repair_agent_in_play(request)
        .is_some_and(|agent| agent.permission == PermissionPolicy::Ask);
    if node_asks.is_none() && !repair_asks {
        return;
    }
    issues.push(ValidationIssue {
        node: node_asks,
        kind: ValidationKind::NobodyToAsk,
        message: "this run asks a person for permission but was given no way to ask".to_string(),
    });
}

/// The agent a repair's `fix` would run as, when this run can reach a repair
/// at all. `None` when the flow names none, or when every gate that could
/// call for one is past where this run stops.
fn repair_agent_in_play(request: &RunRequest) -> Option<&crate::model::AgentSpec> {
    let flow = request.loaded.flow();
    let selected = crate::graph::Selection::of(flow, request.effective_until());
    selected
        .reaches_a_repair(flow)
        .then_some(flow.default_agent.as_ref())
        .flatten()
}

fn check_agent_id(
    id: &str,
    node: Option<&NodeId>,
    agents: &HashMap<String, LaunchSpec>,
    checked: &mut HashSet<String>,
    issues: &mut Vec<ValidationIssue>,
) {
    if !checked.insert(id.to_string()) || agents.contains_key(id) {
        return;
    }
    issues.push(ValidationIssue {
        node: node.cloned(),
        kind: ValidationKind::UnknownAgent { id: id.to_string() },
        message: format!("`{id}` is not in this run's agent catalog"),
    });
}

fn check_file(
    flow_dir: &Path,
    prompt: &Prompt,
    field: &'static str,
    node: &NodeId,
    ancestors: &HashSet<NodeId>,
    issues: &mut Vec<ValidationIssue>,
) {
    let Prompt::File(relative) = prompt else {
        return;
    };
    if relative.is_absolute()
        || relative.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        issues.push(ValidationIssue {
            node: Some(node.clone()),
            kind: ValidationKind::PromptFileOutsideFlowDir {
                field,
                path: relative.clone(),
            },
            message: format!(
                "`{field}` must stay under the flow directory: `{}`",
                relative.display()
            ),
        });
        return;
    }
    let path = flow_dir.join(relative);
    if !path.is_file() {
        issues.push(ValidationIssue {
            node: Some(node.clone()),
            kind: ValidationKind::MissingPromptFile {
                field,
                path: path.clone(),
            },
            message: format!(
                "`{field}` points at `{}`, which does not exist",
                path.display()
            ),
        });
        return;
    }

    let root = flow_dir
        .canonicalize()
        .unwrap_or_else(|_| flow_dir.to_path_buf());
    let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
    if !canonical.starts_with(&root) {
        issues.push(ValidationIssue {
            node: Some(node.clone()),
            kind: ValidationKind::PromptFileOutsideFlowDir {
                field,
                path: path.clone(),
            },
            message: format!(
                "`{field}` resolves outside `{}`: `{}`",
                root.display(),
                canonical.display()
            ),
        });
        return;
    }

    match std::fs::read_to_string(&path) {
        Ok(text) => crate::validate::check_output_refs(&text, node, ancestors, issues),
        // Unreadable is the same fact as missing to a caller — a distinct
        // kind would only add a branch nothing renders differently.
        Err(e) => issues.push(ValidationIssue {
            node: Some(node.clone()),
            kind: ValidationKind::MissingPromptFile {
                field,
                path: path.clone(),
            },
            message: format!("`{field}` at `{}` cannot be read: {e}", path.display()),
        }),
    }
}

/// Every path here was resolved by the host. A relative one is resolved a
/// second time — against whatever directory the process happens to be in —
/// so the run would take its lock, write its record and build its outputs
/// somewhere that is neither the lane nor the run, and only notice when an
/// adapter refused the `cwd` much later.
fn check_absolute(request: &RunRequest, issues: &mut Vec<ValidationIssue>) {
    for (field, path) in [
        ("cwd", &request.cwd),
        ("run_dir", &request.run_dir),
        ("flow_dir", &request.flow_dir),
        ("node_install_dir", &request.node_install_dir),
    ] {
        if !path.is_absolute() {
            issues.push(ValidationIssue {
                node: None,
                kind: ValidationKind::RelativeRequestPath { field },
                message: format!("`{field}` must be absolute: `{}`", path.display()),
            });
        }
    }
}

#[cfg(test)]
mod tests;
