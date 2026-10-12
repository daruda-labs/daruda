//! Helpers shared by the AddMcpServerModal / EditMcpServerModal.

use crate::ui::theme;
use gpui::{IntoElement, SharedString, div, prelude::*, px};

use crate::agent::mcp::{FieldError, McpScope, McpTransport};
use crate::surface::strings;

/// Quotes group arguments; backslashes are literal on every platform.
/// A doubled matching quote inside a quoted argument represents one quote.
pub(super) fn split_args(s: &str) -> Result<Vec<String>, SharedString> {
    let mut args = Vec::new();
    let mut value = String::new();
    let mut quote = None;
    let mut started = false;
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(delimiter) = quote {
            if ch == delimiter {
                if chars.peek() == Some(&delimiter) {
                    chars.next();
                    value.push(ch);
                } else {
                    quote = None;
                }
            } else {
                value.push(ch);
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
            started = true;
        } else if ch.is_whitespace() {
            if started {
                args.push(std::mem::take(&mut value));
                started = false;
            }
        } else {
            value.push(ch);
            started = true;
        }
    }
    if quote.is_some() {
        return Err(strings::mcp::args_invalid().into());
    }
    if started {
        args.push(value);
    }
    Ok(args)
}

pub(super) fn join_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.is_empty()
                || arg
                    .chars()
                    .any(|ch| ch.is_whitespace() || ch == '\'' || ch == '"')
            {
                format!("\"{}\"", arg.replace('"', "\"\""))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn transport_options() -> Vec<(McpTransport, &'static str)> {
    [McpTransport::Stdio, McpTransport::Sse, McpTransport::Http]
        .into_iter()
        .map(|t| (t, t.slug()))
        .collect()
}

pub(super) fn scope_options() -> Vec<(McpScope, String)> {
    vec![
        (McpScope::Project, strings::common::section_project()),
        (McpScope::Local, strings::mcp::scope_local_short()),
        (McpScope::User, strings::common::section_user()),
    ]
}

/// Small label rendered above a form field in the modal stack-style
/// layout (label-on-top instead of inline).
pub(super) fn field_label(
    text: impl Into<SharedString>,
    t: &crate::ui::theme::DarudaTheme,
) -> impl IntoElement {
    div()
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(t.text_muted)
        .child(text.into())
}

/// Map a typed [`FieldError`] back to its localised banner string.
pub(super) fn field_error_to_msg(e: FieldError) -> SharedString {
    match e {
        FieldError::CommandRequired => strings::mcp::command_required().into(),
        FieldError::UrlRequired => strings::mcp::url_required().into(),
        FieldError::UrlInvalidScheme => strings::mcp::url_invalid().into(),
        FieldError::EnvInvalidLine { line } => strings::mcp::env_invalid(&line).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parser records which line it choked on; the banner has to spend it.
    /// A message that only says "one of these must be KEY=VALUE" leaves the
    /// user scanning a block they may have pasted from a README.
    #[test]
    fn the_env_banner_names_the_line_that_failed() {
        let msg = field_error_to_msg(FieldError::EnvInvalidLine {
            line: "OPENAI_API_KEY".to_string(),
        });
        assert!(msg.contains("OPENAI_API_KEY"), "{msg}");
    }
}

/// Form parsing diagnoses duplicate keys and preserves value whitespace.
pub(super) fn parse_pairs(
    text: &str,
    headers: bool,
) -> Result<std::collections::BTreeMap<String, String>, SharedString> {
    let mut pairs = std::collections::BTreeMap::new();
    for (index, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() || raw.trim_start().starts_with('#') {
            continue;
        }
        let (key, value) = raw
            .split_once('=')
            .filter(|(key, _)| !key.trim().is_empty())
            .ok_or_else(|| {
                SharedString::from(if headers {
                    strings::mcp::headers_invalid(index + 1)
                } else {
                    strings::mcp::pairs_invalid(index + 1)
                })
            })?;
        let key = key.trim().to_string();
        if pairs.contains_key(&key)
            || (headers && pairs.keys().any(|k: &String| k.eq_ignore_ascii_case(&key)))
        {
            return Err(strings::mcp::pairs_duplicate(index + 1, &key).into());
        }
        pairs.insert(key, value.to_string());
    }
    Ok(pairs)
}

#[cfg(test)]
mod modal_safety_tests {
    use super::*;

    #[test]
    fn modal_safety_args_preserve_boundaries_empty_values_and_backslashes() {
        let args = vec![
            "--path".into(),
            "/tmp/my folder".into(),
            "".into(),
            r"C:\Users\Ada\file".into(),
            "a'\"b".into(),
        ];
        assert_eq!(split_args(&join_args(&args)).unwrap(), args);
        assert!(split_args("--path 'unfinished").is_err());
    }

    #[test]
    fn modal_safety_typed_paths_are_literal() {
        assert_eq!(
            split_args(r#"C:\Users\Ada\server.js "C:\Program Files\server.js" \\host\share\ #tag"#)
                .unwrap(),
            vec![
                r"C:\Users\Ada\server.js",
                r"C:\Program Files\server.js",
                r"\\host\share\",
                "#tag"
            ]
        );
        assert_eq!(
            split_args(r#""say ""hello""" ''"#).unwrap(),
            vec!["say \"hello\"", ""]
        );
    }

    #[test]
    fn modal_safety_pairs_preserve_values_and_reject_duplicate_keys() {
        let pairs = parse_pairs("TOKEN=  value  \nEMPTY=\nJSON=a=b", false).unwrap();
        assert_eq!(pairs["TOKEN"], "  value  ");
        assert_eq!(pairs["EMPTY"], "");
        assert_eq!(pairs["JSON"], "a=b");
        assert!(parse_pairs("TOKEN=a\nTOKEN=b", false).is_err());
        assert!(
            parse_pairs("Authorization", true)
                .unwrap_err()
                .contains('1')
        );
    }
}
