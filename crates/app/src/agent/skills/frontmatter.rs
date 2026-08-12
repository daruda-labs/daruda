//! YAML frontmatter parsing + serialization for `SKILL.md`.
//!
//! Format: a leading `---\n…\n---\n` block followed by markdown body.
//! When the leading delimiter is missing the whole file is treated as
//! body, and the parsed frontmatter is `Default`.
//!
//! The parser is **lossless**: any key not in [`super::KNOWN_KEYS`] is
//! kept in [`SkillFrontmatter::extra`] as a `yaml_serde::Value` so a
//! later [`serialize_frontmatter`] reproduces the same KV set.
//! Unknown keys can carry nested objects (e.g. `hooks:`) which daruda
//! does not interpret but must round-trip when the user edits other
//! fields.

use std::collections::BTreeMap;

use yaml_serde::Value;

use super::{DEFAULT_USER_INVOCABLE, KNOWN_KEYS};

/// Strongly-typed view of the keys daruda understands.
///
/// Every field is either `Option<T>` (truly absent) or carries a
/// pragmatic default (the two booleans). The modal can therefore tell
/// "user removed the key" from "user left the field empty" — the
/// former takes the field back to `None` so re-serialisation drops the
/// key entirely.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkillFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub when_to_use: Option<String>,
    pub argument_hint: Option<String>,
    pub arguments: Vec<String>,
    pub allowed_tools: Option<String>,
    pub paths: Option<String>,
    pub context: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub shell: Option<String>,
    pub disable_model_invocation: bool,
    pub user_invocable: bool,
    /// Keys daruda does not interpret. Preserved in source order via
    /// `BTreeMap` (alphabetical) — good enough since the typed fields
    /// always serialise first in a fixed order.
    pub extra: BTreeMap<String, Value>,
}

impl SkillFrontmatter {
    /// Empty frontmatter with spec-compliant defaults — used as the
    /// baseline for the Create modal.
    pub fn empty() -> Self {
        Self {
            user_invocable: DEFAULT_USER_INVOCABLE,
            ..Self::default()
        }
    }
}

/// Split a `SKILL.md` body into its frontmatter source and the body
/// after the closing `---`. Returns `(None, full_text)` when no
/// leading frontmatter delimiter is present.
///
/// The recogniser is intentionally narrow: only `---\n…\n---\n` (the
/// shape Claude Code emits). A leading BOM or trailing whitespace on
/// the closing delimiter line is tolerated.
pub fn split_frontmatter(source: &str) -> (Option<&str>, &str) {
    let trimmed = source.strip_prefix('\u{feff}').unwrap_or(source);
    let Some(rest) = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"))
    else {
        return (None, source);
    };
    // Find the next `---` line (terminated by `\n` or end-of-input).
    let mut idx = 0usize;
    while idx < rest.len() {
        let line_end = rest[idx..]
            .find('\n')
            .map(|n| idx + n)
            .unwrap_or(rest.len());
        let line = rest[idx..line_end].trim_end_matches('\r').trim_end();
        if line == "---" {
            let yaml = &rest[..idx];
            // Skip past the closing delimiter line itself.
            let body_start = if line_end < rest.len() {
                line_end + 1
            } else {
                rest.len()
            };
            return (Some(yaml), &rest[body_start..]);
        }
        if line_end == rest.len() {
            break;
        }
        idx = line_end + 1;
    }
    (None, source)
}

/// Parse the YAML chunk produced by [`split_frontmatter`]. An empty
/// chunk yields [`SkillFrontmatter::empty`]; malformed YAML returns an
/// `Err` carrying the parser message.
pub fn parse_frontmatter(yaml_src: &str) -> Result<SkillFrontmatter, yaml_serde::Error> {
    if yaml_src.trim().is_empty() {
        return Ok(SkillFrontmatter::empty());
    }
    let value: Value = yaml_serde::from_str(yaml_src)?;
    let mapping = match value {
        Value::Mapping(m) => m,
        Value::Null => return Ok(SkillFrontmatter::empty()),
        _ => {
            // Treat non-mapping roots as empty + preserve nothing —
            // matches yaml-frontmatter's behaviour on user mistakes.
            return Ok(SkillFrontmatter::empty());
        }
    };

    let mut fm = SkillFrontmatter {
        user_invocable: DEFAULT_USER_INVOCABLE,
        ..SkillFrontmatter::default()
    };
    let known: std::collections::HashSet<&str> = KNOWN_KEYS.iter().copied().collect();

    for (k, v) in mapping {
        let key = match &k {
            Value::String(s) => s.clone(),
            other => match yaml_serde::to_string(other) {
                Ok(rendered) => rendered.trim().to_string(),
                Err(_) => continue,
            },
        };
        if !known.contains(key.as_str()) {
            fm.extra.insert(key, v);
            continue;
        }
        match key.as_str() {
            "name" => fm.name = value_to_string(&v),
            "description" => fm.description = value_to_string(&v),
            "when_to_use" => fm.when_to_use = value_to_string(&v),
            "argument-hint" => fm.argument_hint = value_to_string(&v),
            "arguments" => fm.arguments = value_to_string_list(&v),
            "allowed-tools" => fm.allowed_tools = value_to_string(&v),
            "paths" => fm.paths = value_to_string(&v),
            "context" => fm.context = value_to_string(&v),
            "agent" => fm.agent = value_to_string(&v),
            "model" => fm.model = value_to_string(&v),
            "effort" => fm.effort = value_to_string(&v),
            "shell" => fm.shell = value_to_string(&v),
            "disable-model-invocation" => {
                fm.disable_model_invocation = value_to_bool(&v).unwrap_or(false);
            }
            "user-invocable" => {
                fm.user_invocable = value_to_bool(&v).unwrap_or(DEFAULT_USER_INVOCABLE);
            }
            _ => unreachable!("KNOWN_KEYS membership already filtered"),
        }
    }
    Ok(fm)
}

/// Serialize `fm` back to a YAML frontmatter string **including** the
/// `---` fences and trailing newline. Order is fixed — typed keys
/// first (in spec order), then any `extra` keys alphabetically. Keys
/// whose value is `None` / empty / default are omitted entirely so the
/// output stays minimal.
pub fn serialize_frontmatter(fm: &SkillFrontmatter) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(256);
    out.push_str("---\n");

    // Helpers — keep one place for the YAML scalar rules so the modal
    // never produces invalid frontmatter.
    let push_scalar = |out: &mut String, key: &str, val: &str| {
        let _ = writeln!(out, "{}: {}", key, scalar_yaml(val));
    };
    let push_bool = |out: &mut String, key: &str, val: bool| {
        let _ = writeln!(out, "{}: {}", key, val);
    };

    if let Some(v) = fm.name.as_deref() {
        push_scalar(&mut out, "name", v);
    }
    if let Some(v) = fm.description.as_deref() {
        push_scalar(&mut out, "description", v);
    }
    if let Some(v) = fm.when_to_use.as_deref() {
        push_scalar(&mut out, "when_to_use", v);
    }
    if let Some(v) = fm.argument_hint.as_deref() {
        push_scalar(&mut out, "argument-hint", v);
    }
    if !fm.arguments.is_empty() {
        out.push_str("arguments:\n");
        for arg in &fm.arguments {
            let _ = writeln!(out, "  - {}", scalar_yaml(arg));
        }
    }
    if let Some(v) = fm.allowed_tools.as_deref() {
        push_scalar(&mut out, "allowed-tools", v);
    }
    if let Some(v) = fm.paths.as_deref() {
        push_scalar(&mut out, "paths", v);
    }
    if let Some(v) = fm.context.as_deref() {
        push_scalar(&mut out, "context", v);
    }
    if let Some(v) = fm.agent.as_deref() {
        push_scalar(&mut out, "agent", v);
    }
    if let Some(v) = fm.model.as_deref() {
        push_scalar(&mut out, "model", v);
    }
    if let Some(v) = fm.effort.as_deref() {
        push_scalar(&mut out, "effort", v);
    }
    if let Some(v) = fm.shell.as_deref() {
        push_scalar(&mut out, "shell", v);
    }
    if fm.disable_model_invocation {
        push_bool(&mut out, "disable-model-invocation", true);
    }
    if fm.user_invocable != DEFAULT_USER_INVOCABLE {
        push_bool(&mut out, "user-invocable", fm.user_invocable);
    }

    // `extra` round-trip — let yaml_serde render the values; trim
    // trailing newline so we control exactly one between entries.
    for (key, val) in &fm.extra {
        let mut single = yaml_serde::Mapping::new();
        single.insert(Value::String(key.clone()), val.clone());
        match yaml_serde::to_string(&Value::Mapping(single)) {
            Ok(rendered) => {
                let trimmed = rendered.trim_end_matches('\n');
                out.push_str(trimmed);
                out.push('\n');
            }
            Err(_) => {
                // Drop the key on serialization failure — preserving
                // an unparseable Value here would corrupt the file.
                continue;
            }
        }
    }

    out.push_str("---\n");
    out
}

/// Compose the full SKILL.md source from a frontmatter struct + body.
/// Inserts a single blank line after the closing `---` so the body
/// starts cleanly.
pub fn render_skill_md(fm: &SkillFrontmatter, body: &str) -> String {
    let mut out = serialize_frontmatter(fm);
    out.push('\n');
    out.push_str(body.trim_start_matches('\n'));
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn value_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Null => None,
        _ => None,
    }
}

fn value_to_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Some(true),
            "false" | "no" | "off" | "0" => Some(false),
            _ => None,
        },
        Value::Number(n) => n.as_i64().map(|x| x != 0),
        _ => None,
    }
}

fn value_to_string_list(v: &Value) -> Vec<String> {
    match v {
        Value::Sequence(seq) => seq.iter().filter_map(value_to_string).collect(),
        Value::String(s) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// Render a scalar suitable for a single-line `key: value` pair. Quotes
/// the value when YAML 1.2 plain-scalar rules would mis-parse it.
///
/// Quote-triggering cases:
/// - empty string
/// - leading or trailing whitespace
/// - any line break
/// - YAML indicator characters (`: # { } [ ] , & * ? | > ! % @ \``)
/// - leading scalar markers that change semantics (`- ? : ' " % @ \``)
/// - **YAML 1.2 reserved scalars** that the core schema would re-parse
///   as `bool` / `null` / `int` / `float` and round-trip back as a
///   non-string, silently dropping the value through
///   [`value_to_string`]. This includes `"true"`, `"false"`, `"null"`,
///   `"~"`, `".inf"` / `".nan"` (case-variants), and any string that
///   parses as `i64` or `f64`. Without this guard a user setting
///   `description: "true"` would round-trip to `description: None`.
fn scalar_yaml(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }
    let needs_quote = value.contains([
        ':', '#', '{', '}', '[', ']', ',', '&', '*', '?', '|', '>', '!', '%', '@', '`',
    ]) || value.starts_with([' ', '-', '?', ':', '\'', '"', '%', '@', '`'])
        || value.ends_with(' ')
        || value.contains('\n')
        || is_yaml_reserved_scalar(value);
    if !needs_quote {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Strings that YAML 1.2's core schema parses as a non-string type.
/// We have to quote them on emit so the next round-trip parse keeps
/// them as `String`.
fn is_yaml_reserved_scalar(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "null"
            | "~"
            | "true"
            | "false"
            | ".inf"
            | "+.inf"
            | "-.inf"
            | ".nan"
            | "yes"
            | "no"
            | "on"
            | "off"
    ) {
        return true;
    }
    // Decimal / hex / octal integer literals.
    if value.parse::<i64>().is_ok() {
        return true;
    }
    // Float literals (excluding the special `.inf`/`.nan` already
    // handled above — `parse::<f64>` accepts `inf`/`nan` too).
    if value.parse::<f64>().is_ok() {
        return true;
    }
    false
}
