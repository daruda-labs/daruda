use super::*;

#[test]
fn quick_entry_preserves_unknown_json_fields_and_multiple_servers() {
    let drafts = parse(r#"{"mcpServers":{"a":{"command":"npx","args":["","a b"],"future":{"x":1}},"b":{"url":"https://example.com/mcp"}}}"#).unwrap();
    assert_eq!(drafts.len(), 2);
    assert_eq!(drafts[0].args, ["", "a b"]);
    assert_eq!(drafts[0].extra["future"], serde_json::json!({"x":1}));
    assert_eq!(drafts[1].transport, McpTransport::Http);
}
#[test]
fn quick_entry_never_accepts_shell_programs_or_coerces_invalid_fields() {
    for source in [
        "npx x && echo ok",
        "TOKEN=secret npx x",
        "npx $(echo x)",
        "npx 'unterminated",
        r#"{"command":"npx","args":[1]}"#,
        r#"{"command":"npx","url":"https://example.com"}"#,
        r#"{"command":"npx","env":{"TOKEN":42}}"#,
    ] {
        assert!(parse(source).is_err(), "{source}");
    }
    let draft = parse("npx -y 'some package' --name='two words'")
        .unwrap()
        .remove(0);
    assert_eq!(draft.args, ["-y", "some package", "--name=two words"]);
}
#[test]
fn quick_entry_codex_conversion_is_explicit_and_lossless_or_rejected() {
    let drafts =
        parse("[mcp_servers.docs]\nurl='https://example.com/mcp'\nbearer_token_env_var='TOKEN'")
            .unwrap();
    assert_eq!(drafts[0].headers["Authorization"], "Bearer ${TOKEN}");
    assert!(parse("[mcp_servers.docs]\ncommand='node'\nstartup_timeout_sec=12").is_err());
    assert!(parse("[mcp_servers.docs]\nurl='https://example.com/${TOKEN}'").is_err());
}

#[test]
fn quick_entry_rejects_ambiguous_duplicate_keys_and_invalid_urls() {
    for source in [
        r#"{"command":"node","command":"npx"}"#,
        r#"{"command":"node","future":{"x":1,"x":2}}"#,
        r#"{"url":"https://example.com","headers":{"Authorization":"a","authorization":"b"}}"#,
        "https://",
        "https://example.com invalid",
    ] {
        assert!(parse(source).is_err(), "{source}");
    }
}
