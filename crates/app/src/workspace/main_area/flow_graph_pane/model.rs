//! A flow, in the shape the canvas draws it. Pure — no GPUI, no vendor
//! types — so the mapping is testable on its own.
//!
//! The model carries *data*, not wording. Formatting an axis into
//! `claude · effort high · 10m` belongs to the renderer, which is also the
//! only layer allowed to reach `surface::strings`.

use std::path::PathBuf;
use std::time::Duration;

use daruda_flow::NodeId;
use daruda_flow::event::FlowEvent;
use daruda_flow::model::{AgentFail, Flow, GateFail, NodeKind, Prompt};

/// Nodes and the two kinds of line between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) struct FlowGraphModel {
    pub nodes: Vec<GraphNode>,
    /// `deps` — execution order. Drawn as the canvas's own edges.
    pub deps: Vec<GraphEdge>,
    /// A gate's `rerun` — nodes it sends back to be re-derived.
    ///
    /// **Not a dependency.** In the model this lives inside
    /// `GateFail::Repair`, so it is drawn as an overlay rather than as a
    /// graph edge; giving it an `Edge` would put it in the same visual
    /// vocabulary as `deps` and say the wrong thing.
    pub rerun: Vec<GraphEdge>,
    /// What the engine's graph-dependent rules refuse about this flow.
    ///
    /// Carried rather than turned into a refusal, because they are about a
    /// flow that resolved and whose graph built — so the cards can be drawn
    /// and the ones at fault can say so. The run still refuses it: the engine
    /// asks `load`, which is `inspect` plus that refusal.
    pub issues: Vec<daruda_flow::error::ValidationIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) struct GraphEdge {
    pub from: NodeId,
    pub to: NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) struct GraphNode {
    pub id: NodeId,
    pub kind: GraphNodeKind,
    pub timeout: Duration,
    /// Where this node runs, relative to the run directory. `None` is that
    /// directory itself — which is also what lets two nodes overlap.
    pub cwd: Option<PathBuf>,
    pub fail: FailPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) enum GraphNodeKind {
    Agent {
        agent: AgentAxes,
        prompt: PromptSummary,
        output: PathBuf,
    },
    /// A shell line whose exit code is the verdict.
    Gate { run: String },
}

/// The agent axes worth seeing without opening the node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) struct AgentAxes {
    pub id: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) enum PromptSummary {
    /// First line of an inline prompt — the card has one line to spend.
    Inline(String),
    File(PathBuf),
}

/// What the node does when it fails, in the terms the card shows.
///
/// A gate that repairs says so even when `rerun` is empty: an empty set
/// re-runs the gate alone, which is a real policy and not the absence of
/// one, and no edge would otherwise reveal it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) enum FailPolicy {
    Halt,
    Retry {
        max_attempts: u32,
    },
    Repair {
        max_attempts: u32,
        rerun_count: usize,
    },
}

/// Where a node is in the run driving this graph.
///
/// `Pending` is also the answer when no run drives it at all — a flow
/// opened from a file is every node pending, which is exactly how it
/// should read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::workspace) enum NodeRunState {
    #[default]
    Pending,
    Running {
        attempt: u32,
    },
    Passed,
    Failed,
    /// A gate running its repair's `fix` session.
    Fixing,
}

/// Per-node state a run has reported so far. Absent means [`NodeRunState::Pending`]
/// — a node the run has not spoken about yet is one that has not started.
pub(in crate::workspace) type NodeRunStates = std::collections::HashMap<NodeId, NodeRunState>;

/// What a run says about the flow a graph is drawing.
///
/// The states and the nodes they are *about* travel together and are only
/// meaningful together: an id freed by a delete and taken by a rename would
/// otherwise wear the first node's state. One value so a caller cannot pass
/// one without the other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct RunColouring {
    pub states: NodeRunStates,
    /// The flow's node ids as they were when the run was submitted.
    pub of_nodes: Vec<NodeId>,
}

impl RunColouring {
    /// Whether this colouring is about `nodes` — the same ids, in the same
    /// order. A graph that has changed shape since the run started is left
    /// uncoloured rather than coloured wrongly.
    pub(in crate::workspace) fn is_about(&self, nodes: impl Iterator<Item = NodeId>) -> bool {
        self.of_nodes.iter().cloned().eq(nodes)
    }
}

/// Fold one event into the per-node states.
///
/// Pure so the projection is testable without a run: every rule here is a
/// claim about what the engine's stream means, and getting one wrong leaves
/// a node showing the wrong colour for the rest of the run.
pub(in crate::workspace) fn apply_run_event(states: &mut NodeRunStates, event: &FlowEvent) {
    match event {
        // The node set comes from the file, which is already drawn. Seeding
        // from the run as well would make a file edited mid-run disagree with
        // itself, and the states default to `Pending` regardless.
        FlowEvent::RunStarted { .. } => {}
        FlowEvent::NodeStarted { node, attempt } => {
            states.insert(node.clone(), NodeRunState::Running { attempt: *attempt });
        }
        FlowEvent::NodePassed { node, .. } => {
            states.insert(node.clone(), NodeRunState::Passed);
        }
        FlowEvent::NodeFailed { node, .. } => {
            states.insert(node.clone(), NodeRunState::Failed);
        }
        FlowEvent::FixStarted { gate } => {
            states.insert(gate.clone(), NodeRunState::Fixing);
        }
        // The fix is over but the gate has not re-run, so it is still the
        // failure it was — not a pass, and no longer fixing.
        FlowEvent::FixEnded { gate, .. } => {
            states.insert(gate.clone(), NodeRunState::Failed);
        }
        // The one transition a host cannot infer (`event.rs` says so): these
        // already passed, and are about to be derived again. Left alone they
        // would keep the colour of a result that no longer holds.
        FlowEvent::Rerunning { members, .. } => {
            for node in members {
                states.insert(node.clone(), NodeRunState::Pending);
            }
        }
        // Whatever the last word on each node was, it stands.
        FlowEvent::RunEnded { .. } => {}
    }
}

impl FlowGraphModel {
    pub(in crate::workspace) fn from_flow(
        flow: &Flow,
        issues: Vec<daruda_flow::error::ValidationIssue>,
    ) -> Self {
        let mut deps = Vec::new();
        let mut rerun = Vec::new();
        let nodes = flow
            .nodes
            .iter()
            .map(|node| {
                for dep in &node.deps {
                    deps.push(GraphEdge {
                        from: dep.clone(),
                        to: node.id.clone(),
                    });
                }
                if let NodeKind::Command {
                    on_fail: GateFail::Repair { rerun: members, .. },
                    ..
                } = &node.kind
                {
                    for member in members {
                        rerun.push(GraphEdge {
                            from: node.id.clone(),
                            to: member.clone(),
                        });
                    }
                }
                GraphNode {
                    id: node.id.clone(),
                    kind: kind_of(&node.kind),
                    timeout: node.timeout,
                    cwd: node.cwd.clone(),
                    fail: fail_of(&node.kind),
                }
            })
            .collect();
        Self {
            nodes,
            deps,
            rerun,
            issues,
        }
    }

    /// How many refusals name `node`. Flow-level ones — a cycle, an unknown
    /// version — name none, and stay in the banner where the whole sentence
    /// is readable.
    pub(in crate::workspace) fn issues_naming(&self, node: &NodeId) -> usize {
        self.issues
            .iter()
            .filter(|issue| issue.node.as_ref() == Some(node))
            .count()
    }
}

fn kind_of(kind: &NodeKind) -> GraphNodeKind {
    match kind {
        NodeKind::Agent {
            agent,
            prompt,
            output,
            ..
        } => GraphNodeKind::Agent {
            agent: AgentAxes {
                id: agent.id.clone(),
                model: agent.model.clone(),
                effort: agent.effort.clone(),
                mode: agent.mode.clone(),
            },
            prompt: match prompt {
                Prompt::Inline(text) => PromptSummary::Inline(first_line(text)),
                Prompt::File(path) => PromptSummary::File(path.clone()),
            },
            output: output.clone(),
        },
        NodeKind::Command { run, .. } => GraphNodeKind::Gate { run: run.clone() },
    }
}

fn fail_of(kind: &NodeKind) -> FailPolicy {
    match kind {
        NodeKind::Agent {
            on_fail: AgentFail::Halt,
            ..
        }
        | NodeKind::Command {
            on_fail: GateFail::Halt,
            ..
        } => FailPolicy::Halt,
        NodeKind::Agent {
            on_fail: AgentFail::Retry { max_attempts, .. },
            ..
        } => FailPolicy::Retry {
            max_attempts: *max_attempts,
        },
        NodeKind::Command {
            on_fail:
                GateFail::Repair {
                    max_attempts,
                    rerun,
                    ..
                },
            ..
        } => FailPolicy::Repair {
            max_attempts: *max_attempts,
            rerun_count: rerun.len(),
        },
    }
}

/// The card has one line for a prompt, and a prompt is prose that usually
/// opens with the instruction worth seeing.
fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every fixture shares one `defaults` block; the test supplies only
    /// the nodes. A backslash line-continuation would eat the leading
    /// indentation and hand YAML a different document, so the node text is
    /// always a plain literal.
    const AGENT_DEFAULTS: &str = concat!(
        "version: 1\n",
        "defaults:\n",
        "  agent:\n",
        "    id: claude\n",
        "    mode: bypassPermissions\n",
        "nodes:\n",
    );

    fn model(nodes: &str) -> FlowGraphModel {
        let yaml = format!("{AGENT_DEFAULTS}{nodes}");
        let loaded = daruda_flow::load(&yaml, None)
            .unwrap_or_else(|e| panic!("fixture should load: {e}\n{yaml}"));
        FlowGraphModel::from_flow(loaded.flow(), Vec::new())
    }

    fn edge(from: &str, to: &str) -> GraphEdge {
        GraphEdge {
            from: from.into(),
            to: to.into(),
        }
    }

    #[test]
    fn a_chain_becomes_one_edge_per_link() {
        let m = model(
            "  - id: a
    kind: agent
    output: a.md
    prompt: first
  - id: b
    kind: agent
    deps: [a]
    output: b.md
    prompt: second
  - id: c
    kind: agent
    deps: [b]
    output: c.md
    prompt: third
",
        );

        assert_eq!(m.nodes.len(), 3);
        assert_eq!(m.deps, vec![edge("a", "b"), edge("b", "c")]);
        assert!(m.rerun.is_empty());
    }

    #[test]
    fn two_nodes_on_one_parent_both_get_an_edge() {
        let m = model(
            "  - id: root
    kind: agent
    output: root.md
    prompt: go
  - id: left
    kind: agent
    deps: [root]
    output: left.md
    prompt: go
  - id: right
    kind: agent
    deps: [root]
    output: right.md
    prompt: go
",
        );

        let from_root: Vec<_> = m.deps.iter().filter(|e| e.from == "root").collect();
        assert_eq!(from_root.len(), 2, "a fan-out draws both branches");
    }

    /// The design's two axes meet here: a gate that repairs sends named
    /// nodes back, and that line is not a dependency.
    #[test]
    fn a_gate_s_rerun_becomes_a_back_edge_not_a_dep() {
        let m = model(
            "  - id: review
    kind: agent
    output: review.md
    prompt: review it
  - id: gate
    kind: command
    deps: [review]
    run: \"true\"
    on_fail:
      repair:
        fix: \"{{failure}} {{attempts}} fix it\"
        rerun: [review]
        max_attempts: 2
",
        );

        assert_eq!(
            m.deps,
            vec![edge("review", "gate")],
            "the only dependency is review -> gate"
        );
        assert_eq!(
            m.rerun,
            vec![edge("gate", "review")],
            "and the back edge runs the other way, from the gate"
        );
    }

    /// An empty `rerun` re-runs the gate alone. There is no edge to draw,
    /// so the card is the only place that can say a repair policy exists.
    #[test]
    fn an_empty_rerun_draws_no_edge_but_still_reports_the_policy() {
        let m = model(
            "  - id: gate
    kind: command
    run: \"true\"
    on_fail:
      repair:
        fix: \"{{failure}} {{attempts}} fix it\"
        max_attempts: 3
",
        );

        assert!(m.rerun.is_empty());
        assert_eq!(
            m.nodes[0].fail,
            FailPolicy::Repair {
                max_attempts: 3,
                rerun_count: 0
            }
        );
    }

    #[test]
    fn an_agent_node_and_a_gate_carry_different_shapes() {
        let m = model(
            "  - id: work
    kind: agent
    agent: { effort: high }
    output: out.md
    timeout: 5m
    prompt: |
      first line worth showing
      second line that is not
  - id: check
    kind: command
    deps: [work]
    run: cargo test
",
        );

        let work = &m.nodes[0];
        assert_eq!(work.timeout, Duration::from_secs(300));
        assert_eq!(work.fail, FailPolicy::Halt);
        let GraphNodeKind::Agent {
            agent,
            prompt,
            output,
        } = &work.kind
        else {
            panic!("work is an agent node");
        };
        assert_eq!(agent.id, "claude");
        assert_eq!(agent.effort.as_deref(), Some("high"));
        assert_eq!(
            *prompt,
            PromptSummary::Inline("first line worth showing".into()),
            "the card has one line to spend"
        );
        assert_eq!(output, &PathBuf::from("out.md"));

        assert_eq!(
            m.nodes[1].kind,
            GraphNodeKind::Gate {
                run: "cargo test".into()
            }
        );
    }

    #[test]
    fn a_file_backed_prompt_keeps_its_path() {
        let m = model(
            "  - id: work
    kind: agent
    output: out.md
    prompt_file: ./prompts/work.md
",
        );

        let GraphNodeKind::Agent { prompt, .. } = &m.nodes[0].kind else {
            panic!("work is an agent node");
        };
        assert_eq!(
            *prompt,
            PromptSummary::File(PathBuf::from("./prompts/work.md"))
        );
    }

    fn fold(events: &[FlowEvent]) -> NodeRunStates {
        let mut states = NodeRunStates::new();
        for event in events {
            apply_run_event(&mut states, event);
        }
        states
    }

    fn started(node: &str, attempt: u32) -> FlowEvent {
        FlowEvent::NodeStarted {
            node: node.into(),
            attempt,
        }
    }

    fn passed(node: &str) -> FlowEvent {
        FlowEvent::NodePassed {
            node: node.into(),
            attempt: 1,
        }
    }

    fn failed(node: &str) -> FlowEvent {
        FlowEvent::NodeFailed {
            node: node.into(),
            attempt: 1,
            failure: daruda_flow::runner::NodeFailure::ContextExhausted,
        }
    }

    /// A node nobody has mentioned is pending, not missing.
    #[test]
    fn a_node_the_run_has_not_reached_is_pending() {
        let states = fold(&[started("design", 1)]);
        assert_eq!(
            states.get("design"),
            Some(&NodeRunState::Running { attempt: 1 })
        );
        assert_eq!(
            states.get("build").copied().unwrap_or_default(),
            NodeRunState::Pending
        );
    }

    /// The straight line through a run: start, pass, next.
    #[test]
    fn a_node_that_runs_and_passes_ends_passed() {
        let states = fold(&[started("design", 1), passed("design"), started("build", 1)]);
        assert_eq!(states.get("design"), Some(&NodeRunState::Passed));
        assert_eq!(
            states.get("build"),
            Some(&NodeRunState::Running { attempt: 1 })
        );
    }

    /// A second attempt is visible as one — the card says which try it is.
    #[test]
    fn a_retry_reports_its_attempt_number() {
        let states = fold(&[started("design", 1), failed("design"), started("design", 2)]);
        assert_eq!(
            states.get("design"),
            Some(&NodeRunState::Running { attempt: 2 })
        );
    }

    /// A gate under repair reads as fixing, and goes back to failed when the
    /// fix ends — it has not re-run yet, so it is not a pass.
    #[test]
    fn a_gate_under_repair_is_fixing_then_failed_again() {
        let mid = fold(&[
            failed("gate"),
            FlowEvent::FixStarted {
                gate: "gate".into(),
            },
        ]);
        assert_eq!(mid.get("gate"), Some(&NodeRunState::Fixing));

        let after = fold(&[
            failed("gate"),
            FlowEvent::FixStarted {
                gate: "gate".into(),
            },
            FlowEvent::FixEnded {
                gate: "gate".into(),
                failure: None,
            },
        ]);
        assert_eq!(after.get("gate"), Some(&NodeRunState::Failed));
    }

    /// The transition `event.rs` says a host cannot infer: a repair sends
    /// nodes that already passed back to pending. Miss it and the re-run set
    /// keeps the green of a result that no longer holds.
    #[test]
    fn rerunning_sends_its_members_back_to_pending() {
        let states = fold(&[
            started("design", 1),
            passed("design"),
            started("verdict", 1),
            passed("verdict"),
            failed("gate"),
            FlowEvent::Rerunning {
                gate: "gate".into(),
                members: vec!["verdict".into()],
            },
        ]);
        assert_eq!(
            states.get("verdict"),
            Some(&NodeRunState::Pending),
            "a re-derived node must lose its pass"
        );
        assert_eq!(
            states.get("design"),
            Some(&NodeRunState::Passed),
            "a node outside the rerun set keeps its result"
        );
        assert_eq!(states.get("gate"), Some(&NodeRunState::Failed));
    }

    /// The end of a run does not repaint anything: the last word on each node
    /// is the word that stands.
    #[test]
    fn the_end_of_a_run_settles_rather_than_clears() {
        let states = fold(&[
            started("design", 1),
            passed("design"),
            FlowEvent::RunEnded {
                end: daruda_flow::event::RunEnd::Done,
            },
        ]);
        assert_eq!(states.get("design"), Some(&NodeRunState::Passed));
    }

    fn colouring(of_nodes: &[&str]) -> RunColouring {
        RunColouring {
            states: NodeRunStates::new(),
            of_nodes: of_nodes
                .iter()
                .map(|s| daruda_flow::NodeId::from(*s))
                .collect(),
        }
    }

    /// A colouring is about the nodes the run was submitted with — the same ids
    /// in the same order. Anything else is a different flow wearing the same
    /// names, which is exactly the case that painted a green card on a node
    /// that never ran.
    #[test]
    fn a_colouring_knows_which_nodes_it_is_about() {
        let c = colouring(&["design", "build"]);
        assert!(c.is_about(["design", "build"].iter().map(|s| NodeId::from(*s))));
        assert!(
            !c.is_about(["design"].iter().map(|s| NodeId::from(*s))),
            "a node gone is a different flow"
        );
        assert!(
            !c.is_about(["design", "build", "ship"].iter().map(|s| NodeId::from(*s))),
            "and so is a node arrived"
        );
        assert!(
            !c.is_about(["build", "design"].iter().map(|s| NodeId::from(*s))),
            "order is part of it — the layout is derived from it"
        );
    }
}
