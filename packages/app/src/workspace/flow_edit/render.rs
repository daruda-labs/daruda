//! A value, written the way a flow file writes it.
//!
//! Deliberately not `yaml_serde::to_string`: that emits a whole document with
//! its own indentation, and what an edit needs is one value at a column the file
//! already chose. The quoting rules are the narrow set this app can produce —
//! anything outside them is refused by the caller rather than guessed at.

use yaml_serde::Value;

use super::FlowEditError;
use super::locate::{BlockHeader, ValueKind};

/// Render `value` for the slot [`super::locate::Found`] describes.
///
/// `column` is the entry's own column; a nested container or a block scalar's
/// body goes one `indent` step further right.
pub(super) fn value_at(
    value: &Value,
    kind: ValueKind,
    column: usize,
    indent: usize,
) -> Result<String, FlowEditError> {
    match (kind, value) {
        // A string that was written as a block scalar stays one, and a string
        // that has grown a newline becomes one: a plain scalar cannot hold it.
        (
            ValueKind::BlockScalar {
                header,
                column: content,
            },
            Value::String(s),
        ) => block_scalar(s, header, content, content.saturating_sub(column)),
        (_, Value::String(s)) if s.contains('\n') => {
            block_scalar(s, BlockHeader::Keep, column + indent, indent)
        }
        // A value the file wrote in flow style is replaced in flow style.
        (ValueKind::FlowSequence, _) => flow_sequence(value),
        (_, Value::Mapping(_) | Value::Sequence(_)) => container(value, column + indent, indent),
        _ => scalar(value),
    }
}

/// A whole new entry for a vacancy: `key: value` or `- value`, on its own line
/// at `column`.
pub(super) fn entry(
    key: Option<&str>,
    value: &Value,
    column: usize,
    indent: usize,
) -> Result<String, FlowEditError> {
    let pad = " ".repeat(column);
    match key {
        Some(key) => {
            let rendered = match value {
                Value::Mapping(_) | Value::Sequence(_) => {
                    format!("\n{}", container(value, column + indent, indent)?)
                }
                Value::String(s) if s.contains('\n') => {
                    format!(
                        " {}",
                        block_scalar(s, BlockHeader::Keep, column + indent, indent)?
                    )
                }
                _ => format!(" {}", scalar(value)?),
            };
            Ok(format!("\n{pad}{}:{rendered}", plain_key(key)?))
        }
        None => {
            // A sequence element's own fields sit two columns in from the `- `,
            // which is what makes `- id: x` and its siblings line up.
            let body = match value {
                Value::Mapping(_) | Value::Sequence(_) => container(value, column + 2, indent)?
                    .trim_start()
                    .to_string(),
                _ => scalar(value)?,
            };
            Ok(format!("\n{pad}- {body}"))
        }
    }
}

/// `[a, b]` — a sequence kept in the flow style the file wrote it in.
///
/// Scalars only. A flow sequence holding a mapping is refused rather than
/// flattened: it would have to be re-indented to say the same thing, and a value
/// somebody chose to write on one line is not ours to reformat.
pub(super) fn flow_sequence(value: &Value) -> Result<String, FlowEditError> {
    let Value::Sequence(items) = value else {
        return Err(FlowEditError::FlowStyle(
            "a flow-style list replaced by something that is not a list".into(),
        ));
    };
    let mut out = String::from("[");
    for (ix, item) in items.iter().enumerate() {
        if matches!(item, Value::Sequence(_) | Value::Mapping(_)) {
            return Err(FlowEditError::FlowStyle(
                "a flow-style list holding a list or a mapping".into(),
            ));
        }
        if ix > 0 {
            out.push_str(", ");
        }
        out.push_str(&scalar(item)?);
    }
    out.push(']');
    Ok(out)
}

/// A block mapping or sequence, every line indented to `column`.
fn container(value: &Value, column: usize, indent: usize) -> Result<String, FlowEditError> {
    let pad = " ".repeat(column);
    let mut out = String::new();
    match value {
        Value::Mapping(map) => {
            for (k, v) in map {
                let key = k
                    .as_str()
                    .ok_or_else(|| FlowEditError::Unrepresentable("a non-string key".into()))?;
                if !out.is_empty() {
                    out.push('\n');
                }
                match v {
                    Value::Mapping(_) | Value::Sequence(_) => {
                        out.push_str(&format!("{pad}{}:\n", plain_key(key)?));
                        out.push_str(&container(v, column + indent, indent)?);
                    }
                    Value::String(s) if s.contains('\n') => out.push_str(&format!(
                        "{pad}{}: {}",
                        plain_key(key)?,
                        block_scalar(s, BlockHeader::Keep, column + indent, indent)?
                    )),
                    _ => out.push_str(&format!("{pad}{}: {}", plain_key(key)?, scalar(v)?)),
                }
            }
        }
        Value::Sequence(items) => {
            for item in items {
                if !out.is_empty() {
                    out.push('\n');
                }
                match item {
                    Value::Mapping(_) | Value::Sequence(_) => {
                        let inner = container(item, column + 2, indent)?;
                        out.push_str(&format!("{pad}- {}", inner.trim_start()));
                    }
                    _ => out.push_str(&format!("{pad}- {}", scalar(item)?)),
                }
            }
        }
        _ => return scalar(value),
    }
    Ok(out)
}

/// `column` is where the body goes; `relative` is that column measured from the
/// entry's own, which is the number a block header's indentation indicator takes.
fn block_scalar(
    text: &str,
    header: BlockHeader,
    column: usize,
    relative: usize,
) -> Result<String, FlowEditError> {
    let body = text.trim_end_matches('\n');
    let mut out = String::from(header.as_str());
    // YAML reads a block scalar's indentation off its **first line**, so a first
    // line that itself begins with a space or a tab would be taken as the indent
    // and swallow the lines under it — measured: the file then does not parse at
    // all. An explicit indicator states the indentation instead of leaving it to
    // be inferred, and the content comes back byte for byte.
    if body.starts_with([' ', '\t']) {
        if !(1..=MAX_INDENT_INDICATOR).contains(&relative) {
            return Err(FlowEditError::Unrepresentable(format!(
                "a block scalar starting with whitespace at an indentation of {relative}"
            )));
        }
        out.push_str(&relative.to_string());
    }
    let pad = " ".repeat(column);
    for line in body.split('\n') {
        out.push('\n');
        if line.is_empty() {
            // An empty line carries no indentation; padding it would put
            // trailing whitespace in a file somebody reads.
            continue;
        }
        out.push_str(&pad);
        out.push_str(line);
    }
    Ok(out)
}

/// YAML's indentation indicator is a single digit.
const MAX_INDENT_INDICATOR: usize = 9;

fn scalar(value: &Value) -> Result<String, FlowEditError> {
    Ok(match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => plain_or_quoted(s),
        Value::Sequence(_) | Value::Mapping(_) => {
            return Err(FlowEditError::Unrepresentable(
                "a container where a scalar was expected".into(),
            ));
        }
        Value::Tagged(_) => {
            return Err(FlowEditError::Unrepresentable("a tagged value".into()));
        }
    })
}

/// A key, plain when it can be. A key needing quotes is refused rather than
/// quoted: every key a flow file has is a field name this app spells itself, so
/// one that needs escaping means the value tree is not what we think it is.
fn plain_key(key: &str) -> Result<String, FlowEditError> {
    if key.is_empty() || key.chars().any(|c| !is_plain_key_char(c)) {
        return Err(FlowEditError::Unrepresentable(format!("the key `{key}`")));
    }
    Ok(key.to_string())
}

fn is_plain_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// Quote only where a plain scalar would read as something else. The list is
/// what YAML 1.2 resolves specially plus the indicators that end a plain scalar.
fn plain_or_quoted(s: &str) -> String {
    let needs_quotes = s.is_empty()
        || s.trim() != s
        || s.contains(": ")
        || s.contains(" #")
        || s.ends_with(':')
        || s.starts_with(|c: char| {
            matches!(
                c,
                '-' | '?'
                    | ':'
                    | ','
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '#'
                    | '&'
                    | '*'
                    | '!'
                    | '|'
                    | '>'
                    | '\''
                    | '"'
                    | '%'
                    | '@'
                    | '`'
            )
        })
        || reads_as_another_type(s);
    if !needs_quotes {
        return s.to_string();
    }
    let escaped = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}

/// Would a plain scalar of this text come back as a bool, a number, or null?
fn reads_as_another_type(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~"
    ) || s.parse::<f64>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(yaml: &str) -> Value {
        yaml_serde::from_str(yaml).expect("test fixture parses")
    }

    #[test]
    fn a_plain_string_stays_plain_and_a_confusing_one_is_quoted() {
        assert_eq!(
            scalar(&Value::String("design.md".into())).unwrap(),
            "design.md"
        );
        for confusing in ["true", "3", "null", "- x", "a: b", "", " x"] {
            let out = scalar(&Value::String(confusing.into())).unwrap();
            assert!(
                out.starts_with('"'),
                "{confusing:?} must be quoted, got {out}"
            );
        }
    }

    /// A string that grew a newline cannot stay a plain scalar.
    #[test]
    fn a_multiline_string_becomes_a_block_scalar_indented_to_its_slot() {
        let out = value_at(
            &Value::String("first\nsecond\n".into()),
            ValueKind::Scalar,
            4,
            2,
        )
        .unwrap();
        assert_eq!(out, "|\n      first\n      second");
    }

    /// And one that already was a block scalar keeps the header and the column
    /// the file chose, not the one we would have picked.
    #[test]
    fn a_block_scalar_keeps_the_style_it_was_written_in() {
        let out = value_at(
            &Value::String("only\n".into()),
            ValueKind::BlockScalar {
                header: BlockHeader::Strip,
                column: 8,
            },
            4,
            2,
        )
        .unwrap();
        assert_eq!(out, "|-\n        only");
    }

    /// The header states the indentation when the first line has some of its
    /// own, and the content comes back exactly.
    #[test]
    fn a_body_starting_with_whitespace_says_how_far_it_is_indented() {
        let out = value_at(
            &Value::String("  indented\nplain\n".into()),
            ValueKind::Scalar,
            4,
            2,
        )
        .unwrap();
        assert_eq!(out, "|2\n        indented\n      plain");
        // Read back where it was rendered for: the indicator is the body's
        // indentation *relative to its own entry*, so the entry has to sit at
        // the column it was rendered for (4) or the digit means something else.
        let round_tripped: Value = yaml_serde::from_str(&format!("a:\n  b:\n    p: {out}\n"))
            .expect("what we wrote parses");
        assert_eq!(
            round_tripped
                .get("a")
                .and_then(|a| a.get("b"))
                .and_then(|b| b.get("p"))
                .and_then(Value::as_str),
            Some("  indented\nplain\n"),
            "and says the same thing"
        );
    }

    /// A tab is not indentation to YAML, but it is still refused where an indent
    /// is expected — the same indicator covers it.
    #[test]
    fn a_body_starting_with_a_tab_is_covered_by_the_same_indicator() {
        let out = value_at(&Value::String("\tx\ny\n".into()), ValueKind::Scalar, 0, 2).unwrap();
        let round_tripped: Value =
            yaml_serde::from_str(&format!("p: {out}\n")).expect("what we wrote parses");
        assert_eq!(
            round_tripped.get("p").and_then(Value::as_str),
            Some("\tx\ny\n")
        );
    }

    #[test]
    fn an_empty_line_inside_a_block_scalar_is_not_padded() {
        let out = value_at(&Value::String("a\n\nb\n".into()), ValueKind::Scalar, 0, 2).unwrap();
        assert_eq!(out, "|\n  a\n\n  b");
    }

    #[test]
    fn a_new_field_is_written_on_its_own_line_at_the_column_given() {
        let out = entry(Some("output"), &Value::String("build.md".into()), 4, 2).unwrap();
        assert_eq!(out, "\n    output: build.md");
    }

    #[test]
    fn a_new_sequence_element_carries_its_fields_under_the_dash() {
        let out = entry(None, &v("id: ship\nkind: agent\n"), 2, 2).unwrap();
        assert_eq!(out, "\n  - id: ship\n    kind: agent");
    }

    #[test]
    fn a_nested_mapping_is_written_one_step_in() {
        let out = entry(Some("agent"), &v("id: claude\nmode: bypass\n"), 0, 2).unwrap();
        assert_eq!(out, "\nagent:\n  id: claude\n  mode: bypass");
    }

    #[test]
    fn a_tagged_value_is_refused_rather_than_invented() {
        let err = scalar(&Value::Tagged(Box::new(yaml_serde::value::TaggedValue {
            tag: yaml_serde::value::Tag::new("x"),
            value: Value::Null,
        })))
        .expect_err("refused");
        assert!(matches!(err, FlowEditError::Unrepresentable(_)), "{err:?}");
    }
}
