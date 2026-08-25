//! The words the contract is stated in, asserted whole: the block a node
//! that owes a file is told, and the correction sent when it did not keep it.

use super::*;
use crate::runner::BreachKind;

/// The absolute path is the one thing the agent cannot derive, so it is
/// the one thing the block must carry verbatim — and a node that declares
/// no shape is told nothing else, the clause being appended, never folded
/// in.
#[test]
fn the_block_states_the_absolute_path_under_a_validated_header() {
    let text = block(Path::new("/tmp/run/design.md"), None);
    assert_eq!(
        text,
        "OUTPUT CONTRACT (machine-validated):\n\
         When you are done, write your result to /tmp/run/design.md.\n\
         The file must exist and be non-empty; a symlink is refused."
    );
}

/// A declared shape is stated in the terms the check judges it by: JSON,
/// the whole schema, and the one thing the check deliberately does *not*
/// ask — so an agent is never refused for a rule it was not told.
#[test]
fn a_declared_shape_is_stated_as_json_with_the_rule_that_is_not_enforced() {
    let schema: SchemaSubset = yaml_serde::from_str(
        "\
type: object
required: [verdict]
properties:
  verdict: { type: string, enum: [pass, fail] }
",
    )
    .expect("the fixture is a schema");
    let text = block(Path::new("/tmp/run/verdict.json"), Some(&schema));
    assert_eq!(
        text,
        "OUTPUT CONTRACT (machine-validated):\n\
         When you are done, write your result to /tmp/run/verdict.json.\n\
         The file must exist and be non-empty; a symlink is refused.\n\
         Its contents must be a single JSON value matching this schema, with no prose \
         and no code fence around it. Properties the schema does not name are ignored:\n\
         {\n  \"type\": \"object\",\n  \"required\": [\n    \"verdict\"\n  ],\n  \
         \"properties\": {\n    \"verdict\": {\n      \"type\": \"string\",\n      \
         \"enum\": [\n        \"pass\",\n        \"fail\"\n      ]\n    }\n  }\n}"
    );
}

fn breach(first: &str, rest: Vec<String>) -> ContractBreach {
    ContractBreach {
        kind: BreachKind::Missing {
            expected: std::path::PathBuf::from("/tmp/run/design.md"),
        },
        first: first.to_string(),
        rest,
    }
}

/// The whole correction, spelled out: the reason is the check's own
/// wording, and the contract is referred to rather than repeated.
#[test]
fn the_correction_carries_the_reason_and_points_back_at_the_contract() {
    let text = correction(&breach(
        "nothing usable is at /tmp/run/design.md: the file is absent or empty",
        Vec::new(),
    ));
    assert_eq!(
        text,
        "Your previous turn ended without satisfying the OUTPUT CONTRACT above.\n\
         - nothing usable is at /tmp/run/design.md: the file is absent or empty\n\
         Satisfy the contract now. Change nothing else."
    );
}

/// A breach carries a lead reason and possibly more; every one of them
/// reaches the agent, because a correction told half the story produces
/// half a fix.
#[test]
fn every_reason_reaches_the_agent() {
    let text = correction(&breach(
        "the file is empty",
        vec!["it needs a `## Risks` section".to_string()],
    ));
    assert!(text.contains("- the file is empty"), "{text}");
    assert!(text.contains("- it needs a `## Risks` section"), "{text}");
}
