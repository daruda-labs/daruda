//! Where a pinned node's finished output comes from.
//!
//! A pin names a node; the engine wants a file. Closing that gap needs a
//! *finished run* to read from, and the only witness to one is disk: a run
//! handle is dropped the moment the run ends (`flow_runs.rs`), so nothing in
//! memory can answer this afterwards — including after a restart.
//!
//! **The newest run is the whole answer.** That is not a shortcut: the engine
//! copies every pinned output into the new run's directory before its first
//! node, so a run started with pins accumulates them, and the last run holds
//! everything the one before it held. Reaching further back would be reading a
//! file the flow has since moved past, with nothing on screen saying so.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use daruda_flow::NodeId;

/// The resolved spec every run directory holds — what says which flow it was.
const RUN_SPEC: &str = "run.yaml";
use daruda_flow::request::PinnedOutput;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::Context;

use super::Workspace;
use crate::surface::strings as s;

/// What each pin turned into: a file to copy, or nothing.
#[derive(Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) struct ResolvedPins {
    pub reuse: Vec<PinnedOutput>,
    /// Pins with nowhere to read from. Kept apart rather than dropped inside
    /// the loop because sending one of these would have `validate_request`
    /// refuse the *whole* run, and swallowing it silently would charge for a
    /// node the person had said not to charge for without saying why.
    pub unavailable: Vec<NodeId>,
}

/// Resolve `pinned` against `run_dir`, the run expected to hold their outputs.
///
/// A pin resolves only when the run's own spec still describes that node the
/// way this flow does. Matching on the id and the output path alone was wrong
/// and quietly so: two flows in one lane can both declare `design ->
/// design.md`, and the newest run in the lane is not necessarily a run of
/// *this* flow — so a pin would copy an unrelated output in and skip the node
/// with nothing said.
///
/// `run.yaml` is that spec, and it is resolved, so both sides go through
/// `load` and the comparison is of resolved nodes. Deliberately the whole
/// node and not just its prompt: the pane clears a pin on any change to its
/// node's definition (`flow_graph_pane::pins::surviving`), and one rule that
/// both places share beats two that nearly agree.
pub(in crate::workspace) fn resolve_in(
    pinned: &[NodeId],
    flow_text: &str,
    profile: Option<&str>,
    run_dir: Option<&Path>,
) -> ResolvedPins {
    let unavailable = || ResolvedPins {
        reuse: Vec::new(),
        unavailable: pinned.to_vec(),
    };
    let Some(run_dir) = run_dir else {
        return unavailable();
    };
    // The run's spec, read the way the run itself was: `run.yaml` records the
    // resolved flow, so it takes no profile of its own.
    let Ok(spec) = std::fs::read_to_string(run_dir.join(RUN_SPEC)) else {
        return unavailable();
    };
    let (Ok(then), Ok(now)) = (
        daruda_flow::load(&spec, None),
        daruda_flow::load(flow_text, profile),
    ) else {
        return unavailable();
    };
    let passed: HashSet<NodeId> = daruda_flow::journal::read(run_dir)
        .passed
        .into_iter()
        .collect();

    let mut resolved = ResolvedPins::default();
    for id in pinned {
        match reusable_output(then.flow(), now.flow(), id) {
            Some(relative) if passed.contains(id) => {
                let from = run_dir.join(relative);
                if from.is_file() {
                    resolved.reuse.push(PinnedOutput {
                        node: id.clone(),
                        from,
                    });
                    continue;
                }
                resolved.unavailable.push(id.clone());
            }
            _ => resolved.unavailable.push(id.clone()),
        }
    }
    resolved
}

/// The output `id` owes, when the run that produced it described the node the
/// way this flow does. `None` the moment the two disagree — including when
/// either side has no such node at all.
fn reusable_output(
    then: &daruda_flow::model::Flow,
    now: &daruda_flow::model::Flow,
    id: &NodeId,
) -> Option<PathBuf> {
    let find =
        |flow: &daruda_flow::model::Flow| flow.nodes.iter().find(|node| &node.id == id).cloned();
    let (then, now) = (find(then)?, find(now)?);
    if then != now {
        return None;
    }
    match now.kind {
        daruda_flow::model::NodeKind::Agent { output, .. } => Some(output),
        daruda_flow::model::NodeKind::Command { .. } => None,
    }
}

impl Workspace {
    /// Turn this lane's pins into outputs the engine can copy, and say what
    /// could not be honoured.
    ///
    /// The history is read through the shared cache rather than freshly: it is
    /// dropped whenever a run starts or settles, which is exactly when the
    /// newest run changes.
    pub(in crate::workspace) fn resolve_flow_pins(
        &mut self,
        flow_path: &Path,
        profile: Option<&str>,
        pinned: &[NodeId],
        cx: &mut Context<Self>,
    ) -> Vec<PinnedOutput> {
        if pinned.is_empty() {
            return Vec::new();
        }
        let newest = self
            .flow_history_of_active_lane()
            .and_then(|history| history.runs().first().map(|run| run.dir.clone()));
        let text = std::fs::read_to_string(flow_path).unwrap_or_default();
        let resolved = resolve_in(pinned, &text, profile, newest.as_deref());
        if !resolved.unavailable.is_empty() {
            self.report_error(
                ErrorReport::new(s::flow_pin_unavailable(&resolved.unavailable))
                    .severity(ErrorSeverity::Warning)
                    .dedup("flow.pin_unavailable")
                    .at(file!(), line!())
                    .build(),
                cx,
            );
            // The toast is dismissed in a moment; the card is not. A pin left
            // set after the source it named is gone keeps drawing as reused
            // and keeps paying for the node underneath it.
            self.forget_pins(flow_path, &resolved.unavailable, cx);
        }
        resolved.reuse
    }
}

impl Workspace {
    /// Clear `nodes` from the pane drawing `flow_path`, if one is open.
    ///
    /// Nothing to do when it is not: pins live in the pane, so a flow run from
    /// the picker with no canvas open has none to forget.
    fn forget_pins(&mut self, flow_path: &Path, nodes: &[NodeId], cx: &mut Context<Self>) {
        let Some(view) = self
            .find_flow_graph_pane(flow_path)
            .and_then(|pane| self.flow_graph_of_pane(pane))
            .map(|(_, view)| view)
        else {
            return;
        };
        view.update(cx, |view, cx| view.drop_pins(nodes, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write a line
  - id: gate
    kind: command
    run: \"true\"
";

    /// A run directory as the engine leaves one: the spec it ran, a journal
    /// saying what passed, and the outputs. Written as the engine writes them,
    /// because reading those back is exactly what is under test.
    fn run_dir(spec: &str, passed: &[&str], files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(RUN_SPEC), spec).expect("the run's spec");
        let mut journal = "{\"kind\":\"started\",\"v\":1,\"profile\":null}\n".to_string();
        for node in passed {
            journal.push_str(&format!(
                "{{\"kind\":\"attempt\",\"v\":1,\"node\":\"{node}\",\"attempt\":1,\
                 \"evidence_seq\":1,\"outcome\":{{\"result\":\"passed\"}},\
                 \"spent\":{{\"node_runs\":1}}}}\n"
            ));
        }
        std::fs::write(dir.path().join(daruda_flow::journal::JOURNAL_FILE), journal)
            .expect("journal");
        for file in files {
            std::fs::write(dir.path().join(file), "done\n").expect("output");
        }
        dir
    }

    #[test]
    fn a_node_that_passed_resolves_to_that_runs_output() {
        let dir = run_dir(TWO, &["design"], &["design.md"]);
        let resolved = resolve_in(&["design".into()], TWO, None, Some(dir.path()));
        assert_eq!(
            resolved.reuse,
            vec![PinnedOutput {
                node: "design".into(),
                from: dir.path().join("design.md"),
            }]
        );
        assert!(resolved.unavailable.is_empty());
    }

    /// The defect this rule exists for: two flows in one lane can both declare
    /// `design -> design.md`, and the newest run in the lane need not be a run
    /// of *this* flow. Matching on the id and the path alone copied an
    /// unrelated output in and skipped the node with nothing said.
    #[test]
    fn a_run_of_another_flow_satisfies_nothing() {
        const OTHER: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: a different job entirely
";
        let dir = run_dir(OTHER, &["design"], &["design.md"]);
        let resolved = resolve_in(&["design".into()], TWO, None, Some(dir.path()));
        assert!(resolved.reuse.is_empty(), "{:?}", resolved.reuse);
        assert_eq!(resolved.unavailable, vec![NodeId::from("design")]);
    }

    /// A run directory with no spec cannot say which flow it was, so it
    /// satisfies nothing either — the same answer as disagreeing.
    #[test]
    fn a_run_with_no_spec_satisfies_nothing() {
        let dir = run_dir(TWO, &["design"], &["design.md"]);
        std::fs::remove_file(dir.path().join(RUN_SPEC)).expect("drop the spec");
        let resolved = resolve_in(&["design".into()], TWO, None, Some(dir.path()));
        assert!(resolved.reuse.is_empty(), "{:?}", resolved.reuse);
    }

    /// Sending a pin the engine cannot satisfy refuses the whole run, so a
    /// node the newest run never finished must not be sent at all.
    #[test]
    fn a_node_that_did_not_pass_is_not_sent() {
        let dir = run_dir(TWO, &[], &["design.md"]);
        let resolved = resolve_in(&["design".into()], TWO, None, Some(dir.path()));
        assert!(resolved.reuse.is_empty(), "{:?}", resolved.reuse);
        assert_eq!(resolved.unavailable, vec![NodeId::from("design")]);
    }

    /// Passed and yet the file is gone — a swept or hand-deleted run
    /// directory. The journal alone is not enough to promise a copy.
    #[test]
    fn a_missing_file_is_not_sent_even_when_the_journal_says_it_passed() {
        let dir = run_dir(TWO, &["design"], &[]);
        let resolved = resolve_in(&["design".into()], TWO, None, Some(dir.path()));
        assert!(resolved.reuse.is_empty());
        assert_eq!(resolved.unavailable, vec![NodeId::from("design")]);
    }

    /// A gate declares no output, so there is no file to copy in — pinning one
    /// can only ever be reported back.
    #[test]
    fn a_command_node_has_no_output_to_reuse() {
        let dir = run_dir(TWO, &["gate"], &[]);
        let resolved = resolve_in(&["gate".into()], TWO, None, Some(dir.path()));
        assert!(resolved.reuse.is_empty());
        assert_eq!(resolved.unavailable, vec![NodeId::from("gate")]);
    }

    /// A lane that has never run this flow has nothing to reuse, and says so
    /// rather than pointing the engine at a path it invented.
    #[test]
    fn no_previous_run_leaves_every_pin_unavailable() {
        let resolved = resolve_in(&["design".into()], TWO, None, None);
        assert!(resolved.reuse.is_empty());
        assert_eq!(resolved.unavailable, vec![NodeId::from("design")]);
    }
}
