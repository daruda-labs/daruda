//! What `validate` refuses, stated as the flow file an author would have
//! written. Each fixture is the smallest file that reaches one rule, so a
//! failure names the rule rather than the parser.

use super::*;
use crate::error::{ValidationIssue, ValidationKind};
use crate::graph::FlowGraph;
use crate::parse::parse_flow_file;
use crate::resolve::resolve;

fn issues_for(text: &str) -> Vec<ValidationIssue> {
    let flow = resolve(parse_flow_file(text).expect("parses"), None).expect("resolves");
    let graph = FlowGraph::build(&flow).expect("acyclic");
    validate(&flow, &graph)
}

/// A schema keyword this build does not enforce is refused **softly**: the
/// file still parses, so the refusal names the node and is worded for a
/// reader. `deny_unknown_fields` on the schema type would have made it a
/// raw serde error instead. The canvas goes either way — the pane maps a
/// validation failure to `Reload::Unreadable` exactly as it does a parse
/// one.
#[test]
fn an_unenforced_schema_keyword_is_a_node_named_issue_on_a_readable_file() {
    for keyword in ["additionalProperties: false", "$ref: '#/defs/v'"] {
        let text = format!(
            "\
version: 1
defaults: {{ agent: {{ id: claude }} }}
nodes:
  - id: design
    kind: agent
    output: design.json
    prompt: write
    output_schema:
      type: object
      {keyword}
      properties:
        verdict: {{ type: string }}
"
        );
        crate::parse::parse_flow_file(&text)
            .unwrap_or_else(|e| panic!("{keyword} must leave the file readable: {e}"));
        let issues = issues_for(&text);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].node.as_ref(), Some(&NodeId::from("design")));
        assert!(
            matches!(
                issues[0].kind,
                ValidationKind::UnsupportedSchemaKeyword { .. }
            ),
            "{:?}",
            issues[0].kind
        );
    }
}

/// The design document's flagship example. It must pass its own
/// validator — earlier revisions of the design shipped an example that
/// did not, twice, and a third that was not even valid YAML.
///
/// Note the quoted `run:` on the gate: a plain YAML scalar may not hold
/// `": "`, which `VERDICT: PASS` does.
const FLAGSHIP: &str = "\
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
    prompt: Read DESIGN.md and write the design to {{output}}.
  - id: implement
    kind: agent
    deps: [design]
    prompt: Implement the design in {{node.design.output}}.
    output: implement.md
  - id: test
    kind: command
    deps: [implement]
    run: cargo test
    on_fail:
      repair:
        fix: \"{{failure}}. Read {{attempts}} and fix the cause.\"
        max_attempts: 2
  - id: review
    kind: agent
    deps: [test]
    output: review.md
    prompt: Review {{node.implement.output}} and write VERDICT first.
  - id: review-gate
    kind: command
    deps: [review]
    run: \"grep -q '^VERDICT: PASS' {{node.review.output}}\"
    on_fail:
      repair:
        fix: Apply the review notes in {{attempts}}.
        rerun: [review]
        max_attempts: 2
";

#[test]
fn the_flagship_example_passes_its_own_validator() {
    assert_eq!(issues_for(FLAGSHIP), Vec::new());
}

/// `review` reads `implement`'s output while depending only on `test`.
/// Legal: `deps` declares order, and `implement` is still an ancestor,
/// so its output is guaranteed to exist. Checking `deps` instead of
/// ancestors would reject the flagship example above.
#[test]
fn an_output_reference_to_a_non_dep_ancestor_is_allowed() {
    assert!(
        !issues_for(FLAGSHIP)
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::UnreachableOutputRef { .. }))
    );
}

#[test]
fn an_output_reference_to_a_non_ancestor_is_rejected() {
    let issues = issues_for(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
  - id: b
    kind: agent
    output: b.md
    prompt: read {{node.a.output}}
",
    );
    assert!(
        issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::UnreachableOutputRef { .. })),
        "b does not depend on a, so a's output may not exist yet: {issues:?}"
    );
}

/// The same rule applies to a retry hint — it is one of the four
/// template-bearing fields, and a hint reading a non-ancestor's output
/// is the same defect as a prompt doing it.
#[test]
fn an_output_reference_inside_a_retry_hint_is_checked() {
    let issues = issues_for(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
  - id: b
    kind: agent
    output: b.md
    prompt: write
    on_fail:
      retry:
        max_attempts: 2
        hint: look at {{node.a.output}}
",
    );
    assert!(
        issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::UnreachableOutputRef { .. }))
    );
}

/// An unrelated `{{node.x}}` earlier in the prompt must not swallow the
/// text up to a later `.output}}` — each `{{ }}` pair is scanned on its
/// own, so a legal reference after a non-output template still passes.
#[test]
fn an_unrelated_template_before_a_legal_output_ref_does_not_break_the_scan() {
    let issues = issues_for(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
  - id: b
    kind: agent
    deps: [a]
    output: b.md
    prompt: \"{{node.x}} see {{node.a.output}}\"
",
    );
    assert!(
        !issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::UnreachableOutputRef { .. })),
        "a is an ancestor of b, so its output reference is legal: {issues:?}"
    );
}

#[test]
fn duplicate_output_paths_are_rejected() {
    let issues = issues_for(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: shared.md
    prompt: write
  - id: b
    kind: agent
    deps: [a]
    output: shared.md
    prompt: write
",
    );
    assert!(
        issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::DuplicateOutput))
    );
}

/// `./out.md` and `out.md` name the same file once `.` components are
/// stripped — the collision must fire even though the strings differ.
/// macOS ships a case-insensitive filesystem by default, so these are
/// one file: the second node overwrites the first, and whatever reads
/// `{{node.first.output}}` downstream gets work it did not ask for
/// with nothing saying so. Verified against a real APFS volume, not
/// assumed.
#[test]
fn output_paths_differing_only_in_case_are_rejected() {
    let issues = issues_for(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: out.md
    prompt: write
  - id: b
    kind: agent
    output: Out.md
    prompt: write
",
    );
    assert!(
        issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::DuplicateOutput)),
        "{issues:?}"
    );
}

/// Same filesystem, same reasoning: `LOGS/` reaches the directory the
/// engine keeps its archives and runner logs in.
#[test]
fn an_output_in_the_reserved_directory_is_rejected_whatever_its_case() {
    let issues = issues_for(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: LOGS/out.md
    prompt: write
",
    );
    assert!(
        issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::OutputInReservedDir { .. })),
        "{issues:?}"
    );
}

/// The gap case-folding alone left, and the one that reaches past Latin:
/// the same word composed and decomposed is one file on macOS's default
/// filesystem, so a flow naming both silently has one node overwrite the
/// other. Verified against a real APFS volume for Latin, Hangul and kana
/// before this rule was written.
#[test]
fn output_paths_differing_only_in_unicode_normalisation_are_rejected() {
    for (composed, decomposed) in [("각.md", "각.md"), ("Å.md", "Å.md"), ("が.md", "が.md")]
    {
        assert_ne!(
            composed, decomposed,
            "the fixture only bites while the two spellings differ"
        );
        let issues = issues_for(&format!(
            "\
version: 1
defaults: {{ agent: {{ id: claude }} }}
nodes:
  - id: a
    kind: agent
    output: \"{composed}\"
    prompt: write
  - id: b
    kind: agent
    output: \"{decomposed}\"
    prompt: write
"
        ));
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::DuplicateOutput)),
            "`{composed}` and `{decomposed}` are one file: {issues:?}"
        );
    }
}

#[test]
fn duplicate_output_paths_with_a_leading_cur_dir_are_rejected() {
    let issues = issues_for(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: out.md
    prompt: write
  - id: b
    kind: agent
    deps: [a]
    output: ./out.md
    prompt: write
",
    );
    assert!(
        issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::DuplicateOutput)),
        "./out.md and out.md are the same path: {issues:?}"
    );
}

#[test]
fn an_output_escaping_the_run_directory_is_rejected() {
    let issues = issues_for(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: ../outside.md
    prompt: write
",
    );
    assert!(
        issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::OutputEscapesRunDir))
    );
}

#[test]
fn a_rerun_root_outside_the_gates_ancestors_is_rejected() {
    let issues = issues_for(
        "\
version: 1
nodes:
  - id: a
    kind: command
    run: \"true\"
  - id: b
    kind: command
    run: \"true\"
  - id: gate
    kind: command
    deps: [a]
    run: \"true\"
    on_fail:
      repair:
        fix: fix it from {{attempts}}
        rerun: [b]
        max_attempts: 2
",
    );
    assert!(
        issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::RerunNotAnAncestor { .. })),
        "b is not an ancestor of gate, so re-running it cannot change the verdict"
    );
}

#[test]
fn a_repair_that_names_no_failure_channel_is_rejected() {
    let issues = issues_for(
        "\
version: 1
nodes:
  - id: gate
    kind: command
    run: \"true\"
    on_fail:
      repair:
        fix: just try again
        max_attempts: 2
",
    );
    assert!(
        issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::RepairWithoutFailureContext))
    );
}

/// A repair runs its `fix` in an agent session, so a flow that declares
/// one without any agent to run it cannot be executed.
#[test]
fn a_repair_in_a_flow_with_no_agent_is_rejected() {
    let issues = issues_for(
        "\
version: 1
nodes:
  - id: gate
    kind: command
    run: \"true\"
    on_fail:
      repair:
        fix: fix it from {{attempts}}
        max_attempts: 2
",
    );
    assert!(
        issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::RepairWithoutAgent)),
        "nothing here can run the fix prompt: {issues:?}"
    );
}

/// An id becomes a filename in `logs/`, so a separator in one steers
/// `archive_attempt`'s rename out of the run directory entirely — it
/// moves a real output to wherever the id points.
#[test]
fn an_id_that_is_not_a_safe_filename_is_rejected() {
    for id in ["../../pwned", "/tmp/abs", "a/b", "a b", "design.v2", ""] {
        let issues = issues_for(&format!(
            "\
version: 1
defaults: {{ agent: {{ id: claude }} }}
nodes:
  - id: \"{id}\"
    kind: agent
    output: out.md
    prompt: write
"
        ));
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::InvalidNodeId)),
            "`{id}` must be rejected, got {issues:?}"
        );
    }
}

#[test]
fn an_ordinary_id_is_accepted() {
    let issues = issues_for(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design_v2-final
    kind: agent
    output: out.md
    prompt: write
",
    );
    assert!(issues.is_empty(), "{issues:?}");
}

/// `logs/` holds the archived evidence, named from node id and attempt.
/// An output living there can be renamed onto itself, which leaves the
/// failed attempt's file live for the next attempt to inherit.
#[test]
fn an_output_inside_the_engines_own_directory_is_rejected() {
    for output in ["logs/review.md", "./logs/deep/review.md"] {
        let issues = issues_for(&format!(
            "\
version: 1
defaults: {{ agent: {{ id: claude }} }}
nodes:
  - id: review
    kind: agent
    output: {output}
    prompt: write
"
        ));
        assert!(
            issues
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::OutputInReservedDir { .. })),
            "`{output}` must be rejected, got {issues:?}"
        );
    }
}

/// The archive and the runner log both key on `RunContext.node_id`, so a
/// node that takes the repair session's id makes its own artifacts and
/// the fix's indistinguishable.
#[test]
fn a_node_that_takes_the_repair_sessions_id_is_rejected() {
    let issues = issues_for(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: __fix__
    kind: agent
    output: fix.md
    prompt: write
",
    );
    assert!(
        issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::ReservedNodeId)),
        "{issues:?}"
    );
}

/// Each of the three ways a `continue_until` cannot be read fails the
/// same way if unchecked: the node spends every turn it is allowed, never
/// matches, and fails on the cap — having paid for a session per turn.
/// The point of these rules is that it happens at load instead.
#[test]
fn a_continue_until_nothing_could_read_is_rejected() {
    type Wanted = fn(&ValidationKind) -> bool;
    let cases: [(&str, Wanted); 3] = [
        // No schema at all: nothing to read a field out of.
        (
            "    continue_until: { field: state, equals: done }\n",
            |k| matches!(k, ValidationKind::ContinueUntilWithoutObjectSchema),
        ),
        // A schema that does not declare the field.
        (
            "    continue_until: { field: state, equals: done }
    output_schema:
      type: object
      required: [other]
      properties:
        other: { type: string }
",
            |k| matches!(k, ValidationKind::ContinueUntilFieldNotDeclared { .. }),
        ),
        // Declared but optional: an output that omits it would count as
        // finished, which is the opposite of what was asked.
        (
            "    continue_until: { field: state, equals: done }
    output_schema:
      type: object
      properties:
        state: { type: string }
",
            |k| matches!(k, ValidationKind::ContinueUntilFieldNotRequired { .. }),
        ),
    ];
    for (tail, wanted) in cases {
        let issues = issues_for(&format!(
            "\
version: 1
defaults: {{ agent: {{ id: claude }} }}
nodes:
  - id: work
    kind: agent
    output: out.json
    prompt: write
{tail}"
        ));
        assert!(
            issues.iter().any(|i| wanted(&i.kind)),
            "for {tail:?} got {issues:?}"
        );
    }
}

/// The one easiest to write by accident: the enum and the verdict are two
/// lines apart, and a typo in either reads fine. Without this rule the
/// node spends every turn it is allowed — a paid session each — and fails
/// on the cap having never had a value it could report.
#[test]
fn a_continue_until_waiting_for_a_value_the_enum_forbids_is_rejected() {
    let issues = issues_for(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: work
    kind: agent
    output: out.json
    prompt: write
    continue_until: { field: state, equals: complete }
    output_schema:
      type: object
      required: [state]
      properties:
        state: { type: string, enum: [in_progress, done] }
",
    );
    assert!(
        issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::ContinueUntilValueNotAllowed { .. })),
        "{issues:?}"
    );
}

/// A field with no `enum` takes any value, so there is nothing to check
/// against — the rule must not refuse that.
#[test]
fn a_continue_until_on_a_field_with_no_enum_is_accepted() {
    let issues = issues_for(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: work
    kind: agent
    output: out.json
    prompt: write
    continue_until: { field: state, equals: anything }
    output_schema:
      type: object
      required: [state]
      properties:
        state: { type: string }
",
    );
    assert!(issues.is_empty(), "{issues:?}");
}

/// The shape that works, so the rules above are not refusing everything.
#[test]
fn a_continue_until_the_agent_is_told_to_write_is_accepted() {
    let issues = issues_for(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: work
    kind: agent
    output: out.json
    prompt: write
    continue_until: { field: state, equals: done }
    max_turns: 5
    output_schema:
      type: object
      required: [state]
      properties:
        state: { type: string, enum: [in_progress, done] }
",
    );
    assert!(issues.is_empty(), "{issues:?}");
}

/// An attempt that may send no prompt cannot run at all.
#[test]
fn max_turns_of_zero_is_rejected() {
    let issues = issues_for(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: work
    kind: agent
    output: out.md
    prompt: write
    max_turns: 0
",
    );
    assert!(
        issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::MaxTurnsIsZero))
    );
}

#[test]
fn an_unknown_version_is_rejected() {
    let issues =
        issues_for("version: 99\nnodes:\n  - id: a\n    kind: command\n    run: \"true\"\n");
    assert!(
        issues
            .iter()
            .any(|i| matches!(i.kind, ValidationKind::UnknownVersion(99)))
    );
}
