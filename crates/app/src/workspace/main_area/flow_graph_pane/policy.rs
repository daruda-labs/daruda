//! What a reload should do, decided before anything is done.
//!
//! GPUI-free: every question here is about bytes, a model and a set of typed
//! values, and none of it needs a window. That matters because this is where
//! the pane's bugs have been — a graph rebuilt when it did not have to be, a
//! restored file that never redrew, typing thrown away without a word — and
//! each one had to be reproduced through a window to be seen at all.
//!
//! The view reads the file and carries the decision out. It does not make it.

use super::model::FlowGraphModel;
use super::{FlowGraphError, form::NodeFields};
use daruda_flow::NodeId;

/// What is on screen now, as the decision needs it. The canvas is deliberately
/// absent: it is an entity, and nothing here may hold one.
pub(super) enum Drawn<'a> {
    Graph(&'a FlowGraphModel),
    Nothing(&'a FlowGraphError),
}

/// What to do about the file as it now reads.
#[derive(Debug, PartialEq)]
pub(super) enum Reload {
    /// The file says what the pane already shows.
    Unchanged,
    /// Same nodes, same lines between them — the layout still holds, so the
    /// canvas stays and only the cards are written again. This is what keeps a
    /// save from moving the picture under the person who pressed it.
    Restamp { text: String, model: FlowGraphModel },
    /// A node arrived or left: the layout is genuinely different.
    Rebuild { text: String, model: FlowGraphModel },
    /// Say why instead of drawing.
    ///
    /// `text` is `Some` when the file was read but does not load — a later
    /// reload of those same bytes is then rightly nothing to do — and `None`
    /// when it could not be read at all, so that the *same* bytes coming back
    /// count as a change and redraw.
    Unreadable {
        text: Option<String>,
        error: FlowGraphError,
    },
}

/// Decide, given what the file reads as and what the pane shows.
pub(super) fn decide(
    incoming: Result<String, FlowGraphError>,
    shown_text: Option<&str>,
    drawn: Drawn<'_>,
) -> Reload {
    let text = match incoming {
        Ok(text) => text,
        Err(error) => {
            // Already saying this, about a file it could not read either: there
            // is nothing to change.
            if shown_text.is_none() && matches!(drawn, Drawn::Nothing(shown) if *shown == error) {
                return Reload::Unchanged;
            }
            return Reload::Unreadable { text: None, error };
        }
    };
    if shown_text == Some(text.as_str()) {
        return Reload::Unchanged;
    }
    match model_from(&text) {
        Ok(model) => match drawn {
            Drawn::Graph(current) if same_shape(current, &model) => Reload::Restamp { text, model },
            _ => Reload::Rebuild { text, model },
        },
        Err(error) => Reload::Unreadable {
            text: Some(text),
            error,
        },
    }
}

/// The model to draw, and what the engine refuses about it.
///
/// `inspect` rather than `load`: the graph-dependent rules run on a resolved
/// flow and a built graph, so a flow they refuse is one that can still be
/// drawn — and a card that says where the problem is beats a banner that took
/// the picture away. The stages before that leave nothing to draw and stay a
/// refusal here.
pub(super) fn model_from(text: &str) -> Result<FlowGraphModel, FlowGraphError> {
    match daruda_flow::inspect(text, None) {
        Ok(inspected) => Ok(FlowGraphModel::from_flow(
            inspected.loaded.flow(),
            inspected.issues,
        )),
        Err(daruda_flow::FlowError::Parse(detail)) => Err(FlowGraphError::Parse { detail }),
        // Worded here, not carried: `ValidationIssue.message` is the engine's
        // developer detail and says so, and `s::flow_issue` exists to be the
        // one place a `ValidationKind` becomes something a person reads.
        Err(daruda_flow::FlowError::Validate(issues)) => Err(FlowGraphError::Validate {
            issues: crate::surface::strings::flow_issue_lines(&issues),
        }),
    }
}

/// Everything the layout is derived from: the node ids in order, and both kinds
/// of line. What a card *says* — a prompt, an output, a policy — is not shape,
/// and changing one of those is the common save.
fn same_shape(a: &FlowGraphModel, b: &FlowGraphModel) -> bool {
    a.nodes
        .iter()
        .map(|n| &n.id)
        .eq(b.nodes.iter().map(|n| &n.id))
        && a.deps == b.deps
        && a.rerun == b.rerun
}

/// Whether what the person typed is still in front of them.
///
/// Not "was the form dirty": the pane that pressed Save is dirty too when its
/// own write comes back, and saying something on every successful save would be
/// noise. A node that is gone after the rebuild counts as lost — there is
/// nothing left to compare against.
pub(super) fn typing_survived(
    typed_node: &NodeId,
    typed: &NodeFields,
    rebuilt: Option<(&NodeId, &NodeFields)>,
) -> bool {
    rebuilt.is_some_and(|(node, fields)| node == typed_node && fields == typed)
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
  - id: build
    kind: agent
    deps: [design]
    output: build.md
    prompt: build it
";

    fn model(text: &str) -> FlowGraphModel {
        model_from(text).expect("the fixture loads")
    }

    #[test]
    fn the_same_bytes_are_nothing_to_do() {
        let current = model(TWO);
        assert_eq!(
            decide(Ok(TWO.to_string()), Some(TWO), Drawn::Graph(&current)),
            Reload::Unchanged
        );
    }

    /// The common save: one field, same nodes. The canvas has to survive it.
    #[test]
    fn a_changed_field_keeps_the_layout() {
        let current = model(TWO);
        let edited = TWO.replace("build it", "build it twice");
        assert!(matches!(
            decide(Ok(edited), Some(TWO), Drawn::Graph(&current)),
            Reload::Restamp { .. }
        ));
    }

    #[test]
    fn a_node_arriving_or_leaving_is_a_new_layout() {
        let current = model(TWO);
        let gone = TWO.replace(
            "  - id: build\n    kind: agent\n    deps: [design]\n    output: build.md\n    prompt: build it\n",
            "",
        );
        assert!(matches!(
            decide(Ok(gone), Some(TWO), Drawn::Graph(&current)),
            Reload::Rebuild { .. }
        ));
    }

    /// A line changing moves nothing on a card and everything in the layout.
    #[test]
    fn a_dependency_changing_is_a_new_layout() {
        let current = model(TWO);
        let unchained = TWO.replace("    deps: [design]\n", "");
        assert!(matches!(
            decide(Ok(unchained), Some(TWO), Drawn::Graph(&current)),
            Reload::Rebuild { .. }
        ));
    }

    /// The trap a restored file walked into: nothing on screen, and bytes that
    /// match what the pane last held. Keeping the text would make this
    /// `Unchanged` and leave the pane on its message for good — so a fall to
    /// `Unreadable` drops the text with it.
    #[test]
    fn a_file_that_could_not_be_read_keeps_no_text() {
        let error = FlowGraphError::Parse {
            detail: "gone".into(),
        };
        assert_eq!(
            decide(Err(error.clone()), Some(TWO), Drawn::Graph(&model(TWO))),
            Reload::Unreadable { text: None, error }
        );
    }

    /// But a file that reads and does not load keeps its bytes: reading the
    /// same broken file again is genuinely nothing to do.
    #[test]
    fn a_file_that_does_not_load_keeps_its_bytes() {
        let broken = "nodes: [".to_string();
        let Reload::Unreadable { text, .. } =
            decide(Ok(broken.clone()), Some(TWO), Drawn::Graph(&model(TWO)))
        else {
            panic!("a file that does not load is unreadable");
        };
        assert_eq!(text.as_deref(), Some(broken.as_str()));
    }

    #[test]
    fn the_same_failure_twice_is_nothing_to_do() {
        let error = FlowGraphError::Parse {
            detail: "same".into(),
        };
        assert_eq!(
            decide(Err(error.clone()), None, Drawn::Nothing(&error)),
            Reload::Unchanged
        );
        let other = FlowGraphError::Parse {
            detail: "different".into(),
        };
        assert!(matches!(
            decide(Err(other), None, Drawn::Nothing(&error)),
            Reload::Unreadable { .. }
        ));
    }

    fn fields(output: &str) -> NodeFields {
        use super::super::form::{BodyFields, FailFields, KindChoice, SourceField, TimeoutField};
        NodeFields {
            id: "design".into(),
            deps: Vec::new(),
            timeout: TimeoutField::Absent,
            cwd: None,
            agent: Default::default(),
            body: BodyFields {
                kind: KindChoice::Agent,
                prompt: SourceField::Inline("write a line".into()),
                output: output.to_string(),
                run: String::new(),
                on_fail: FailFields::Halt,
            },
        }
    }

    #[test]
    fn typing_is_lost_unless_the_same_node_comes_back_saying_it() {
        let mine = fields("mine.md");
        assert!(typing_survived(
            &"design".into(),
            &mine,
            Some((&"design".into(), &mine))
        ));
        assert!(
            !typing_survived(
                &"design".into(),
                &mine,
                Some((&"design".into(), &fields("theirs.md")))
            ),
            "the file's value replaced it"
        );
        assert!(
            !typing_survived(&"design".into(), &mine, Some((&"drawing".into(), &mine))),
            "a different node is not where it was typed"
        );
        assert!(
            !typing_survived(&"design".into(), &mine, None),
            "and nothing coming back is the case most worth saying"
        );
    }
}
