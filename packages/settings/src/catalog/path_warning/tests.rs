use super::{agent_command_path_warning, path_check_token};

/// Never a real executable name, so `which` is guaranteed to miss it.
const MISSING_COMMAND: &str = "daruda-settings-path-warning-test-missing-binary";

#[test]
fn path_check_token_cases() {
    let cases = [
        ("npx", "npx -y some-pkg@latest --acp", None),
        ("uvx", "uvx some-pkg@latest -x", None),
        ("json stdio", r#"{"command": "some-binary"}"#, None),
        (
            "env npx",
            "AUGMENT_DISABLE_AUTO_UPDATE=1 npx -y pkg@latest --acp",
            None,
        ),
        (
            "env local command",
            "FOO=1 my-local-cli acp",
            Some("my-local-cli"),
        ),
        ("local command", "my-local-cli acp", Some("my-local-cli")),
    ];

    for (name, command, expected) in cases {
        assert_eq!(path_check_token(command).as_deref(), expected, "{name}");
    }
}

#[test]
fn agent_command_path_warning_cases() {
    let cases = [
        ("found on path", "sh -c true", None),
        ("missing", MISSING_COMMAND, Some(MISSING_COMMAND)),
        (
            "npx unavailable",
            "npx -y definitely-nonexistent-package@latest",
            None,
        ),
    ];

    for (name, command, expected) in cases {
        assert_eq!(
            agent_command_path_warning(command).as_deref(),
            expected,
            "{name}"
        );
    }
}
