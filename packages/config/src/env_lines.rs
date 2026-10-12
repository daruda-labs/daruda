//! Environment variables in the block form a user edits them in — one
//! `KEY=VALUE` per line. Shared by the MCP server editor and the agent
//! catalog's Environment field.

use std::collections::BTreeMap;

/// A line that is not `KEY=VALUE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvLineError {
    pub line: String,
}

/// Parse `KEY=VALUE` lines (one per line) into a sorted map. Empty
/// lines and lines starting with `#` are treated as comments. Both
/// the key and the value are trimmed of surrounding whitespace —
/// `TOKEN=  abc  ` lands in the map as `("TOKEN", "abc")`. Embedded
/// whitespace (`KEY=foo bar`) is preserved verbatim. An empty value
/// (`KEY=`) is allowed and stored as the empty string — Claude Code
/// writes a literal empty env var in this case.
pub fn parse_env_lines(text: &str) -> Result<BTreeMap<String, String>, EnvLineError> {
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line.split_once('=').ok_or_else(|| EnvLineError {
            line: line.to_string(),
        })?;
        let k = k.trim();
        if k.is_empty() {
            return Err(EnvLineError {
                line: line.to_string(),
            });
        }
        map.insert(k.to_string(), v.trim().to_string());
    }
    Ok(map)
}

/// Render an env map back to the block form [`parse_env_lines`] reads.
pub fn format_env_lines(env: &BTreeMap<String, String>) -> String {
    env.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_lines_skips_comments_and_blank_lines() {
        let text = "# this is a comment\n\nFOO=bar\n  TOKEN=secret\n";
        let env = parse_env_lines(text).unwrap();
        assert_eq!(env.get("FOO").unwrap(), "bar");
        assert_eq!(env.get("TOKEN").unwrap(), "secret");
        assert_eq!(env.len(), 2);
    }

    #[test]
    fn parse_env_lines_trims_value_padding() {
        let env = parse_env_lines("KEY=  padded  ").unwrap();
        assert_eq!(env.get("KEY").unwrap(), "padded");
    }

    #[test]
    fn parse_env_lines_preserves_embedded_whitespace() {
        let env = parse_env_lines("MSG=hello world\n").unwrap();
        assert_eq!(env.get("MSG").unwrap(), "hello world");
    }

    #[test]
    fn parse_env_lines_accepts_empty_value() {
        let env = parse_env_lines("UNSET=").unwrap();
        assert_eq!(env.get("UNSET").unwrap(), "");
        assert_eq!(env.len(), 1);
    }

    #[test]
    fn parse_env_lines_rejects_bad_form() {
        assert!(parse_env_lines("no_equals_here").is_err());
        assert!(parse_env_lines("=novalue").is_err());
    }

    #[test]
    fn format_env_lines_round_trip_sorted() {
        let env: BTreeMap<String, String> = [
            ("ZED".to_string(), "1".to_string()),
            ("ALPHA".to_string(), "2".to_string()),
        ]
        .into_iter()
        .collect();
        let text = format_env_lines(&env);
        // BTreeMap iteration is sorted — ALPHA precedes ZED.
        assert!(text.starts_with("ALPHA="));
    }
}
