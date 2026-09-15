use super::*;

#[test]
fn recognizes_only_supported_local_npx_launches() {
    for command in [
        "npx -y @agentclientprotocol/claude-agent-acp@latest",
        "npx --yes -- @agentclientprotocol/codex-acp@1.11.0",
        "npx @agentclientprotocol/claude-agent-acp",
    ] {
        assert!(NpmAdapter::parse(command).is_some(), "{command}");
    }
    for command in [
        "npx -y unrelated-package",
        "npx --offline @agentclientprotocol/claude-agent-acp@latest",
        "npx -p @agentclientprotocol/claude-agent-acp claude-agent-acp",
        "/custom/bin/npx -y @agentclientprotocol/claude-agent-acp@latest",
        "ssh host npx -y @agentclientprotocol/claude-agent-acp@latest",
        "docker exec box npx -y @agentclientprotocol/claude-agent-acp@latest",
        "node adapter.js",
        "npx @agentclientprotocol/claude-agent-acp-unknown@latest",
        "npx @agentclientprotocol/claude-agent-acp@file:local",
    ] {
        assert!(NpmAdapter::parse(command).is_none(), "{command}");
    }
}

#[test]
fn preserves_quoted_environment_arguments_and_json_launches() {
    let adapter = NpmAdapter::parse("CLAUDE_CONFIG_DIR='/space in/path' npx -y @agentclientprotocol/claude-agent-acp@0.77.0 --cli auth login").unwrap();
    assert_eq!(adapter.selector, "0.77.0");
    assert_eq!(
        adapter.config.environment()["CLAUDE_CONFIG_DIR"],
        "/space in/path"
    );
    assert_eq!(adapter.config.arguments(), ["--cli", "auth", "login"]);
    let json = r#"{"command":"npx","args":["-y","@agentclientprotocol/codex-acp@latest"],"env":{"CODEX_HOME":"/space in/path"}}"#;
    assert_eq!(
        NpmAdapter::parse(json).unwrap().config.environment()["CODEX_HOME"],
        "/space in/path"
    );
}

/// Opt-in integration check: downloads packages into an isolated temporary root.
#[test]
#[ignore = "requires npm registry access and a Node runtime"]
fn prepares_published_adapters_without_starting_acp() {
    let root = tempfile::tempdir().unwrap();
    for (name, _) in SUPPORTED_ADAPTERS {
        let launch = crate::LaunchSpec {
            command: format!("npx -y {name}@latest"),
            strip_env: Vec::new(),
        };
        let prepared =
            crate::launch_env::prepare_adapter_command(&launch, root.path(), &mut |_| {}).unwrap();
        let command = prepared.command();
        let config = AcpAgent::from_str(&command.0).unwrap().into_config();
        assert_eq!(config.command().file_name().unwrap(), "node");
        assert!(Path::new(&config.arguments()[0]).is_file());
        assert!(!config.arguments().iter().any(|arg| arg == "-y"));
        let offline_config = AcpAgent::from_str(&launch.command)
            .unwrap()
            .into_config()
            .env("npm_config_offline", "true");
        let offline = crate::LaunchSpec {
            command: serde_json::to_string(&offline_config).unwrap(),
            strip_env: Vec::new(),
        };
        let notices = std::cell::RefCell::new(Vec::new());
        let notice = |text: &str| notices.borrow_mut().push(text.to_owned());
        let context = PreparationContext::new(&|| false, &notice);
        let reused =
            crate::launch_env::prepare_adapter(&offline, root.path(), &mut |_| {}, &context)
                .unwrap();
        let reused = AcpAgent::from_str(&reused.command().0)
            .unwrap()
            .into_config();
        assert_eq!(reused.arguments()[0], config.arguments()[0]);
        assert!(
            notices
                .borrow()
                .iter()
                .any(|notice| notice.contains("using verified cached"))
        );
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires Node on PATH"]
fn node_runs_a_readable_entry_without_execute_permission() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir().unwrap();
    let entry = root.path().join("entry with spaces.js");
    std::fs::write(&entry, "console.log('ready')").unwrap();
    std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        std::process::Command::new(&entry)
            .output()
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    let output = std::process::Command::new(which::which("node").unwrap())
        .arg(entry)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "ready");
}
