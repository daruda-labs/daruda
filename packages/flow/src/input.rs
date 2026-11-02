//! A flow's inputs: the parameters it declares, the values a run is given,
//! and the checks between the two.
//!
//! Values are substituted as text into the surfaces templates already reach
//! (`{{input.<name>}}`), through `crate::template::render`, so a command
//! still gets every value shell-quoted. The run records the values it used,
//! so a resume renders exactly what the first process did.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{ValidationIssue, ValidationKind};

/// What a value of an input may be.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputType {
    #[default]
    String,
    Number,
    Boolean,
    /// One of the declared `choices`.
    Choice,
}

/// One declared input, as the file writes it and as the engine uses it:
/// nothing in it resolves against `defaults` or a profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputDecl {
    /// The key under `inputs:`; not a field of the YAML entry itself.
    #[serde(skip)]
    pub name: String,
    #[serde(default, rename = "type")]
    pub kind: InputType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<InputValue>,
    /// The values a `choice` input allows, in the order they are offered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    /// A `string` input whose value is expected to span lines; a host offers
    /// a larger box for it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multiline: bool,
}

/// A value given for an input. Untagged so YAML and JSON write it plainly;
/// a number keeps the spelling it was given, so `1234` renders as `1234`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InputValue {
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
}

/// The values one run uses, by input name.
pub type InputValues = BTreeMap<String, InputValue>;

impl InputValue {
    /// The text a template substitutes: `true`/`false`, a number as given,
    /// a string as is. Never localized — a prompt and a command alike must
    /// read the same value the run recorded.
    pub fn as_text(&self) -> String {
        match self {
            Self::Bool(value) => value.to_string(),
            Self::Number(value) => value.to_string(),
            Self::Text(value) => value.clone(),
        }
    }

    /// This value as `kind` takes it, or `None` when it cannot be one. A
    /// `string` input accepts any scalar as its text; a number or boolean
    /// written as a string is read, since a form field and a phone message
    /// both deliver text.
    pub fn coerce(&self, decl: &InputDecl) -> Option<InputValue> {
        match decl.kind {
            InputType::String => Some(Self::Text(self.as_text())),
            InputType::Number => match self {
                Self::Number(_) => Some(self.clone()),
                Self::Text(text) => parse_number(text).map(Self::Number),
                Self::Bool(_) => None,
            },
            InputType::Boolean => match self {
                Self::Bool(_) => Some(self.clone()),
                Self::Text(text) => match text.trim() {
                    "true" => Some(Self::Bool(true)),
                    "false" => Some(Self::Bool(false)),
                    _ => None,
                },
                Self::Number(_) => None,
            },
            InputType::Choice => {
                let text = self.as_text();
                decl.choices.contains(&text).then_some(Self::Text(text))
            }
        }
    }
}

/// `text` as a number: a JSON number as JSON reads it, else the value a
/// person plainly meant (`+5`, `007`). An integer renders as typed.
fn parse_number(text: &str) -> Option<serde_json::Number> {
    let text = text.trim();
    if let Ok(number) = text.parse::<serde_json::Number>() {
        return Some(number);
    }
    let unsigned = text.strip_prefix('+').unwrap_or(text);
    if let Ok(whole) = unsigned.parse::<i64>() {
        return Some(whole.into());
    }
    unsigned
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .and_then(serde_json::Number::from_f64)
}

/// Problems in the declarations themselves, found at load: a `choice`
/// with nothing to choose, or a default its own type refuses.
pub(crate) fn check_decls(decls: &[InputDecl]) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    for decl in decls {
        if !is_input_name(&decl.name) {
            issues.push(issue(
                ValidationKind::InvalidInputName {
                    name: decl.name.clone(),
                },
                format!(
                    "input `{}` cannot be named in a template: use letters, digits, `_` and `-`",
                    decl.name
                ),
            ));
        }
        if decl.kind == InputType::Choice && decl.choices.is_empty() {
            issues.push(issue(
                ValidationKind::InputChoicesMissing {
                    name: decl.name.clone(),
                },
                format!("input `{}` is a choice with no `choices`", decl.name),
            ));
        }
        if let Some(default) = &decl.default
            && default.coerce(decl).is_none()
        {
            issues.push(issue(
                ValidationKind::InputDefaultInvalid {
                    name: decl.name.clone(),
                },
                format!("input `{}` has a default its type refuses", decl.name),
            ));
        }
    }
    issues
}

/// The values a run uses: each given value checked against its declaration,
/// each missing one filled from its default, and an optional one with no
/// default left empty. Every problem is reported, not the first.
///
/// What `crate::request::validate_request` asks of a run's values; a host
/// asks it first when it has its own answer for a refusal.
pub fn resolve(
    decls: &[InputDecl],
    given: &InputValues,
) -> Result<InputValues, Vec<ValidationIssue>> {
    let mut issues = Vec::new();
    for name in given.keys() {
        if !decls.iter().any(|decl| &decl.name == name) {
            issues.push(issue(
                ValidationKind::InputNotDeclared { name: name.clone() },
                format!("the flow declares no input `{name}`"),
            ));
        }
    }
    let mut values = InputValues::new();
    for decl in decls {
        // A blank field is no number, boolean or choice, so for those it is
        // no value and the default applies. A string keeps it: emptying a
        // pre-filled field is how a person runs without the default.
        let given_value = given.get(&decl.name).filter(|value| {
            decl.kind == InputType::String
                || !matches!(value, InputValue::Text(text) if text.trim().is_empty())
        });
        let value = match given_value {
            Some(value) => match value.coerce(decl) {
                Some(value) => Some(value),
                None => {
                    issues.push(issue(
                        ValidationKind::InputValueInvalid {
                            name: decl.name.clone(),
                        },
                        format!(
                            "`{}` is not a valid value for input `{}`",
                            value.as_text(),
                            decl.name
                        ),
                    ));
                    continue;
                }
            },
            None => decl.default.as_ref().and_then(|d| d.coerce(decl)),
        };
        // A required input whose value is still blank has not been answered.
        let blank = value.as_ref().is_none_or(|v| v.as_text().trim().is_empty());
        if decl.required && blank {
            issues.push(issue(
                ValidationKind::InputMissing {
                    name: decl.name.clone(),
                },
                format!("input `{}` is required", decl.name),
            ));
            continue;
        }
        values.insert(
            decl.name.clone(),
            value.unwrap_or_else(|| InputValue::Text(String::new())),
        );
    }
    if issues.is_empty() {
        Ok(values)
    } else {
        Err(issues)
    }
}

/// Whether `name` can be written as `{{input.<name>}}`.
fn is_input_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Every `{{input.<name>}}` in `text` that `decls` does not declare.
pub(crate) fn undeclared_refs<'t>(text: &'t str, decls: &[InputDecl]) -> Vec<&'t str> {
    crate::template::input_refs(text)
        .filter(|name| !decls.iter().any(|decl| decl.name == *name))
        .collect()
}

/// One issue per `{{input.<name>}}` in `texts` that `decls` does not
/// declare, pinned to the node whose text it is.
pub(crate) fn check_refs(
    node: &crate::NodeId,
    texts: &[&str],
    decls: &[InputDecl],
    issues: &mut Vec<ValidationIssue>,
) {
    for text in texts {
        for name in undeclared_refs(text, decls) {
            issues.push(ValidationIssue {
                node: Some(node.clone()),
                kind: ValidationKind::UndeclaredInput {
                    name: name.to_owned(),
                },
                message: format!("`{{{{input.{name}}}}}` names an input the flow does not declare"),
            });
        }
    }
}

fn issue(kind: ValidationKind, message: String) -> ValidationIssue {
    ValidationIssue {
        node: None,
        kind,
        message,
    }
}

/// `inputs:` is a YAML map, read and written in the order the file gives it:
/// a host offers the inputs in that order, and a sorted map would reorder a
/// form its author laid out.
pub(crate) mod ordered {
    use std::fmt;

    use serde::de::{MapAccess, Visitor};
    use serde::ser::SerializeMap;
    use serde::{Deserializer, Serializer};

    use super::InputDecl;

    pub(crate) fn serialize<S: Serializer>(decls: &[InputDecl], ser: S) -> Result<S::Ok, S::Error> {
        let mut map = ser.serialize_map(Some(decls.len()))?;
        for decl in decls {
            map.serialize_entry(&decl.name, decl)?;
        }
        map.end()
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        de: D,
    ) -> Result<Vec<InputDecl>, D::Error> {
        struct InOrder;
        impl<'de> Visitor<'de> for InOrder {
            type Value = Vec<InputDecl>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a map of input names to declarations")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut decls: Vec<InputDecl> = Vec::new();
                while let Some((name, mut decl)) = map.next_entry::<String, InputDecl>()? {
                    if decls.iter().any(|d| d.name == name) {
                        return Err(serde::de::Error::custom(format!(
                            "input `{name}` is declared twice"
                        )));
                    }
                    decl.name = name;
                    decls.push(decl);
                }
                Ok(decls)
            }
        }
        de.deserialize_map(InOrder)
    }
}

#[cfg(test)]
mod tests;
