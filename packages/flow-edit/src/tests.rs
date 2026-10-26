use super::{Step, value_at_path};
use yaml_serde::Value;

/// The coordinate pair, asserted where it is read.
///
/// `value_at_path` is the only reader of a step's `value`, and the whole point
/// of the pair is that a removed element has no place in the new tree. Both
/// sequence differs build removals that way now; the keyed one was fixed after
/// it wrote a survivor's `deps` with another node's value, and the positional
/// one was still claiming a place it did not have.
#[test]
fn a_removed_elements_step_reads_nothing_from_the_new_tree() {
    let tree: Value = yaml_serde::from_str("nodes: [a, b]").expect("fixture");
    let kept = vec![
        Step::Key("nodes".to_string()),
        Step::Index {
            text: 1,
            value: Some(1),
        },
    ];
    assert_eq!(
        value_at_path(&tree, &kept).and_then(Value::as_str),
        Some("b"),
        "an element that is still there reads as itself"
    );

    let going = vec![
        Step::Key("nodes".to_string()),
        Step::Index {
            text: 1,
            value: None,
        },
    ];
    assert!(
        value_at_path(&tree, &going).is_none(),
        "and one that is going reads as nothing — not as whatever sits at its old index"
    );
}
