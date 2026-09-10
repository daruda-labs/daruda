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
        /// The shape the output's contents must have, checked as JSON.
        /// Absent is what every node did before this existed: the file only
        /// has to be there.
        ///
        /// Boxed because this is the largest thing an agent node can carry and
        /// almost no node declares one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_schema: Option<Box<SchemaSubset>>,
        /// What the output has to say before this node is finished.
        ///
        /// Absent is the rule that held before this existed: a well-formed
        /// output is a finished node. Present means an output that parses and
        /// matches its schema can still leave the node unfinished, which is
        /// how a node keeps going without a person telling it to.
        ///
        /// Boxed for the reason `output_schema` is: almost no node declares
        /// one, and unboxed it makes the agent variant much larger than the
        /// command variant for every node that does not.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        continue_until: Option<Box<DoneWhenFile>>,
        /// How many prompts one attempt may send, the first included.
        ///
        /// Absent is 2 — the first prompt plus the one correction turn that
        /// existed before this field. A flow that wants a node to keep going
        /// raises it; `1` turns both off.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_turns: Option<u32>,
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

/// The slice of JSON Schema an `output_schema` may spell.
///
/// Unknown *properties in the data* are allowed on purpose — there is no
/// `additionalProperties`. The schema reaches the agent as prompt text, which
/// gets extra invented fields where a provider-enforced structured response
/// does not, and refusing what cannot be enforced only buys another node run.
///
/// Keywords this build does not enforce land in [`SchemaSubset::rest`] rather
/// than failing the parse, so a flow naming one stays readable and editable.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SchemaSubset {
    #[serde(rename = "type")]
    pub kind: SchemaKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, SchemaSubset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<SchemaSubset>>,
    /// `Value`, not `String`: `enum: [1, 2]` is legal JSON Schema, and a
    /// `Vec<String>` here would make it a parse error — which takes the whole
    /// file's graph and inspector away instead of naming one node.
    #[serde(default, rename = "enum", skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Vec<serde_json::Value>>,
    /// Keywords this build does not enforce. `crate::validate` refuses them by
    /// name, with the node attached.
    #[serde(flatten)]
    pub rest: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaKind {
    Object,
    Array,
    String,
    Number,
    Integer,
    Boolean,
}

impl SchemaKind {
    /// The word a refusal uses, which is the word the schema was written in.
    /// The only rendering of a kind there is — a `Display` beside it would be a
    /// second one to keep in step.
    pub fn as_str(self) -> &'static str {
        match self {
            SchemaKind::Object => "object",
            SchemaKind::Array => "array",
            SchemaKind::String => "string",
            SchemaKind::Number => "number",
            SchemaKind::Integer => "integer",
            SchemaKind::Boolean => "boolean",
        }
    }
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
/// One field of the output, and the value that means finished.
///
/// A field-and-value pair rather than an expression: an expression would need
/// its own validation, its own display in the inspector and its own way to be
/// debugged when it silently never matches. `state: done` is what the case
/// this exists for actually needs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct DoneWhenFile {
    pub field: String,
    pub equals: serde_json::Value,
}

const NODE_KEYS: &[&str] = &["id", "deps", "timeout", "kind", "cwd"];
/// Keys an agent node adds. `prompt` / `prompt_file` are one choice, and
/// naming both is its own error below rather than an unknown key.
const AGENT_KEYS: &[&str] = &[
    "agent",
    "prompt",
    "prompt_file",
    "output",
    "output_schema",
    "continue_until",
    "max_turns",
    "on_fail",
];
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
        node: Some(NodeId::from(node)),
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
        node: Some(NodeId::from(node)),
        kind: ValidationKind::ConflictingField {
            field: field.to_string(),
            wins,
        },
        message: format!("`{node}` names both `{field}` and `{wins}`"),
    }
}

#[cfg(test)]
mod tests;
