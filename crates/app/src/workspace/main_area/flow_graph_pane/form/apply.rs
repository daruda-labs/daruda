//! From the form's values onto the file's shape.
//!
//! Pure, and the only place that knows how a form maps onto a `FlowFile`. A
//! rename is this module moving one string and then every reference to it —
//! `deps` and a gate's `rerun` — and the differ turns that into the handful of
//! text edits it amounts to. Keeping it here rather than in the op means the
//! mapping is testable without a workspace, and that a deletion's sweep sits
//! next to the rename's.

use daruda_flow::NodeId;
use daruda_flow::parse::FlowFile;

use super::{AttemptsField, FailFields, KindChoice, NodeFields, SourceField, TimeoutField};

/// Write `fields` onto the node `node` names, mentions included.
///
/// Pure, and the only place that knows how a form maps onto the file: a rename
/// is this function moving one string and then every reference to it, which the
/// differ turns into the handful of text edits that says.
pub(in crate::workspace) fn node_fields(file: &mut FlowFile, node: &NodeId, fields: &NodeFields) {
    use daruda_flow::parse::{NodeKindFile, PromptSource};

    let Some(index) = file.nodes.iter().position(|n| &n.id == node) else {
        return;
    };
    let target = &mut file.nodes[index];
    target.deps = fields
        .deps
        .iter()
        .map(|d| NodeId::from(d.as_str()))
        .collect();
    target.timeout = match &fields.timeout {
        TimeoutField::Set(duration) => Some(*duration),
        // Unreadable text never reaches here — the form refuses to save on it.
        TimeoutField::Absent | TimeoutField::Unreadable(_) => None,
    };

    target.cwd = fields.cwd.as_ref().map(std::path::PathBuf::from);
    // The kind is the form's to say: switching it replaces the node's body
    // wholesale, and the boxes for the other kind have been alive all along.
    target.kind = match fields.body.kind {
        KindChoice::Agent => NodeKindFile::Agent {
            agent: fields.agent.to_override(),
            prompt: match &fields.body.prompt {
                SourceField::Inline(text) => PromptSource::Prompt(text.clone()),
                SourceField::File(path) => PromptSource::PromptFile(path.into()),
            },
            output: std::path::PathBuf::from(&fields.body.output),
            on_fail: retry_of(&target.kind, &fields.body.on_fail),
        },
        KindChoice::Command => NodeKindFile::Command {
            run: fields.body.run.clone(),
            on_fail: repair_of(&target.kind, &fields.body.on_fail),
        },
    };

    let new_id = NodeId::from(fields.id.as_str());
    if &new_id == node {
        return;
    }
    file.nodes[index].id = new_id.clone();
    for other in file.nodes.iter_mut() {
        for dep in other.deps.iter_mut() {
            if dep == node {
                *dep = new_id.clone();
            }
        }
        if let NodeKindFile::Command {
            on_fail: daruda_flow::parse::GateFailFile::Repair { rerun, .. },
            ..
        } = &mut other.kind
        {
            for name in rerun.iter_mut() {
                if name == node {
                    *name = new_id.clone();
                }
            }
        }
    }
}

/// A node the file does not have yet, chained after `after` when there is one.
///
/// Named `node-N` for the first `N` the file has no node for. Not asked for: the
/// name is a box in the inspector, and a dialog before every node would make
/// "add one and fill it in" two steps instead of one.
///
/// An agent with an empty prompt and an output named after it — the shape that
/// loads, so the graph draws the new card immediately and the person types into
/// it rather than reading a refusal.
pub(in crate::workspace) fn new_node(file: &mut FlowFile, after: Option<&NodeId>) -> NodeId {
    use daruda_flow::parse::{NodeFile, NodeKindFile, PromptSource};
    let id = next_node_id(file);
    file.nodes.push(NodeFile {
        id: id.clone(),
        deps: after.map(|dep| vec![dep.clone()]).unwrap_or_default(),
        timeout: None,
        cwd: None,
        kind: NodeKindFile::Agent {
            agent: None,
            prompt: PromptSource::Prompt(String::new()),
            output: std::path::PathBuf::from(format!("{id}.md")),
            on_fail: Default::default(),
        },
    });
    id
}

/// `node-1`, `node-2`, … — the first the file has no node for. Numbering from the
/// count would collide the moment a node is deleted.
fn next_node_id(file: &FlowFile) -> NodeId {
    (1..)
        .map(|n| NodeId::from(format!("node-{n}")))
        .find(|candidate| !file.nodes.iter().any(|node| &node.id == candidate))
        .unwrap_or_else(|| NodeId::from("node"))
}

/// Take `node` out, and out of everything that pointed at it — every `deps` and
/// every gate's `rerun`. The mirror of a rename, and the same rule: a name
/// written in three places has to move in all three.
/// How many nodes *outside* `going` would lose a reference if `going` were
/// removed — what the confirm dialog means by "and from what N other nodes run
/// after".
///
/// A set question, not a sum: a node that depends on two of the selection is
/// one loser, not two, and a node inside the selection is not a loser at all
/// because it is going too. Counting per-node and correcting arithmetically got
/// both of those wrong.
pub(in crate::workspace) fn dependents_outside(file: &FlowFile, going: &[NodeId]) -> usize {
    use daruda_flow::parse::{GateFailFile, NodeKindFile};
    file.nodes
        .iter()
        .filter(|node| !going.contains(&node.id))
        .filter(|node| {
            let rerun = match &node.kind {
                NodeKindFile::Command {
                    on_fail: GateFailFile::Repair { rerun, .. },
                    ..
                } => rerun.as_slice(),
                _ => &[],
            };
            node.deps
                .iter()
                .chain(rerun.iter())
                .any(|name| going.contains(name))
        })
        .count()
}

/// Make `into` wait for `out_of` — one line drawn on the canvas, as the file
/// spells it.
///
/// Records the dep on `into`, which is the direction
/// [`super::super::connect::dep_from_edge`] decided and the only thing about
/// this function that can be wrong. A link the file already has changes
/// nothing, and the differ turns that into `NothingToDo`; a node the file does
/// not have is likewise nothing, because the canvas it was drawn on is by then
/// out of date.
pub(in crate::workspace) fn connect(file: &mut FlowFile, out_of: &NodeId, into: &NodeId) {
    let Some(target) = file.nodes.iter_mut().find(|n| &n.id == into) else {
        return;
    };
    if target.deps.iter().any(|dep| dep == out_of) {
        return;
    }
    target.deps.push(out_of.clone());
}

pub(in crate::workspace) fn remove_node(file: &mut FlowFile, node: &NodeId) {
    use daruda_flow::parse::{GateFailFile, NodeKindFile};
    file.nodes.retain(|n| &n.id != node);
    for other in file.nodes.iter_mut() {
        other.deps.retain(|dep| dep != node);
        if let NodeKindFile::Command {
            on_fail: GateFailFile::Repair { rerun, .. },
            ..
        } = &mut other.kind
        {
            rerun.retain(|name| name != node);
        }
    }
}

/// The retry the form describes, starting from whatever the node held — so a
/// node that was already an agent keeps anything the form does not carry.
fn retry_of(
    current: &daruda_flow::parse::NodeKindFile,
    fields: &FailFields,
) -> daruda_flow::parse::AgentFailFile {
    let mut policy = match current {
        daruda_flow::parse::NodeKindFile::Agent { on_fail, .. } => on_fail.clone(),
        daruda_flow::parse::NodeKindFile::Command { .. } => Default::default(),
    };
    apply_retry(&mut policy, fields);
    policy
}

fn repair_of(
    current: &daruda_flow::parse::NodeKindFile,
    fields: &FailFields,
) -> daruda_flow::parse::GateFailFile {
    let mut policy = match current {
        daruda_flow::parse::NodeKindFile::Command { on_fail, .. } => on_fail.clone(),
        daruda_flow::parse::NodeKindFile::Agent { .. } => Default::default(),
    };
    apply_repair(&mut policy, fields);
    policy
}

/// An agent node's retry. Prose or a file, never both — which is what the engine
/// refuses (`ConflictingField`), and what this shape makes impossible.
fn apply_retry(policy: &mut daruda_flow::parse::AgentFailFile, fields: &FailFields) {
    use daruda_flow::parse::{AgentFailFile, HintSource};
    match fields {
        FailFields::Halt => *policy = AgentFailFile::Halt,
        FailFields::Retry {
            hint,
            max_attempts,
            wait,
        } => {
            *policy = AgentFailFile::Retry {
                hint: match hint {
                    SourceField::Inline(text) => HintSource::Hint(text.clone()),
                    SourceField::File(path) => HintSource::HintFile(path.into()),
                },
                max_attempts: attempts_or_one(max_attempts),
                wait: duration_of(wait),
            };
        }
        // A gate's policy on an agent node cannot happen: the form is built from
        // the node's own kind, and a mismatch means the file moved under it —
        // which the first gate has already refused.
        FailFields::Repair { .. } => {}
    }
}

/// A gate's repair.
fn apply_repair(policy: &mut daruda_flow::parse::GateFailFile, fields: &FailFields) {
    use daruda_flow::parse::GateFailFile;
    match fields {
        FailFields::Halt => *policy = GateFailFile::Halt,
        FailFields::Repair {
            fix,
            rerun,
            max_attempts,
            wait,
        } => {
            *policy = GateFailFile::Repair {
                fix: fix.clone(),
                rerun: rerun.iter().map(|r| NodeId::from(r.as_str())).collect(),
                max_attempts: attempts_or_one(max_attempts),
                wait: duration_of(wait),
            };
        }
        FailFields::Retry { .. } => {}
    }
}

/// The form refuses to save an unreadable count, so the fallback is only for a
/// shape that cannot arrive — one attempt, which is what "no retry" means.
fn attempts_or_one(field: &AttemptsField) -> u32 {
    match field {
        AttemptsField::Set(n) => *n,
        AttemptsField::Unreadable(_) => 1,
    }
}

fn duration_of(field: &TimeoutField) -> Option<std::time::Duration> {
    match field {
        TimeoutField::Set(d) => Some(*d),
        TimeoutField::Absent | TimeoutField::Unreadable(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::BodyFields;
    use super::*;
    use daruda_flow::parse::{GateFailFile, NodeKindFile};

    const GATE_FLOW: &str = "\
version: 1
nodes:
  - id: build
    kind: agent
    output: build.md
    prompt: build it
  - id: check
    kind: command
    deps: [build]
    run: cargo test
    on_fail:
      repair:
        fix: fix the tests
        rerun: [build]
        max_attempts: 2
";

    fn fields_renaming(to: &str) -> NodeFields {
        NodeFields {
            id: to.to_string(),
            deps: Vec::new(),
            timeout: TimeoutField::Absent,
            cwd: None,
            agent: Default::default(),
            body: BodyFields {
                kind: KindChoice::Agent,
                prompt: SourceField::Inline("build it".into()),
                output: "build.md".into(),
                run: String::new(),
                on_fail: FailFields::Halt,
            },
        }
    }

    /// A gate's `rerun` names nodes too, and it is the mention an integration
    /// test through the form does not reach — the form has no gate fields yet.
    #[test]
    fn a_rename_reaches_a_gates_rerun_list() {
        let mut file = daruda_flow::parse::parse_flow_file(GATE_FLOW).expect("fixture parses");
        node_fields(&mut file, &"build".into(), &fields_renaming("assemble"));

        assert_eq!(file.nodes[0].id, "assemble");
        assert_eq!(file.nodes[1].deps, vec![NodeId::from("assemble")]);
        let NodeKindFile::Command {
            on_fail: GateFailFile::Repair { rerun, .. },
            ..
        } = &file.nodes[1].kind
        else {
            panic!("the gate still repairs");
        };
        assert_eq!(
            rerun,
            &vec![NodeId::from("assemble")],
            "the repair reruns the node under its new name"
        );
    }

    #[test]
    fn a_new_node_takes_the_first_name_the_file_does_not_have() {
        let mut file = daruda_flow::parse::parse_flow_file(GATE_FLOW).expect("fixture parses");
        assert_eq!(new_node(&mut file, Some(&"check".into())), "node-1");
        assert_eq!(new_node(&mut file, None), "node-2");
        // A deletion frees the name again rather than the counter marching on.
        remove_node(&mut file, &"node-1".into());
        assert_eq!(new_node(&mut file, None), "node-1");
    }

    #[test]
    fn a_new_node_is_chained_after_the_one_it_was_added_from() {
        let mut file = daruda_flow::parse::parse_flow_file(GATE_FLOW).expect("fixture parses");
        let id = new_node(&mut file, Some(&"check".into()));
        let added = file.nodes.last().expect("it was added");
        assert_eq!(added.id, id);
        assert_eq!(added.deps, vec![NodeId::from("check")]);
    }

    /// The direction, carried through to the file. `connect.rs` asserts that a
    /// line out of `build` and into `check` *means* `check.deps += build`; this
    /// asserts the file ends up saying it, and — the half a reversed
    /// implementation would still pass — that `build` gained nothing.
    #[test]
    fn a_connection_is_recorded_on_the_card_it_was_drawn_into() {
        let mut file = daruda_flow::parse::parse_flow_file(GATE_FLOW).expect("fixture parses");
        connect(&mut file, &"check".into(), &"build".into());

        let build = file.nodes.iter().find(|n| n.id == "build").expect("build");
        assert_eq!(build.deps, vec![NodeId::from("check")]);
        let check = file.nodes.iter().find(|n| n.id == "check").expect("check");
        assert_eq!(
            check.deps,
            vec![NodeId::from("build")],
            "the card drawn out of keeps the deps it had and gains none"
        );
    }

    #[test]
    fn connecting_what_is_already_connected_changes_nothing() {
        let mut file = daruda_flow::parse::parse_flow_file(GATE_FLOW).expect("fixture parses");
        let before = file.clone();
        connect(&mut file, &"build".into(), &"check".into());
        assert_eq!(file, before, "`check` already runs after `build`");
    }

    /// The canvas the line was drawn on can be out of date by the time the
    /// write lands — a node that left is not an error, it is nothing to do.
    #[test]
    fn connecting_to_a_node_that_is_gone_changes_nothing() {
        let mut file = daruda_flow::parse::parse_flow_file(GATE_FLOW).expect("fixture parses");
        let before = file.clone();
        connect(&mut file, &"build".into(), &"drawing".into());
        assert_eq!(file, before);
    }

    /// A deletion is a rename's mirror: the name goes out of every place that
    /// held it, or the flow stops loading.
    #[test]
    fn a_deletion_takes_the_mentions_of_it_along() {
        let mut file = daruda_flow::parse::parse_flow_file(GATE_FLOW).expect("fixture parses");
        remove_node(&mut file, &"build".into());
        assert_eq!(file.nodes.len(), 1);
        assert!(file.nodes[0].deps.is_empty(), "the dep on it is gone");
        let NodeKindFile::Command {
            on_fail: GateFailFile::Repair { rerun, .. },
            ..
        } = &file.nodes[0].kind
        else {
            panic!("the gate still repairs");
        };
        assert!(rerun.is_empty(), "and so is the rerun root");
    }

    /// A node that is not there is not an error and not a change — the file the
    /// form was built against has moved on, which the first gate has refused.
    #[test]
    fn applying_to_a_node_that_is_gone_changes_nothing() {
        let mut file = daruda_flow::parse::parse_flow_file(GATE_FLOW).expect("fixture parses");
        let before = file.clone();
        node_fields(&mut file, &"nobody".into(), &fields_renaming("x"));
        assert_eq!(file, before);
    }
}

#[cfg(test)]
mod dependents_tests {
    use super::*;

    fn file(text: &str) -> FlowFile {
        daruda_flow::parse::parse_flow_file(text).expect("fixture parses")
    }

    const FORK: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: n1
    kind: agent
    output: n1.md
    prompt: one
  - id: n2
    kind: agent
    deps: [n1]
    output: n2.md
    prompt: two
  - id: n3
    kind: agent
    deps: [n1, n2]
    output: n3.md
    prompt: three
  - id: n5
    kind: agent
    deps: [n1]
    output: n5.md
    prompt: five
  - id: n6
    kind: agent
    deps: [n5]
    output: n6.md
    prompt: six
";

    fn going(ids: &[&str]) -> Vec<NodeId> {
        ids.iter().map(|s| NodeId::from(*s)).collect()
    }

    /// Two nodes in unrelated branches: each leaves its own dependent behind, so
    /// the answer is two. Summing per-node and subtracting the selection size
    /// said one.
    #[test]
    fn two_unrelated_removals_leave_two_dependents() {
        assert_eq!(dependents_outside(&file(FORK), &going(&["n2", "n5"])), 2);
    }

    /// A node that depends on *two* of the selection is one loser, not two.
    #[test]
    fn a_node_depending_on_two_of_them_is_counted_once() {
        assert_eq!(dependents_outside(&file(FORK), &going(&["n1", "n2"])), 2);
    }

    /// A dependent that is going too is not a change left behind.
    #[test]
    fn a_dependent_inside_the_selection_does_not_count() {
        assert_eq!(dependents_outside(&file(FORK), &going(&["n5", "n6"])), 0);
    }

    #[test]
    fn a_node_nothing_runs_after_leaves_nobody() {
        assert_eq!(dependents_outside(&file(FORK), &going(&["n6"])), 0);
    }
}
