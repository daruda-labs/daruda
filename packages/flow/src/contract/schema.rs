//! The shape a node's output must have, asked of the file's contents parsed
//! as JSON — and, at load time, whether the declared shape is one this build
//! actually enforces.
//!
//! Both halves live here on purpose: what [`validate`] checks and what
//! [`issues`] refuses are one statement about the enforced subset, and split
//! across two modules they would be two things to keep in step.
//!
//! Pure: no I/O, no globals. Reading the file is `contract::file`'s.

use crate::NodeId;
use crate::error::{ValidationIssue, ValidationKind};
use crate::parse::{SchemaKind, SchemaSubset};
use serde_json::Value;

/// How many problems one breach carries. The list is pasted into a prompt, so
/// past a handful it costs tokens without telling the agent anything new.
const MAX_PROBLEMS: usize = 10;

/// Every way `value` fails `schema`, as path-prefixed lines
/// (`$.a.b[0]: expected string, found number`), capped at [`MAX_PROBLEMS`].
///
/// Deliberately silent about properties the schema does not mention: the
/// schema reaches the agent as prompt text, and extra invented fields are what
/// prompt-delivered schemas get. Refusing them would fail a node over a field
/// nothing reads.
pub(crate) fn validate(value: &Value, schema: &SchemaSubset) -> Result<(), Vec<String>> {
    let mut problems = Vec::new();
    check(value, schema, "$", &mut problems);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

fn check(value: &Value, schema: &SchemaSubset, path: &str, out: &mut Vec<String>) {
    if out.len() >= MAX_PROBLEMS {
        return;
    }
    if !holds(value, schema.kind) {
        out.push(format!(
            "{path}: expected {}, found {}",
            schema.kind.as_str(),
            found(value)
        ));
        // Everything below is a question about a value of the wrong shape, and
        // would report the same mistake once per field.
        return;
    }
    // Only on strings: that is the whole of what this build enforces, and
    // `crate::validate` refuses `enum` on any other type at load.
    if let (SchemaKind::String, Some(allowed)) = (schema.kind, &schema.allowed)
        && !allowed.contains(value)
    {
        out.push(format!(
            "{path}: expected one of {}, found {value}",
            Value::Array(allowed.clone())
        ));
    }
    match schema.kind {
        SchemaKind::Object => {
            let Some(map) = value.as_object() else {
                return;
            };
            for name in &schema.required {
                if out.len() >= MAX_PROBLEMS {
                    return;
                }
                if !map.contains_key(name) {
                    out.push(format!("{path}.{name}: required, and absent"));
                }
            }
            for (name, sub) in &schema.properties {
                if let Some(held) = map.get(name) {
                    check(held, sub, &format!("{path}.{name}"), out);
                }
            }
        }
        SchemaKind::Array => {
            let Some((items, held)) = schema.items.as_deref().zip(value.as_array()) else {
                return;
            };
            for (index, element) in held.iter().enumerate() {
                check(element, items, &format!("{path}[{index}]"), out);
            }
        }
        _ => {}
    }
}

/// Whether one value is of the declared type.
///
/// `integer` accepts an integer-valued float: JSON Schema calls `3.0` an
/// integer, and `serde_json::Number` has no such case of its own — so a model
/// writing `{"n": 3.0}` would otherwise cost a whole node run.
fn holds(value: &Value, kind: SchemaKind) -> bool {
    match kind {
        SchemaKind::Object => value.is_object(),
        SchemaKind::Array => value.is_array(),
        SchemaKind::String => value.is_string(),
        SchemaKind::Number => value.is_number(),
        SchemaKind::Integer => {
            value.is_i64() || value.is_u64() || value.as_f64().is_some_and(|n| n.fract() == 0.0)
        }
        SchemaKind::Boolean => value.is_boolean(),
    }
}

/// What a value is, in the same vocabulary the schema is written in.
fn found(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Every way a declared schema asks for something this build does not enforce.
///
/// Refused at load, node-named, rather than at run time: a schema whose
/// keywords are silently dropped promises a reader a check that never happens,
/// and discovering that costs an agent turn.
pub(crate) fn issues(schema: &SchemaSubset, node: &NodeId) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    collect(schema, node, &mut issues);
    issues
}

fn collect(schema: &SchemaSubset, node: &NodeId, out: &mut Vec<ValidationIssue>) {
    for keyword in schema.rest.keys() {
        out.push(issue(
            node,
            ValidationKind::UnsupportedSchemaKeyword {
                keyword: keyword.clone(),
            },
            format!("`{keyword}` is not one of the keywords this build enforces"),
        ));
    }
    // Every keyword this build enforces is enforced under exactly one kind,
    // and `check` reaches it only there. Written under any other kind it is
    // read, accepted, and then never asked — which is the one thing this
    // whole function exists to stop a flow author from believing.
    let misplaced = [
        (schema.allowed.is_some(), "enum", SchemaKind::String),
        (!schema.required.is_empty(), "required", SchemaKind::Object),
        (
            !schema.properties.is_empty(),
            "properties",
            SchemaKind::Object,
        ),
        (schema.items.is_some(), "items", SchemaKind::Array),
    ];
    for (written, keyword, enforced_under) in misplaced {
        if written && schema.kind != enforced_under {
            out.push(issue(
                node,
                ValidationKind::SchemaKeywordOnWrongKind {
                    keyword,
                    kind: schema.kind.as_str(),
                },
                format!(
                    "`{keyword}` is enforced on a `{}`, not on a `{}`",
                    enforced_under.as_str(),
                    schema.kind.as_str()
                ),
            ));
        }
    }
    for sub in schema.properties.values() {
        collect(sub, node, out);
    }
    if let Some(items) = schema.items.as_deref() {
        collect(items, node, out);
    }
}

fn issue(node: &NodeId, kind: ValidationKind, message: String) -> ValidationIssue {
    ValidationIssue {
        node: Some(node.clone()),
        kind,
        message,
    }
}

#[cfg(test)]
mod tests;
