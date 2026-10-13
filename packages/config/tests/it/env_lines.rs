//! Public API tests for env lines.

use daruda_config::env_lines::*;
use std::collections::BTreeMap;

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
