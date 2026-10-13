use super::*;

/// The diagnostic exists so a failed spawn names its launcher. An agent's
/// declared environment rides in the same string, so the one thing it must
/// never carry is a value.
#[test]
fn command_diagnostic_keeps_the_launcher_and_drops_every_value() {
    let secret = "sk-live-do-not-log";

    let local = format!("OPENAI_API_KEY='{secret}' CODEX_HOME='/tmp/a' npx -y some-acp");
    let got = command_diagnostic(&local);
    assert!(!got.contains(secret), "{got}");
    assert!(!got.contains("/tmp/a"), "{got}");
    assert_eq!(got, "npx (env: OPENAI_API_KEY, CODEX_HOME)");

    // The remote forms hide their assignments inside the quoted script,
    // so nothing past the transport token may be echoed.
    let remote =
        format!("ssh vm sh -c 'cd \"/w\" && export OPENAI_API_KEY='\\''{secret}'\\''; acp'");
    let got = command_diagnostic(&remote);
    assert!(!got.contains(secret), "{got}");
    assert_eq!(got, "ssh");

    // A JSON stdio config carries its env as a structured field.
    let json = format!(r#"{{"command":"/opt/acp","env":{{"TOKEN":"{secret}"}}}}"#);
    let got = command_diagnostic(&json);
    assert!(!got.contains(secret), "{got}");
    assert_eq!(got, JSON_STDIO_DIAGNOSTIC);

    assert_eq!(command_diagnostic("npx -y some-acp"), "npx");
    assert_eq!(command_diagnostic("   "), NO_LAUNCHER_DIAGNOSTIC);
}

#[test]
fn command_needs_node_only_for_npx_and_node_launchers() {
    assert!(command_needs_node(
        "npx -y @agentclientprotocol/claude-agent-acp@latest"
    ));
    assert!(command_needs_node("node /path/adapter.js"));
    // Leading whitespace before the launcher must not hide it.
    assert!(command_needs_node("   npx -y pkg"));
    // Registry commands may prefix environment assignments before npx.
    assert!(command_needs_node(
        "AUGMENT_DISABLE_AUTO_UPDATE=1 npx -y pkg --acp"
    ));
    assert!(command_needs_node("A=1 B=false node /path/adapter.js"));

    // A non-Node binary runs itself.
    assert!(!command_needs_node("/usr/local/bin/codex-acp"));
    assert!(!command_needs_node("A=1 /usr/local/bin/codex-acp"));
    assert!(!command_needs_node("A=1"));
    // A JSON config config is a self-contained transport.
    assert!(!command_needs_node(
        r#"{"type":"config","command":"codex","args":[]}"#
    ));
    // Empty string needs nothing.
    assert!(!command_needs_node(""));
}

#[test]
fn split_env_prefixed_tokens_dequotes_a_single_quoted_spaced_value() {
    // Regression: `AgentLaunch::wrap_with_env`'s `Raw` branch
    // single-quotes an env value that can contain a space (a real
    // account config dir under `default_data_dir()`, e.g. macOS
    // `~/Library/Application Support/...`). Naive `split_whitespace`
    // would break this into two command tokens; the quote-aware split
    // must parse it as one assignment with the quotes stripped and the
    // internal space intact.
    let command =
        "CLAUDE_CONFIG_DIR='/Users/x/Library/Application Support/daruda/acc/alice' npx -y pkg";
    let (env, tokens) = split_env_prefixed_tokens(command);
    assert_eq!(
        env,
        vec![(
            "CLAUDE_CONFIG_DIR".to_string(),
            "/Users/x/Library/Application Support/daruda/acc/alice".to_string()
        )]
    );
    assert_eq!(
        tokens,
        vec!["npx".to_string(), "-y".to_string(), "pkg".to_string()]
    );
}

#[test]
fn split_env_prefixed_tokens_dequotes_a_double_quoted_spaced_value() {
    let command =
        r#"CLAUDE_CONFIG_DIR="/Users/x/Library/Application Support/daruda/acc/alice" npx -y pkg"#;
    let (env, tokens) = split_env_prefixed_tokens(command);
    assert_eq!(
        env,
        vec![(
            "CLAUDE_CONFIG_DIR".to_string(),
            "/Users/x/Library/Application Support/daruda/acc/alice".to_string()
        )]
    );
    assert_eq!(
        tokens,
        vec!["npx".to_string(), "-y".to_string(), "pkg".to_string()]
    );
}

#[test]
fn split_env_prefixed_tokens_still_parses_an_unquoted_assignment() {
    // Regression: the pre-existing, still-legal unquoted `NAME=value`
    // prefix (e.g. `AUGMENT_DISABLE_AUTO_UPDATE=1 npx ...`) must keep
    // working after switching the tokenizer from `split_whitespace` to
    // `shell_words::split`.
    let command = "AUGMENT_DISABLE_AUTO_UPDATE=1 npx -y pkg --acp";
    let (env, tokens) = split_env_prefixed_tokens(command);
    assert_eq!(
        env,
        vec![("AUGMENT_DISABLE_AUTO_UPDATE".to_string(), "1".to_string())]
    );
    assert_eq!(
        tokens,
        vec![
            "npx".to_string(),
            "-y".to_string(),
            "pkg".to_string(),
            "--acp".to_string()
        ]
    );
}
