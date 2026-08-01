//! Tests for the agent config model — permission modes, launch shapes,
//! serde round-trips and the built-in defaults.

use super::*;

#[test]
fn mode_id_strings_are_exact() {
    assert_eq!(DefaultPermissionMode::Auto.mode_id(), "auto");
    assert_eq!(DefaultPermissionMode::Default.mode_id(), "default");
    assert_eq!(DefaultPermissionMode::AcceptEdits.mode_id(), "acceptEdits");
    assert_eq!(DefaultPermissionMode::Plan.mode_id(), "plan");
    assert_eq!(DefaultPermissionMode::DontAsk.mode_id(), "dontAsk");
    assert_eq!(
        DefaultPermissionMode::BypassPermissions.mode_id(),
        "bypassPermissions"
    );
}

#[test]
fn default_is_bypass_permissions() {
    assert_eq!(
        DefaultPermissionMode::default(),
        DefaultPermissionMode::BypassPermissions
    );
    assert_eq!(
        AgentConfig::default().default_permission_mode.mode_id(),
        "bypassPermissions"
    );
}

fn with_default_mode(mode: DefaultPermissionMode) -> AgentConfig {
    AgentConfig {
        default_permission_mode: mode,
        ..AgentConfig::default()
    }
}

#[test]
fn connect_mode_priority_appends_the_auto_fallback() {
    let config = with_default_mode(DefaultPermissionMode::BypassPermissions);
    assert_eq!(
        config.connect_mode_priority(None),
        ["bypassPermissions", "auto"]
    );

    let config = with_default_mode(DefaultPermissionMode::Auto);
    assert_eq!(
        config.connect_mode_priority(None),
        ["auto"],
        "the fallback is not repeated when it is already the default"
    );
}

#[test]
fn a_per_agent_mode_outranks_the_global_default() {
    let config = with_default_mode(DefaultPermissionMode::BypassPermissions);
    assert_eq!(
        config.connect_mode_priority(Some("yolo")),
        ["yolo", "bypassPermissions", "auto"],
        "an agent's own vocabulary is tried first, the global default backs it up"
    );
}

#[test]
fn connect_mode_priority_drops_blank_and_duplicate_candidates() {
    let config = with_default_mode(DefaultPermissionMode::Plan);
    assert_eq!(
        config.connect_mode_priority(Some("  ")),
        ["plan", "auto"],
        "a whitespace-only override is no override"
    );
    assert_eq!(
        config.connect_mode_priority(Some("plan")),
        ["plan", "auto"],
        "an override equal to the global default is not tried twice"
    );
}

#[test]
fn from_mode_id_round_trips_all_variants() {
    for m in DefaultPermissionMode::ALL {
        assert_eq!(
            DefaultPermissionMode::from_mode_id(m.mode_id()),
            Some(m),
            "from_mode_id({}) should return the original variant",
            m.mode_id()
        );
    }
}

#[test]
fn from_mode_id_returns_none_for_unknown_id() {
    assert_eq!(DefaultPermissionMode::from_mode_id("bogus"), None);
    assert_eq!(DefaultPermissionMode::from_mode_id(""), None);
    assert_eq!(
        DefaultPermissionMode::from_mode_id("BypassPermissions"),
        None
    );
}

#[test]
fn toml_round_trip_all_variants() {
    // Verify serde(rename_all = "camelCase") produces the right TOML keys.
    let cases = [
        (DefaultPermissionMode::Auto, "auto"),
        (DefaultPermissionMode::Default, "default"),
        (DefaultPermissionMode::AcceptEdits, "acceptEdits"),
        (DefaultPermissionMode::Plan, "plan"),
        (DefaultPermissionMode::DontAsk, "dontAsk"),
        (
            DefaultPermissionMode::BypassPermissions,
            "bypassPermissions",
        ),
    ];
    for (variant, expected_str) in cases {
        let cfg = AgentConfig {
            default_permission_mode: variant,
            ..AgentConfig::default()
        };
        let toml_str = toml::to_string(&cfg).expect("serialize");
        assert!(
            toml_str.contains(expected_str),
            "expected TOML to contain \"{expected_str}\", got: {toml_str}"
        );
        let back: AgentConfig = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(back.default_permission_mode, variant);
    }
}

#[test]
fn missing_agent_section_deserializes_to_default() {
    let cfg: AgentConfig = toml::from_str("").expect("empty TOML should produce defaults");
    assert_eq!(
        cfg.default_permission_mode,
        DefaultPermissionMode::BypassPermissions
    );
}

#[test]
fn use_modifier_to_send_defaults_false_and_round_trips() {
    // Default matches Zed's agent panel: Enter sends, Shift+Enter newline.
    assert!(!AgentConfig::default().use_modifier_to_send);

    let cfg = AgentConfig {
        use_modifier_to_send: true,
        ..AgentConfig::default()
    };
    let toml_str = toml::to_string(&cfg).expect("serialize");
    let back: AgentConfig = toml::from_str(&toml_str).expect("deserialize");
    assert!(back.use_modifier_to_send);

    // A config that omits the key keeps the default.
    let omitted: AgentConfig =
        toml::from_str("default_permission_mode = \"plan\"").expect("deserialize");
    assert!(!omitted.use_modifier_to_send);
}

#[test]
fn hidden_config_option_descriptions_defaults_to_fast_mode_and_is_clearable() {
    // Shipped default: Claude's Fast mode chip is hidden for everyone —
    // the toggle does not stick through the ACP adapter path, so by
    // default the chip would only silently flip back.
    assert_eq!(
        AgentConfig::default().hidden_config_option_descriptions,
        vec![FAST_MODE_PLAIN_DESCRIPTION.to_string()]
    );

    // Omitting the key keeps the default…
    let omitted: AgentConfig =
        toml::from_str("default_permission_mode = \"plan\"").expect("deserialize");
    assert_eq!(
        omitted.hidden_config_option_descriptions,
        vec![FAST_MODE_PLAIN_DESCRIPTION.to_string()]
    );

    // …while an explicit empty list opts back into showing everything.
    let cleared: AgentConfig =
        toml::from_str("hidden_config_option_descriptions = []").expect("deserialize");
    assert!(cleared.hidden_config_option_descriptions.is_empty());
}

#[test]
fn claude_default_has_expected_fields() {
    let d = AgentDefinition::claude_default();
    assert_eq!(d.id, "claude");
    assert_eq!(d.name, "Claude Code");
    assert_eq!(
        d.launch,
        AgentLaunch::Raw("npx -y @agentclientprotocol/claude-agent-acp@latest".to_string())
    );
}

#[test]
fn codex_default_has_expected_fields() {
    let d = AgentDefinition::codex_default();
    assert_eq!(d.id, "codex-acp");
    assert_eq!(d.name, "Codex");
    assert_eq!(
        d.launch,
        AgentLaunch::Raw("npx -y @agentclientprotocol/codex-acp@latest".to_string())
    );
}

#[test]
fn a_runnable_preset_resolves_to_a_definition_that_keeps_its_env_prefix() {
    // Every launchable preset must survive the trip through `registry_preset`
    // verbatim — `factory-droid` is the sharp case, since its command carries
    // leading `KEY=value` pairs the shell wrapper has to keep.
    for preset in preset::presets() {
        let PresetLaunchability::Runnable { command } = preset.launchability else {
            continue;
        };
        let definition = AgentDefinition::registry_preset(preset.id)
            .unwrap_or_else(|| panic!("{} is runnable", preset.id));
        assert_eq!(definition.launch, AgentLaunch::Raw(command.to_string()));
    }
    let droid = AgentDefinition::registry_preset("factory-droid").expect("runnable");
    let AgentLaunch::Raw(command) = &droid.launch else {
        panic!("presets launch Raw");
    };
    assert!(
        command.starts_with("DROID_DISABLE_AUTO_UPDATE=true "),
        "{command}"
    );
}

#[test]
fn registry_preset_lookup_declines_an_agent_that_needs_a_manual_install() {
    // `cursor` is in the table but ships only binary archives, so there is no
    // command to launch and no catalog row to add — same as an unknown id.
    assert!(preset::presets().any(|p| p.id == "cursor"));
    assert_eq!(AgentDefinition::registry_preset("cursor"), None);
    assert_eq!(AgentDefinition::registry_preset("no-such-agent"), None);
}

#[test]
fn default_agents_is_a_single_custom_claude_entry() {
    // Custom, not a `claude-acp` reference: see `default_agents`.
    assert_eq!(
        default_agents(),
        vec![AgentEntry::Custom(AgentDefinition::claude_default())]
    );
}

#[test]
fn agent_definition_field_round_trip() {
    let d = AgentDefinition {
        id: "codex".to_string(),
        name: "Codex".to_string(),
        launch: AgentLaunch::Raw("codex acp".to_string()),
        default_mode: Some("yolo".to_string()),
    };
    let toml_str = toml::to_string(&d).expect("serialize");
    let back: AgentDefinition = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(back, d);
}

#[test]
fn a_definition_without_a_default_mode_round_trips_and_stays_absent() {
    // Pre-`default_mode` configs must keep loading, and daruda must not
    // start writing an empty key back into them.
    let d: AgentDefinition =
        toml::from_str("id = \"codex\"\nname = \"Codex\"\ncommand = \"codex acp\"\n")
            .expect("deserialize");
    assert_eq!(d.default_mode, None);
    let toml_str = toml::to_string(&d).expect("serialize");
    assert!(!toml_str.contains("default_mode"), "{toml_str}");
}

#[test]
fn migration_command_toml_deserializes_to_raw_unchanged() {
    // Every pre-migration config line, including a hand-written {{cwd}}
    // token, must land in `Raw` byte-for-byte.
    let toml_str =
        "id = \"legacy\"\nname = \"Legacy\"\ncommand = \"ssh vm-work \\\"cd {{cwd}} && run\\\"\"\n";
    let d: AgentDefinition = toml::from_str(toml_str).expect("deserialize");
    assert_eq!(d.id, "legacy");
    assert_eq!(d.name, "Legacy");
    assert_eq!(
        d.launch,
        AgentLaunch::Raw("ssh vm-work \"cd {{cwd}} && run\"".to_string())
    );

    // And it serializes back to the exact same flat `command` shape.
    let toml_str = toml::to_string(&d).expect("serialize");
    assert!(toml_str.contains("command = "));
    assert!(!toml_str.contains("[ssh]"));
    assert!(!toml_str.contains("[docker]"));
}

#[test]
fn ssh_launch_toml_round_trips() {
    let d = AgentDefinition {
        id: "remote-agent".to_string(),
        name: "Remote Agent".to_string(),
        launch: AgentLaunch::Ssh {
            adapter_command: "npx -y some-acp".to_string(),
            host: "vm-work".to_string(),
        },
        default_mode: None,
    };
    let toml_str = toml::to_string(&d).expect("serialize");
    assert!(toml_str.contains("[ssh]"));
    assert!(toml_str.contains("host = \"vm-work\""));
    // No flat `command` key — only `adapter_command` inside `[ssh]`.
    assert!(!toml_str.contains("\ncommand = "));
    let back: AgentDefinition = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(back, d);
}

#[test]
fn docker_launch_toml_round_trips() {
    let d = AgentDefinition {
        id: "docker-agent".to_string(),
        name: "Docker Agent".to_string(),
        launch: AgentLaunch::Docker {
            adapter_command: "npx -y some-acp".to_string(),
            container: "ubuntu-dev".to_string(),
        },
        default_mode: None,
    };
    let toml_str = toml::to_string(&d).expect("serialize");
    assert!(toml_str.contains("[docker]"));
    assert!(toml_str.contains("container = \"ubuntu-dev\""));
    // No flat `command` key — only `adapter_command` inside `[docker]`.
    assert!(!toml_str.contains("\ncommand = "));
    let back: AgentDefinition = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(back, d);
}

#[test]
fn repr_priority_prefers_ssh_over_docker_over_command_when_hand_edited() {
    // Defensive priority order for a hand-edited config that sets more
    // than one launch shape at once: ssh > docker > command.
    let both: AgentDefinition = toml::from_str(
        "id = \"x\"\nname = \"X\"\ncommand = \"legacy\"\n\
         [docker]\nadapter_command = \"d\"\ncontainer = \"c\"\n\
         [ssh]\nadapter_command = \"s\"\nhost = \"h\"\n",
    )
    .expect("deserialize");
    assert_eq!(
        both.launch,
        AgentLaunch::Ssh {
            adapter_command: "s".to_string(),
            host: "h".to_string(),
        }
    );

    let docker_and_command: AgentDefinition = toml::from_str(
        "id = \"x\"\nname = \"X\"\ncommand = \"legacy\"\n\
         [docker]\nadapter_command = \"d\"\ncontainer = \"c\"\n",
    )
    .expect("deserialize");
    assert_eq!(
        docker_and_command.launch,
        AgentLaunch::Docker {
            adapter_command: "d".to_string(),
            container: "c".to_string(),
        }
    );
}

#[test]
fn needs_remote_cwd_raw_detects_token() {
    assert!(
        AgentLaunch::Raw(
            "ssh vm-work \"cd {{cwd}} && npx -y @agentclientprotocol/claude-agent-acp@latest\""
                .to_string()
        )
        .needs_remote_cwd()
    );
}

#[test]
fn needs_remote_cwd_raw_false_without_token() {
    assert!(
        !AgentLaunch::Raw("npx -y @agentclientprotocol/claude-agent-acp@latest".to_string())
            .needs_remote_cwd()
    );
}

#[test]
fn needs_remote_cwd_ssh_and_docker_are_always_true() {
    assert!(
        AgentLaunch::Ssh {
            adapter_command: "run".to_string(),
            host: "h".to_string(),
        }
        .needs_remote_cwd()
    );
    assert!(
        AgentLaunch::Docker {
            adapter_command: "run".to_string(),
            container: "c".to_string(),
        }
        .needs_remote_cwd()
    );
}

#[test]
fn wrap_raw_without_token_ignores_remote_path() {
    let launch = AgentLaunch::Raw("npx -y some-acp".to_string());
    assert_eq!(launch.wrap(None), Ok("npx -y some-acp".to_string()));
    assert_eq!(
        launch.wrap(Some("/tmp/anything")),
        Ok("npx -y some-acp".to_string())
    );
}

#[test]
fn wrap_raw_with_token_substitutes_remote_path() {
    let launch = AgentLaunch::Raw("ssh vm-work \"cd {{cwd}} && run\"".to_string());
    assert_eq!(
        launch.wrap(Some("/home/user/project")),
        Ok("ssh vm-work \"cd /home/user/project && run\"".to_string())
    );
}

#[test]
fn wrap_raw_with_token_errs_on_missing_or_blank_remote_path() {
    let launch = AgentLaunch::Raw("cd {{cwd}} && run".to_string());
    assert_eq!(launch.wrap(None), Err(()));
    assert_eq!(launch.wrap(Some("")), Err(()));
    assert_eq!(launch.wrap(Some("   ")), Err(()));
}

#[test]
fn wrap_ssh_builds_exact_command() {
    let launch = AgentLaunch::Ssh {
        adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".to_string(),
        host: "vm-work".to_string(),
    };
    assert_eq!(
        launch.wrap(Some("/home/user/project")),
        Ok(
            "ssh vm-work sh -c 'cd \"/home/user/project\" && npx -y @agentclientprotocol/claude-agent-acp@latest'"
                .to_string()
        )
    );
}

#[test]
fn wrap_ssh_errs_on_missing_or_blank_remote_path() {
    let launch = AgentLaunch::Ssh {
        adapter_command: "run".to_string(),
        host: "vm-work".to_string(),
    };
    assert_eq!(launch.wrap(None), Err(()));
    assert_eq!(launch.wrap(Some("  ")), Err(()));
}

#[test]
fn wrap_docker_builds_exact_command_with_dash_i_never_dash_it() {
    let launch = AgentLaunch::Docker {
        adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".to_string(),
        container: "ubuntu-dev".to_string(),
    };
    let wrapped = launch.wrap(Some("/home/user/project")).unwrap();
    assert_eq!(
        wrapped,
        "docker exec -i ubuntu-dev sh -c 'cd \"/home/user/project\" && npx -y @agentclientprotocol/claude-agent-acp@latest'"
    );
    assert!(wrapped.contains(" -i "));
    assert!(!wrapped.contains(" -it "));
}

#[test]
fn wrap_docker_errs_on_missing_or_blank_remote_path() {
    let launch = AgentLaunch::Docker {
        adapter_command: "run".to_string(),
        container: "ubuntu-dev".to_string(),
    };
    assert_eq!(launch.wrap(None), Err(()));
    assert_eq!(launch.wrap(Some("")), Err(()));
}

#[test]
fn wrap_with_env_prefixes_raw_command() {
    // The value carries a space — like a real account config dir under
    // `default_data_dir()`, which on macOS lands under `~/Library/
    // Application Support` — so the assertion also guards that the
    // emitted prefix is single-quoted, not just concatenated.
    use crate::account_env::AccountEnv;
    let launch = AgentLaunch::Raw("npx -y some-acp".to_string());
    let env = AccountEnv {
        inject: vec![(
            "CLAUDE_CONFIG_DIR".into(),
            "/Users/x/Library/Application Support/daruda/acc/alice".into(),
        )],
        strip: vec!["ANTHROPIC_API_KEY"],
    };
    let cmd = launch.wrap_with_env(None, &env).unwrap();
    assert!(
        cmd.starts_with(
            "CLAUDE_CONFIG_DIR='/Users/x/Library/Application Support/daruda/acc/alice' "
        )
    );
    assert!(cmd.contains("npx -y some-acp"));
}

#[test]
fn wrap_with_env_raw_emits_no_unset_flags() {
    // Deliberate: `daruda_acp::node::command_needs_node` reads the
    // launcher token after the `KEY=value` prefix, so adding an
    // `/usr/bin/env -u …` prefix here would hide `npx` and skip Node.js
    // provisioning. The strip is applied after runtime resolution, in
    // `launch_env::prepare_adapter_command`.
    use crate::account_env::AccountEnv;
    let launch = AgentLaunch::Raw("npx -y some-acp".to_string());
    let env = AccountEnv {
        inject: vec![("CLAUDE_CONFIG_DIR".into(), "/data/acc/alice".into())],
        strip: vec!["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"],
    };
    let cmd = launch.wrap_with_env(None, &env).unwrap();
    assert_eq!(cmd, "CLAUDE_CONFIG_DIR='/data/acc/alice' npx -y some-acp");
    assert!(!cmd.contains("-u "), "{cmd}");
    assert!(!cmd.contains("/usr/bin/env"), "{cmd}");
    assert!(!cmd.contains("unset "), "{cmd}");
}

#[test]
fn wrap_with_env_ssh_exports_and_unsets() {
    use crate::account_env::AccountEnv;
    let launch = AgentLaunch::Ssh {
        adapter_command: "npx -y some-acp".to_string(),
        host: "vm".to_string(),
    };
    let env = AccountEnv {
        inject: vec![("CLAUDE_CONFIG_DIR".into(), "/remote/acc".into())],
        strip: vec!["ANTHROPIC_API_KEY"],
    };
    let cmd = launch.wrap_with_env(Some("/work"), &env).unwrap();
    assert!(cmd.contains("export CLAUDE_CONFIG_DIR=\"/remote/acc\""));
    assert!(cmd.contains("unset ANTHROPIC_API_KEY"));
}

#[test]
fn login_command_appends_the_given_args_for_raw_only() {
    let raw = AgentLaunch::Raw("npx -y @agentclientprotocol/claude-agent-acp@latest".into());
    assert_eq!(
        raw.login_command("--cli auth login --claudeai").as_deref(),
        Some("npx -y @agentclientprotocol/claude-agent-acp@latest --cli auth login --claudeai")
    );
    // A different auth domain's args have a different shape — the
    // recipe owns the exact text, this only joins with one space.
    let codex = AgentLaunch::Raw("npx -y @agentclientprotocol/codex-acp@latest".into());
    assert_eq!(
        codex.login_command("cli login").as_deref(),
        Some("npx -y @agentclientprotocol/codex-acp@latest cli login")
    );
    let ssh = AgentLaunch::Ssh {
        adapter_command: "x".into(),
        host: "h".into(),
    };
    assert_eq!(ssh.login_command("cli login"), None);
    let docker = AgentLaunch::Docker {
        adapter_command: "x".into(),
        container: "c".into(),
    };
    assert_eq!(docker.login_command("cli login"), None);
}

#[test]
fn account_recipe_derives_the_auth_domain_from_the_adapter() {
    let raw = |c: &str| AgentLaunch::Raw(c.to_string());
    assert_eq!(
        raw("npx -y @agentclientprotocol/claude-agent-acp@latest").account_recipe(false),
        Some(AccountRecipeId::Claude)
    );
    assert_eq!(
        raw("npx -y @agentclientprotocol/codex-acp@latest").account_recipe(false),
        Some(AccountRecipeId::Codex)
    );
    // A version pin must not break derivation.
    assert_eq!(
        raw("npx -y @agentclientprotocol/codex-acp@1.1.0").account_recipe(false),
        Some(AccountRecipeId::Codex)
    );
    // The legacy Claude adapter shares Claude's credentials.
    assert_eq!(
        raw("npx -y @zed-industries/claude-code-acp@latest").account_recipe(false),
        Some(AccountRecipeId::Claude)
    );
    // An adapter with no managed-account support.
    assert_eq!(
        raw("npx -y @google/gemini-cli@latest --acp").account_recipe(false),
        None
    );
}

#[test]
fn account_recipe_is_none_for_every_remote_launch() {
    // Remote adapters have no local browser to complete OAuth in, even
    // when the adapter itself is a recognized auth domain.
    assert_eq!(
        AgentLaunch::Ssh {
            adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
            host: "vm-work".into(),
        }
        .account_recipe(false),
        None
    );
    assert_eq!(
        AgentLaunch::Docker {
            adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
            container: "ubuntu-dev".into(),
        }
        .account_recipe(false),
        None
    );
    // A `Raw` carrying `{{cwd}}` is the legacy remote escape hatch.
    assert_eq!(
        AgentLaunch::Raw(
            "docker exec -i ubuntu-dev sh -c \"cd {{cwd}} && npx -y @agentclientprotocol/claude-agent-acp@latest\""
                .into()
        )
        .account_recipe(false),
        None
    );
}

/// A plain `Raw` command carries no host of its own — it's the caller's
/// `is_remote` (the lane it's actually attached to) that must exclude it,
/// since `Ssh`/`Docker`/the `{{cwd}}` token can't self-report a lane-native
/// remote attachment.
#[test]
fn account_recipe_is_none_when_the_caller_reports_a_remote_context() {
    let claude = AgentLaunch::Raw("npx -y @agentclientprotocol/claude-agent-acp@latest".into());
    assert_eq!(claude.account_recipe(false), Some(AccountRecipeId::Claude));
    assert_eq!(claude.account_recipe(true), None);
}

#[test]
fn account_recipe_is_none_for_a_json_stdio_config() {
    // A JSON stdio config carries program/args/env as fields, so the
    // shell-string edits a managed account needs would corrupt it — such
    // a launch carries no account even though the adapter is recognized.
    let json = AgentLaunch::Raw(
        r#"{"command":"/opt/adapters/claude-agent-acp","args":["--stdio"]}"#.into(),
    );
    assert_eq!(json.account_recipe(false), None);
    // Leading whitespace must not hide the JSON shape.
    let padded = AgentLaunch::Raw("  {\"command\":\"/opt/adapters/codex-acp\",\"args\":[]}".into());
    assert_eq!(padded.account_recipe(false), None);
    // A plain shell command naming the same adapter still resolves.
    assert_eq!(
        AgentLaunch::Raw("/opt/adapters/claude-agent-acp --stdio".into()).account_recipe(false),
        Some(AccountRecipeId::Claude)
    );
}

/// `is_json_stdio` is the same discrimination `account_recipe` already
/// relies on, exposed as its own method for a lane-aware caller that needs
/// it independent of the recipe question.
#[test]
fn is_json_stdio_matches_the_leading_brace_only_for_raw() {
    assert!(AgentLaunch::Raw(r#"{"command":"x"}"#.into()).is_json_stdio());
    assert!(!AgentLaunch::Raw("npx -y some-acp".into()).is_json_stdio());
    // Ssh/Docker's adapter_command is always a shell command — never JSON,
    // even if someone typed a leading brace into it.
    assert!(
        !AgentLaunch::Ssh {
            adapter_command: r#"{"command":"x"}"#.into(),
            host: "box".into(),
        }
        .is_json_stdio()
    );
}

/// The fix for a deprecated `Ssh`/`Docker` launch that a lane resolves to
/// `Local`: `account_recipe` itself can't see past the launch's own shape,
/// but a caller holding the lane can feed the bare adapter command here
/// directly and get the same derivation a `Raw` launch would.
#[test]
fn account_recipe_for_local_command_matches_account_recipes_own_derivation() {
    assert_eq!(
        account_recipe_for_local_command("npx -y @agentclientprotocol/claude-agent-acp@latest"),
        Some(AccountRecipeId::Claude)
    );
    assert_eq!(
        account_recipe_for_local_command("npx -y @agentclientprotocol/codex-acp@latest"),
        Some(AccountRecipeId::Codex)
    );
    assert_eq!(
        account_recipe_for_local_command(r#"{"command":"x"}"#),
        None,
        "a JSON stdio adapter command is excluded here too"
    );
    assert_eq!(
        account_recipe_for_local_command("npx -y @google/gemini-cli@latest --acp"),
        None
    );
}

#[test]
fn built_in_defaults_derive_their_own_auth_domain() {
    // Guards a future command change from silently breaking derivation.
    assert_eq!(
        AgentDefinition::claude_default()
            .launch
            .account_recipe(false),
        Some(AccountRecipeId::Claude)
    );
    assert_eq!(
        AgentDefinition::codex_default()
            .launch
            .account_recipe(false),
        Some(AccountRecipeId::Codex)
    );
}

#[test]
fn input_max_rows_defaults_and_clamps() {
    assert_eq!(
        AgentConfig::default().input_max_rows,
        INPUT_MAX_ROWS_DEFAULT
    );

    // Round-trip a non-default value.
    let toml_str = "input_max_rows = 5";
    let cfg: AgentConfig = toml::from_str(toml_str).expect("deserialize");
    assert_eq!(cfg.input_max_rows, 5);

    // Values below the minimum are clamped up.
    let mut too_low: AgentConfig = toml::from_str("input_max_rows = 0").expect("deserialize");
    too_low.clamp();
    assert_eq!(too_low.input_max_rows, INPUT_MAX_ROWS_MIN);

    // Values above the maximum are clamped down.
    let mut too_high: AgentConfig = toml::from_str("input_max_rows = 255").expect("deserialize");
    too_high.clamp();
    assert_eq!(too_high.input_max_rows, INPUT_MAX_ROWS_MAX);

    // Omitting the key keeps the default.
    let omitted: AgentConfig = toml::from_str("use_modifier_to_send = false").expect("deserialize");
    assert_eq!(omitted.input_max_rows, INPUT_MAX_ROWS_DEFAULT);
}
