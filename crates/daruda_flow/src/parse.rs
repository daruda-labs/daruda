//! The YAML a flow file literally contains. Every override is an `Option`
//! so a node can name one axis without restating the rest; merging them
//! against `defaults` is `crate::resolve`'s job, not this module's.
//!
//! `Serialize` belongs here rather than on the resolved model: `kind:`,
//! `prompt:` and `on_fail:` are spelled by these serde attributes alone, so
//! `run.yaml` is written by mapping back to this shape.

use crate::NodeId;
use crate::error::{FlowError, ValidationIssue, ValidationKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// One flow file, exactly as written.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FlowFile {
    /// Schema version. Required so a future change to the execution rules
    /// can coexist with files written against today's.
    pub version: u32,
    #[serde(default)]
    pub defaults: Defaults,
    /// Named layers over `defaults`, chosen at submission. A map rather
    /// than a list so a profile is named where it is used, and ordered so
    /// the host offers them the same way twice.
    ///
    /// Lives in the flow file rather than in daruda's config for the same
    /// reason `agent.mode` has no config fallback: the file is committed
    /// and shared, and a run whose settings came from somewhere else is
    /// one nobody reading the file can predict.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, Defaults>,
    pub nodes: Vec<NodeFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    #[serde(
        default,
        with = "humantime_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout: Option<Duration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentOverride>,
    /// How many nodes may run at once. Absent is one — the flow says when
    /// it wants more, because the engine cannot know whether two of its
    /// nodes are safe to overlap and guessing wrong corrupts a working
    /// tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct NodeFile {
    pub id: NodeId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<NodeId>,
    #[serde(
        default,
        with = "humantime_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout: Option<Duration>,
    /// Where this node runs, relative to the run's own working directory.
    /// Absent is the run's directory itself, which is what every node did
    /// before this existed.
    ///
    /// Relative and inside, enforced by `crate::validate`. That rule is
    /// what lets the run keep **one** lock: it already holds the directory
    /// every node works in. A lock per subdirectory would be worse than
    /// none — a run holding the root and a run holding `sub/` would not
    /// exclude each other, and both would write to `sub/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// `kind` and the fields it selects sit at the same level as `id` in
    /// the file, so the tag is flattened into this struct rather than
    /// nested under a key of its own.
    #[serde(flatten)]
    pub kind: NodeKindFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeKindFile {
    Agent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<AgentOverride>,
        #[serde(flatten)]
        prompt: PromptSource,
        output: PathBuf,
        #[serde(default, with = "yaml_serde::with::singleton_map")]
        on_fail: AgentFailFile,
    },
    Command {
        run: String,
        #[serde(default, with = "yaml_serde::with::singleton_map")]
        on_fail: GateFailFile,
    },
}

/// Every field optional: a node names only the axes it overrides, and
/// `crate::resolve` fills the rest from `defaults`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionPolicyFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionPolicyFile {
    Deny,
    AllowOnce,
    Ask,
}

/// A node's prompt: inline prose or a sibling file, under keys `prompt:` /
/// `prompt_file:`. If a node names both, `prompt` silently wins and the
/// flattened shape leaves no trace of the other — not enforceable here.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptSource {
    Prompt(String),
    PromptFile(PathBuf),
}

/// A retry's hint. A separate enum from [`PromptSource`] on purpose:
/// flattening erases the field name, so reusing `PromptSource` here would
/// make the retry block's key `prompt:` instead of `hint:`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HintSource {
    Hint(String),
    HintFile(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentFailFile {
    #[default]
    Halt,
    Retry {
        #[serde(flatten)]
        hint: HintSource,
        max_attempts: u32,
        #[serde(
            default,
            with = "humantime_serde",
            skip_serializing_if = "Option::is_none"
        )]
        wait: Option<Duration>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateFailFile {
    #[default]
    Halt,
    Repair {
        fix: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rerun: Vec<NodeId>,
        max_attempts: u32,
        #[serde(
            default,
            with = "humantime_serde",
            skip_serializing_if = "Option::is_none"
        )]
        wait: Option<Duration>,
    },
}

/// Parse one flow file's text. Shape errors only — cross-node rules are
/// `crate::validate`'s job.
pub fn parse_flow_file(text: &str) -> Result<FlowFile, FlowError> {
    yaml_serde::from_str(text).map_err(|e| FlowError::Parse(e.to_string()))
}

/// Keys every node may carry, whatever its kind.
const NODE_KEYS: &[&str] = &["id", "deps", "timeout", "kind", "cwd"];
/// Keys an agent node adds. `prompt` / `prompt_file` are one choice, and
/// naming both is its own error below rather than an unknown key.
const AGENT_KEYS: &[&str] = &["agent", "prompt", "prompt_file", "output", "on_fail"];
const COMMAND_KEYS: &[&str] = &["run", "on_fail"];
const RETRY_KEYS: &[&str] = &["hint", "hint_file", "max_attempts", "wait"];
const REPAIR_KEYS: &[&str] = &["fix", "rerun", "max_attempts", "wait"];

/// Every key a node carries that its kind has no field for, and every
/// either-or pair it names both halves of.
///
/// `deny_unknown_fields` guards the structs that do not flatten, but serde
/// refuses it on any struct that does — and `NodeFile` flattens both its
/// kind and its prompt source. So these keys are checked against the
/// schema by hand, from the same text `parse_flow_file` read.
///
/// Reported rather than ignored because the fields most worth mistyping —
/// `deps`, `timeout`, `on_fail` — decide execution order and failure
/// handling. `dep:` for `deps:` leaves the node with no dependency at all,
/// and the DAG then runs in an order the file does not describe.
///
/// Collected, not short-circuited: an author with three typos should see
/// three, the way every other check in this crate reports.
pub(crate) fn schema_issues(text: &str) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    collect_schema_issues(text, &mut issues);
    issues
}

fn collect_schema_issues(text: &str, issues: &mut Vec<ValidationIssue>) -> Option<()> {
    let value: yaml_serde::Value = yaml_serde::from_str(text).ok()?;
    let nodes = value.get("nodes")?.as_sequence()?;
    for node in nodes {
        // Nothing below may use `?`: a node without `on_fail`, or one whose
        // shape serde already accepted differently, must skip to the next
        // node rather than end the scan — which is how `dep:` slipped
        // through the first cut of this check.
        let Some(map) = node.as_mapping() else {
            continue;
        };
        let id = map
            .get("id")
            .and_then(yaml_serde::Value::as_str)
            .unwrap_or("?");
        let kind = map.get("kind").and_then(yaml_serde::Value::as_str);
        let allowed: Vec<&str> = NODE_KEYS
            .iter()
            .copied()
            .chain(match kind {
                Some("agent") => AGENT_KEYS.iter().copied(),
                _ => COMMAND_KEYS.iter().copied(),
            })
            .collect();
        for key in map.keys().filter_map(yaml_serde::Value::as_str) {
            if !allowed.contains(&key) {
                issues.push(unknown_field(id, key));
            }
        }
        if map.contains_key("prompt") && map.contains_key("prompt_file") {
            issues.push(conflicting_field(id, "prompt_file", "prompt"));
        }
        if let Some(on_fail) = map.get("on_fail") {
            policy_issues(on_fail, id, issues);
        }
    }
    Some(())
}

/// One `on_fail` block's keys. The policy is a single-entry map — `retry:`
/// or `repair:` — so its own name selects which field set applies. A bare
/// `halt` is not a mapping and has nothing to check.
fn policy_issues(
    on_fail: &yaml_serde::Value,
    id: &str,
    issues: &mut Vec<ValidationIssue>,
) -> Option<()> {
    let (name, body) = on_fail.as_mapping()?.iter().next()?;
    let allowed = match name.as_str()? {
        "retry" => RETRY_KEYS,
        "repair" => REPAIR_KEYS,
        _ => return None,
    };
    let body = body.as_mapping()?;
    for key in body.keys().filter_map(yaml_serde::Value::as_str) {
        if !allowed.contains(&key) {
            issues.push(unknown_field(id, key));
        }
    }
    if body.contains_key("hint") && body.contains_key("hint_file") {
        issues.push(conflicting_field(id, "hint_file", "hint"));
    }
    Some(())
}

fn unknown_field(node: &str, field: &str) -> ValidationIssue {
    ValidationIssue {
        node: Some(node.to_string()),
        kind: ValidationKind::UnknownField {
            field: field.to_string(),
        },
        message: format!("`{node}` has no field `{field}`"),
    }
}

/// Two keys that are one choice. Whichever serde picks, the other is
/// dropped without trace — and it is usually the one being edited.
fn conflicting_field(node: &str, field: &str, wins: &'static str) -> ValidationIssue {
    ValidationIssue {
        node: Some(node.to_string()),
        kind: ValidationKind::ConflictingField {
            field: field.to_string(),
            wins,
        },
        message: format!("`{node}` names both `{field}` and `{wins}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "\
version: 1
defaults:
  timeout: 10m
nodes:
  - id: test
    kind: command
    run: cargo test
";

    #[test]
    fn parses_a_minimal_command_only_flow() {
        let file = parse_flow_file(MINIMAL).expect("minimal flow parses");
        assert_eq!(file.version, 1);
        assert_eq!(
            file.defaults.timeout,
            Some(std::time::Duration::from_secs(600))
        );
        assert_eq!(file.nodes.len(), 1);
        assert_eq!(file.nodes[0].id, "test");
        assert!(file.nodes[0].deps.is_empty());
        match &file.nodes[0].kind {
            NodeKindFile::Command { run, on_fail } => {
                assert_eq!(run, "cargo test");
                assert!(matches!(on_fail, GateFailFile::Halt));
            }
            other => panic!("expected a command node, got {other:?}"),
        }
    }

    const FULL: &str = "\
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
    prompt: |
      Read DESIGN.md and write the design to {{output}}.
    on_fail:
      retry:
        max_attempts: 2
        hint: |
          The previous attempt failed: {{failure}}
  - id: implement
    kind: agent
    deps: [design]
    prompt_file: ./prompts/implement.md
    output: implement.md
  - id: test
    kind: command
    deps: [implement]
    run: cargo test
    on_fail:
      repair:
        fix: \"{{failure}}. Read {{attempts}} and fix the cause.\"
        max_attempts: 2
";

    #[test]
    fn parses_defaults_partial_override_both_prompt_sources_and_both_fail_shapes() {
        let file = parse_flow_file(FULL).expect("flow parses");

        let defaults_agent = file
            .defaults
            .agent
            .as_ref()
            .expect("defaults name an agent");
        assert_eq!(defaults_agent.id.as_deref(), Some("claude"));
        assert_eq!(defaults_agent.permission, Some(PermissionPolicyFile::Deny));

        match &file.nodes[0].kind {
            NodeKindFile::Agent {
                agent,
                prompt,
                output,
                on_fail,
            } => {
                let agent = agent.as_ref().expect("node overrides the agent axis");
                assert_eq!(agent.effort.as_deref(), Some("high"));
                assert_eq!(
                    agent.id, None,
                    "an unnamed axis stays None for resolve to fill"
                );
                assert!(matches!(prompt, PromptSource::Prompt(t) if t.contains("{{output}}")));
                assert_eq!(output, &PathBuf::from("design.md"));
                // The retry's hint key is `hint:`, not `prompt:` — that is
                // what `HintSource` exists for.
                match on_fail {
                    AgentFailFile::Retry {
                        hint,
                        max_attempts,
                        wait,
                    } => {
                        assert!(matches!(hint, HintSource::Hint(t) if t.contains("{{failure}}")));
                        assert_eq!(*max_attempts, 2);
                        assert_eq!(*wait, None);
                    }
                    AgentFailFile::Halt => panic!("expected retry"),
                }
            }
            other => panic!("expected an agent node, got {other:?}"),
        }

        match &file.nodes[1].kind {
            NodeKindFile::Agent { prompt, .. } => assert_eq!(
                prompt,
                &PromptSource::PromptFile(PathBuf::from("./prompts/implement.md"))
            ),
            other => panic!("expected an agent node, got {other:?}"),
        }

        match &file.nodes[2].kind {
            NodeKindFile::Command { on_fail, .. } => match on_fail {
                GateFailFile::Repair {
                    fix,
                    rerun,
                    max_attempts,
                    wait,
                } => {
                    assert!(fix.contains("{{attempts}}"));
                    assert!(rerun.is_empty());
                    assert_eq!(*max_attempts, 2);
                    assert_eq!(*wait, None);
                }
                GateFailFile::Halt => panic!("expected repair"),
            },
            other => panic!("expected a command node, got {other:?}"),
        }
    }

    /// Issues by kind. `FlowError::Validate`'s `Display` deliberately
    /// carries only a count — the per-issue wording is the host's — so a
    /// test asserts on what a consumer matches, not on a string.
    fn kinds_for(text: &str) -> Vec<ValidationKind> {
        match crate::load(text, None) {
            Err(FlowError::Validate(issues)) => issues.into_iter().map(|i| i.kind).collect(),
            Err(other) => panic!("expected validation issues, got {other}"),
            Ok(_) => panic!("expected the flow to be refused"),
        }
    }

    /// The field most worth mistyping. `dep:` leaves the node with no
    /// dependency at all, so the DAG runs in an order the file does not
    /// describe — and nothing downstream can tell, because an empty `deps`
    /// is perfectly legal.
    #[test]
    fn a_mistyped_ordering_field_is_refused_rather_than_defaulted() {
        let kinds = kinds_for(
            "\
version: 1
nodes:
  - id: a
    kind: command
    run: \"true\"
  - id: b
    kind: command
    dep: [a]
    run: \"true\"
",
        );
        assert_eq!(
            kinds,
            vec![ValidationKind::UnknownField {
                field: "dep".to_string()
            }]
        );
    }

    /// Naming both halves of an either-or pair: serde picks one and drops
    /// the other without trace, and the dropped one is usually the one
    /// being edited. Checked at both levels it occurs.
    #[test]
    fn naming_both_halves_of_a_prompt_or_hint_is_refused() {
        assert_eq!(
            kinds_for(
                "\
version: 1
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: inline
    prompt_file: ./a.md
"
            ),
            vec![ValidationKind::ConflictingField {
                field: "prompt_file".to_string(),
                wins: "prompt",
            }]
        );

        assert_eq!(
            kinds_for(
                "\
version: 1
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: w
    on_fail:
      retry:
        hint: from {{failure}}
        hint_file: ./h.md
        max_attempts: 2
"
            ),
            vec![ValidationKind::ConflictingField {
                field: "hint_file".to_string(),
                wins: "hint",
            }]
        );
    }

    /// Collected, not short-circuited — the same promise every other check
    /// in this crate makes. Three typos should cost one round trip, not
    /// three. A node with no `on_fail` must not end the scan either: the
    /// first cut used `?` on that key and let every later node through.
    ///
    /// All three are *optional* fields, which is the set this check has to
    /// cover: mistype a required one and serde rejects the file first,
    /// with a line and column this check cannot give.
    #[test]
    fn every_mistyped_field_is_reported_in_one_pass() {
        let kinds = kinds_for(
            "\
version: 1
nodes:
  - id: a
    kind: command
    run: \"true\"
  - id: b
    kind: command
    run: \"true\"
    timeoutt: 1s
    dep: [a]
    on_fail:
      repair:
        fix: fix from {{failure}}
        max_attempts: 2
        waitt: 1s
",
        );
        assert_eq!(kinds.len(), 3, "{kinds:?}");
        assert!(
            kinds
                .iter()
                .all(|k| matches!(k, ValidationKind::UnknownField { .. }))
        );
    }

    #[test]
    fn a_missing_version_is_a_parse_error_not_a_default() {
        let err = parse_flow_file("nodes: []").expect_err("version is required");
        assert!(
            matches!(err, FlowError::Parse(msg) if msg.contains("version")),
            "the error should name the missing field"
        );
    }

    /// A plain YAML scalar may not contain `": "`, which a `grep` for
    /// `VERDICT: PASS` does. Quoting is the flow author's job, and the
    /// parser must say so clearly rather than silently mis-read the node.
    /// A typo in an override key must not silently disarm it — `moed`
    /// parsing as `Ok` with `mode: None` would be indistinguishable from an
    /// intentional omission.
    #[test]
    fn an_unknown_field_in_an_agent_override_is_a_parse_error() {
        let err = crate::load(
            "\
version: 1
nodes:
  - id: a
    kind: agent
    agent: { moed: bypassPermissions }
    output: a.md
    prompt: write
",
            None,
        )
        .expect_err("moed is not a known AgentOverride field");
        assert!(matches!(err, FlowError::Parse(_)));
    }

    /// `on_fail: halt` is a bare string, unlike `retry`/`repair`'s mapping
    /// shape — deliberately asymmetric, so it needs its own coverage.
    #[test]
    fn on_fail_halt_parses_on_both_node_kinds() {
        let file = parse_flow_file(
            "\
version: 1
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
    on_fail: halt
  - id: b
    kind: command
    run: \"true\"
    on_fail: halt
",
        )
        .expect("halt parses on both kinds");

        match &file.nodes[0].kind {
            NodeKindFile::Agent { on_fail, .. } => {
                assert!(matches!(on_fail, AgentFailFile::Halt));
            }
            other => panic!("expected an agent node, got {other:?}"),
        }
        match &file.nodes[1].kind {
            NodeKindFile::Command { on_fail, .. } => {
                assert!(matches!(on_fail, GateFailFile::Halt));
            }
            other => panic!("expected a command node, got {other:?}"),
        }
    }

    #[test]
    fn an_unquoted_colon_in_a_command_is_a_parse_error() {
        let err = crate::load(
            "\
version: 1
nodes:
  - id: gate
    kind: command
    run: grep -q '^VERDICT: PASS' out.md
",
            None,
        )
        .expect_err("a plain scalar cannot hold \": \"");
        assert!(matches!(err, FlowError::Parse(_)));
    }
}
