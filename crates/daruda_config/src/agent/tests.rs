//! Tests for the agent config model — launch shapes, serde round-trips and
//! the built-in defaults.

use super::*;

#[test]
fn connect_mode_priority_is_empty_without_an_agent_default() {
    // No per-agent override: the candidate list is empty, so the adapter's
    // own default mode applies untouched.
    assert_eq!(connect_mode_priority(None), Vec::<String>::new());
    // Whitespace-only is no override either.
    assert_eq!(connect_mode_priority(Some("   ")), Vec::<String>::new());
}

#[test]
fn connect_mode_priority_is_just_the_agent_default() {
    assert_eq!(
        connect_mode_priority(Some("yolo")),
        vec!["yolo".to_string()]
    );
    // Surrounding whitespace is trimmed.
    assert_eq!(
        connect_mode_priority(Some("  plan  ")),
        vec!["plan".to_string()]
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
    let omitted: AgentConfig = toml::from_str("input_max_rows = 5").expect("deserialize");
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
    let omitted: AgentConfig = toml::from_str("input_max_rows = 5").expect("deserialize");
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
        default_model: Some("gpt-5-codex".to_string()),
        fold_mode: None,
        tail_window: None,
        display_filter: None,
        env: None,
    };
    let toml_str = toml::to_string(&d).expect("serialize");
    let back: AgentDefinition = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(back, d);
}

#[test]
fn per_agent_transcript_defaults_round_trip() {
    let d = AgentDefinition {
        id: "codex".to_string(),
        name: "Codex".to_string(),
        launch: AgentLaunch::Raw("codex acp".to_string()),
        default_mode: None,
        default_model: None,
        fold_mode: Some(vec!["summary".to_string()]),
        tail_window: Some(3),
        // The empty list is a value of its own here (an empty visible set), so
        // it has to survive the trip as `Some([])` rather than collapse to
        // `None` — see the field's doc.
        display_filter: Some(Vec::new()),
        env: None,
    };
    let toml_str = toml::to_string(&d).expect("serialize");
    let back: AgentDefinition = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(back, d, "{toml_str}");
}

/// The catalog's real persistence boundary is [`AgentEntry`], not
/// [`AgentDefinition`] — a field reaching only the latter's wire struct would
/// still be dropped by every `[[agents]]` read and write.
#[test]
fn per_agent_transcript_defaults_round_trip_through_a_catalog_entry() {
    for entry in [
        AgentEntry::Custom(AgentDefinition {
            id: "hermes".to_string(),
            name: "Hermes".to_string(),
            launch: AgentLaunch::Raw("hermes acp".to_string()),
            default_mode: None,
            default_model: None,
            fold_mode: Some(vec!["expanded".to_string()]),
            tail_window: Some(10),
            display_filter: Some(vec!["prose".to_string()]),
            env: None,
        }),
        AgentEntry::Preset {
            preset: "codex-acp".to_string(),
            overrides: PresetOverrides {
                fold_mode: Some(vec!["summary".to_string()]),
                tail_window: Some(1),
                display_filter: Some(Vec::new()),
                ..PresetOverrides::default()
            },
        },
    ] {
        let toml_str = toml::to_string(&entry).expect("serialize");
        let back: AgentEntry = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(back, entry, "{toml_str}");
        let resolved = back.resolve().expect("both entries are runnable");
        assert_eq!(resolved.fold_mode, entry.resolve().unwrap().fold_mode);
        assert_eq!(resolved.tail_window, entry.resolve().unwrap().tail_window);
        assert_eq!(
            resolved.display_filter,
            entry.resolve().unwrap().display_filter
        );
    }
}

/// A pre-per-agent config keeps loading, and daruda must not start writing
/// empty keys back into it.
#[test]
fn a_definition_without_transcript_defaults_stays_absent() {
    let d: AgentDefinition =
        toml::from_str("id = \"codex\"\nname = \"Codex\"\ncommand = \"codex acp\"\n")
            .expect("deserialize");
    assert_eq!(d.fold_mode, None);
    assert_eq!(d.tail_window, None);
    assert_eq!(d.display_filter, None);
    let toml_str = toml::to_string(&d).expect("serialize");
    for key in ["fold_mode", "tail_window", "display_filter"] {
        assert!(!toml_str.contains(key), "{key} in {toml_str}");
    }
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
fn a_definition_without_a_default_model_round_trips_and_stays_absent() {
    // Pre-`default_model` configs must keep loading, and daruda must not
    // start writing an empty key back into them.
    let d: AgentDefinition =
        toml::from_str("id = \"codex\"\nname = \"Codex\"\ncommand = \"codex acp\"\n")
            .expect("deserialize");
    assert_eq!(d.default_model, None);
    let toml_str = toml::to_string(&d).expect("serialize");
    assert!(!toml_str.contains("default_model"), "{toml_str}");
}

#[test]
fn default_model_round_trips_alongside_a_launch_sub_table() {
    // Guards the TOML ordering constraint noted on `AgentDefinitionRepr`:
    // `default_model` is a scalar key and `ssh` is a sub-table, and TOML
    // forbids a value after a table within the same entry.
    let d = AgentDefinition {
        id: "remote-agent".to_string(),
        name: "Remote Agent".to_string(),
        launch: AgentLaunch::Ssh {
            adapter_command: "npx -y some-acp".to_string(),
            host: "vm-work".to_string(),
        },
        default_mode: None,
        default_model: Some("claude-opus-4".to_string()),
        fold_mode: None,
        tail_window: None,
        display_filter: None,
        env: None,
    };
    let toml_str = toml::to_string(&d).expect("serialize");
    let back: AgentDefinition = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(back, d, "{toml_str}");
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
        default_model: None,
        fold_mode: None,
        tail_window: None,
        display_filter: None,
        env: None,
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
        default_model: None,
        fold_mode: None,
        tail_window: None,
        display_filter: None,
        env: None,
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
    // Asserted on the script the remote shell actually receives, not on the
    // outer command text — the outer layer quotes the script as a whole.
    let script = sh_c_script(&cmd);
    assert!(
        script.contains("export CLAUDE_CONFIG_DIR='/remote/acc'"),
        "{script}"
    );
    assert!(script.contains("unset ANTHROPIC_API_KEY"), "{script}");
}

/// The script argument of a built `Ssh`/`Docker` command, recovered the way
/// the launcher gets it: tokenize the **whole** command with POSIX rules and
/// take the word after `-c`. Passing through that outer split is the point —
/// it is the first of the two layers an injected value has to survive.
fn sh_c_script(cmd: &str) -> String {
    let tokens = shell_words::split(cmd).expect("built command tokenizes as one POSIX command");
    let dash_c = tokens
        .iter()
        .position(|t| t == "-c")
        .unwrap_or_else(|| panic!("no `-c` in tokens: {tokens:?}"));
    tokens
        .get(dash_c + 1)
        .unwrap_or_else(|| panic!("`-c` carries no script in tokens: {tokens:?}"))
        .clone()
}

/// The value `name` is exported to, as the remote shell tokenizes the script
/// it received — the second layer, applied to `sh_c_script`'s output.
fn exported_value(cmd: &str, name: &str) -> String {
    let script = sh_c_script(cmd);
    let tokens = shell_words::split(&script).expect("remote shell can tokenize the script");
    let assignment = tokens
        .iter()
        .find(|t| t.starts_with(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no `{name}=...` assignment in tokens: {tokens:?}"));
    let (_, value) = assignment.split_once('=').expect("assignment has a value");
    // The emitted `export K='v'; ` has no space between the value and the
    // statement-separating `;`, so it rides along as part of this word.
    // Required, not optional: exactly one separator is always there, so
    // stripping it can never eat a `;` the value itself ends with — and if
    // the emitted form ever stops carrying it, this says so instead of
    // silently comparing the wrong string.
    value
        .strip_suffix(';')
        .unwrap_or_else(|| {
            panic!("`export {name}=…; ` must end its word with the statement separator: {value:?}")
        })
        .to_string()
}

#[test]
fn wrap_with_env_raw_preserves_a_double_quote_value_as_one_token() {
    use crate::account_env::AccountEnv;
    let launch = AgentLaunch::Raw("npx -y some-acp".to_string());
    let value = r#"{"features":{"multi_agent_v2":true}}"#;
    let env = AccountEnv {
        inject: vec![("CODEX_CONFIG".into(), value.to_string())],
        strip: vec![],
    };
    let cmd = launch.wrap_with_env(None, &env).unwrap();

    // The same tokenizer `daruda_acp::node::split_env_prefixed_tokens` and
    // `AcpAgent::from_str` both use downstream — a value that doesn't
    // survive this re-split would corrupt or split apart before the
    // adapter ever sees it.
    let tokens = shell_words::split(&cmd).expect("Raw prefix stays one shell-parseable command");
    let assignment = tokens.first().expect("env prefix token present");
    let (name, got_value) = assignment.split_once('=').expect("assignment has a value");
    assert_eq!(name, "CODEX_CONFIG");
    assert_eq!(got_value, value);
}

#[test]
fn wrap_with_env_ssh_preserves_a_double_quote_value_intact() {
    use crate::account_env::AccountEnv;
    let launch = AgentLaunch::Ssh {
        adapter_command: "npx -y some-acp".to_string(),
        host: "vm".to_string(),
    };
    let value = r#"{"features":{"multi_agent_v2":true}}"#;
    let env = AccountEnv {
        inject: vec![("CODEX_CONFIG".into(), value.to_string())],
        strip: vec![],
    };
    let cmd = launch.wrap_with_env(Some("/work"), &env).unwrap();
    assert_eq!(exported_value(&cmd, "CODEX_CONFIG"), value);
}

#[test]
fn wrap_with_env_docker_preserves_a_double_quote_value_intact() {
    use crate::account_env::AccountEnv;
    let launch = AgentLaunch::Docker {
        adapter_command: "npx -y some-acp".to_string(),
        container: "ubuntu-dev".to_string(),
    };
    let value = r#"{"features":{"multi_agent_v2":true}}"#;
    let env = AccountEnv {
        inject: vec![("CODEX_CONFIG".into(), value.to_string())],
        strip: vec![],
    };
    let cmd = launch.wrap_with_env(Some("/work"), &env).unwrap();
    assert_eq!(exported_value(&cmd, "CODEX_CONFIG"), value);
}

/// Run the `sh -c` argv a built remote command would exec, and hand back what
/// the shell printed. Drops the `ssh <host>` / `docker exec -i <container>`
/// transport prefix, which is the only part a local test cannot run.
#[cfg(unix)]
fn run_remote_script(cmd: &str) -> String {
    let tokens = shell_words::split(cmd).expect("built command tokenizes as one POSIX command");
    let sh = tokens
        .iter()
        .position(|t| t == "sh")
        .unwrap_or_else(|| panic!("no `sh` in tokens: {tokens:?}"));
    let out = std::process::Command::new(&tokens[sh])
        .args(&tokens[sh + 1..])
        .output()
        .expect("a POSIX shell runs");
    assert!(
        out.status.success(),
        "shell failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Both layers for real: POSIX-tokenize the built command, then hand the
/// `sh -c` argv to an actual shell that prints the variable back. A modelled
/// tokenizer can agree with a wrong implementation; a live shell cannot.
///
/// Both remote transports, not just `Ssh`: they share the `remote()` helper,
/// but "shares a helper today" is an implementation detail a test should not
/// assume — and the two differ in the transport prefix this has to skip.
#[cfg(unix)]
#[test]
fn wrap_with_env_remote_value_survives_a_real_shell() {
    use crate::account_env::AccountEnv;
    const ADAPTER: &str = "printf %s \"$CODEX_CONFIG\"";
    let value = r#"{"features":{"multi_agent_v2":true}} it's $HOME `pwd` \ "q""#;
    let env = AccountEnv {
        inject: vec![("CODEX_CONFIG".into(), value.to_string())],
        strip: vec![],
    };

    for launch in [
        AgentLaunch::Ssh {
            adapter_command: ADAPTER.to_string(),
            host: "vm".to_string(),
        },
        AgentLaunch::Docker {
            adapter_command: ADAPTER.to_string(),
            container: "dev-1".to_string(),
        },
    ] {
        let cmd = launch.wrap_with_env(Some("/tmp"), &env).unwrap();
        assert_eq!(run_remote_script(&cmd), value, "{cmd}");
    }
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

#[test]
fn reading_width_defaults_round_trips_and_clamps() {
    assert_eq!(AgentConfig::default().reading_width, READING_WIDTH_DEFAULT);

    let cfg: AgentConfig = toml::from_str("reading_width = 840.5").expect("deserialize");
    assert_eq!(cfg.reading_width, 840.5);
    let toml_str = toml::to_string(&cfg).expect("serialize");
    let back: AgentConfig = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(back.reading_width, 840.5);

    let mut too_low: AgentConfig = toml::from_str("reading_width = 120.0").expect("deserialize");
    too_low.clamp();
    assert_eq!(too_low.reading_width, READING_WIDTH_MIN);

    let mut too_high: AgentConfig = toml::from_str("reading_width = 5000.0").expect("deserialize");
    too_high.clamp();
    assert_eq!(too_high.reading_width, READING_WIDTH_MAX);

    let omitted: AgentConfig = toml::from_str("use_modifier_to_send = false").expect("deserialize");
    assert_eq!(omitted.reading_width, READING_WIDTH_DEFAULT);
}

#[test]
fn every_offered_tail_window_size_is_a_real_window() {
    assert!(TAIL_WINDOW_CHOICES.iter().all(|&n| n != TAIL_WINDOW_ALL));
    assert!(TAIL_WINDOW_CHOICES.windows(2).all(|w| w[0] < w[1]));
}

/// The catalog's real persistence boundary is [`AgentEntry`], so `env` is
/// asserted there as well as on the bare definition: a field reaching only
/// [`AgentDefinitionRepr`] would still be dropped by every `[[agents]]` read
/// and write.
///
/// The pairs are deliberately written in non-alphabetical order and run
/// through [`canonical_env`] — the same call every entry point into the model
/// makes. Writing them alphabetically instead would let this pass on nothing
/// but luck: `env` persists as a TOML table, so read-back is always
/// key-sorted, and a source-ordered value would come back reordered.
#[test]
fn env_round_trips_through_a_definition_and_a_catalog_entry() {
    let env = canonical_env([
        ("RUST_LOG".to_string(), "debug".to_string()),
        (
            "CODEX_CONFIG".to_string(),
            r#"{"features":{"multi_agent_v2":true}}"#.to_string(),
        ),
    ]);
    assert_eq!(
        env.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        vec!["CODEX_CONFIG", "RUST_LOG"],
        "canonicalizing is what makes the order independent of the source"
    );
    let definition = AgentDefinition {
        id: "hermes".to_string(),
        name: "Hermes".to_string(),
        launch: AgentLaunch::Raw("hermes acp".to_string()),
        default_mode: None,
        default_model: None,
        fold_mode: None,
        tail_window: None,
        display_filter: None,
        env: Some(env.clone()),
    };
    let toml_str = toml::to_string(&definition).expect("serialize");
    let back: AgentDefinition = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(back, definition, "{toml_str}");

    for entry in [
        AgentEntry::Custom(definition.clone()),
        AgentEntry::Preset {
            preset: "codex-acp".to_string(),
            overrides: PresetOverrides {
                env: Some(env.clone()),
                ..PresetOverrides::default()
            },
        },
    ] {
        let toml_str = toml::to_string(&entry).expect("serialize");
        let back: AgentEntry = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(back, entry, "{toml_str}");
        assert_eq!(
            back.resolve().expect("both entries are runnable").env,
            Some(env.clone()),
            "{toml_str}"
        );
    }
}

#[test]
fn canonical_env_is_key_sorted_and_one_value_per_name() {
    assert_eq!(
        canonical_env([
            ("ZED".to_string(), "3".to_string()),
            ("apple".to_string(), "2".to_string()),
            ("ABLE".to_string(), "1".to_string()),
        ]),
        vec![
            ("ABLE".to_string(), "1".to_string()),
            ("ZED".to_string(), "3".to_string()),
            // Byte order, not case-insensitive — the same order a TOML table
            // read-back produces.
            ("apple".to_string(), "2".to_string()),
        ]
    );
    // A repeated name collapses to one pair, last one written winning —
    // matching what the TOML table on disk can express in the first place.
    assert_eq!(
        canonical_env([
            ("K".to_string(), "first".to_string()),
            ("K".to_string(), "second".to_string()),
        ]),
        vec![("K".to_string(), "second".to_string())]
    );
    // An unusable name is refused here too: this is the same funnel the
    // `[[agents]]` load path goes through.
    assert_eq!(
        canonical_env([("BAD NAME".to_string(), "1".to_string())]),
        Vec::new()
    );
}

/// The freeze the promotion diff exists to prevent: a row that restates
/// exactly what its preset ships must come back out of a disk round trip
/// still following the preset, not frozen as an override.
///
/// Content is what has to decide. `PRESET_ENV_DEFAULTS` is canonically
/// ordered and `env` reads back key-sorted, so the `Vec` comparison in
/// `AgentEntry::reference` *is* a content comparison — this drives that end
/// to end through `Config::load_from`.
#[test]
fn a_row_restating_its_presets_environment_keeps_following_it_after_a_round_trip() {
    let preset_env = AgentDefinition::registry_preset("codex-acp")
        .expect("codex-acp is runnable")
        .env
        .expect("codex-acp ships an environment");
    let table: String = preset_env
        .iter()
        .map(|(key, value)| format!("{key} = {}\n", toml::Value::String(value.clone())))
        .collect();

    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        format!("[[agents]]\npreset = \"codex-acp\"\n\n[agents.env]\n{table}"),
    )
    .expect("write config");

    let config = crate::Config::load_from(&path);
    let row = config
        .agents
        .iter()
        .find(|entry| entry.preset_id() == Some("codex-acp"))
        .expect("the row loads as a reference");
    // Load keeps a written override verbatim — it does not diff. What it
    // resolves to is the preset's own environment either way.
    let resolved = row.resolve().expect("codex-acp resolves");
    assert_eq!(resolved.env.as_ref(), Some(&preset_env));

    // The promotion is where content has to decide: writing that same row
    // back must drop the override, or every save freezes the row against a
    // preset it actually agrees with.
    assert_eq!(
        AgentEntry::for_definition(resolved, Some("codex-acp")),
        AgentEntry::Preset {
            preset: "codex-acp".to_string(),
            overrides: PresetOverrides::default(),
        },
        "restating the preset's own environment is not an override"
    );
}

/// `env` is a TOML table, and TOML forbids a value after a table within the
/// same entry — the ordering constraint [`AgentDefinitionRepr`] already
/// documents for `ssh` / `docker`.
#[test]
fn env_round_trips_alongside_a_launch_sub_table_and_the_scalar_keys() {
    let definition = AgentDefinition {
        id: "remote-agent".to_string(),
        name: "Remote Agent".to_string(),
        launch: AgentLaunch::Ssh {
            adapter_command: "npx -y some-acp".to_string(),
            host: "vm-work".to_string(),
        },
        default_mode: Some("plan".to_string()),
        default_model: Some("claude-opus-4".to_string()),
        fold_mode: Some(vec!["summary".to_string()]),
        tail_window: Some(3),
        display_filter: Some(Vec::new()),
        env: Some(vec![("CODEX_CONFIG".to_string(), "{}".to_string())]),
    };
    let toml_str = toml::to_string(&definition).expect("serialize");
    let back: AgentDefinition = toml::from_str(&toml_str).expect("deserialize");
    assert_eq!(back, definition, "{toml_str}");
}

/// A pre-`env` config keeps loading, and daruda must not start writing an
/// empty key back into it.
#[test]
fn a_definition_without_env_stays_absent() {
    let definition: AgentDefinition =
        toml::from_str("id = \"codex\"\nname = \"Codex\"\ncommand = \"codex acp\"\n")
            .expect("deserialize");
    assert_eq!(definition.env, None);
    let toml_str = toml::to_string(&definition).expect("serialize");
    assert!(!toml_str.contains("env"), "{toml_str}");
    let entry = AgentEntry::Custom(definition);
    let toml_str = toml::to_string(&entry).expect("serialize");
    assert!(!toml_str.contains("env"), "{toml_str}");
}

/// The injection refused end to end through the real `config.toml` load
/// path: a hand-written `[agents.env]` naming a variable
/// `K; echo PWNED >&2 ; X` must not produce a remote script that runs it.
///
/// Driven from [`crate::Config::load_from`] rather than from the predicate,
/// and finished on a live `sh` rather than a modelled tokenizer — the two
/// stages that would each pass while the other is wrong.
#[cfg(unix)]
#[test]
fn a_config_env_name_that_breaks_out_of_the_remote_shell_never_reaches_the_command() {
    use crate::account_env::AccountEnv;

    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        concat!(
            "[[agents]]\n",
            "id = \"hostile\"\n",
            "name = \"Hostile\"\n",
            "command = \"printf ok\"\n",
            "\n",
            "[agents.env]\n",
            "\"K; echo PWNED >&2 ; X\" = \"1\"\n",
            "KEPT = \"2\"\n",
        ),
    )
    .expect("write config");

    let config = crate::Config::load_from(&path);
    let definition = config
        .resolved_agents()
        .into_iter()
        .find(|d| d.id == "hostile")
        .expect("the row still loads");
    // The rest of the file survived — refusing the pair must not cost the
    // user their whole config.
    assert_eq!(
        definition.env.as_deref(),
        Some(&[("KEPT".to_string(), "2".to_string())][..]),
        "only the unusable name is dropped"
    );

    let env = AccountEnv {
        inject: definition.env.clone().unwrap_or_default(),
        strip: vec![],
    };
    let cmd = assemble_launch_command(
        LaunchTransport::Ssh {
            target: "vm",
            session_path: "/tmp",
        },
        "printf ok",
        &env,
    );
    assert!(!cmd.contains("PWNED"), "{cmd}");

    // Run the `sh -c` argv the remote host would have been exec'd with.
    let tokens = shell_words::split(&cmd).expect("built command tokenizes as one POSIX command");
    let sh = tokens
        .iter()
        .position(|t| t == "sh")
        .unwrap_or_else(|| panic!("no `sh` in tokens: {tokens:?}"));
    let out = std::process::Command::new(&tokens[sh])
        .args(&tokens[sh + 1..])
        .output()
        .expect("a POSIX shell runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("PWNED"), "injected: {stderr}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "ok");
}

/// The same refusal one level down, on the entry shape a preset reference
/// persists — an `env` override reaches the identical assembly, so it cannot
/// be the one entry point that skips the check.
#[test]
fn a_preset_overrides_env_name_is_refused_on_load_too() {
    let entry: AgentEntry = toml::from_str(concat!(
        "preset = \"codex-acp\"\n",
        "\n",
        "[env]\n",
        "\"K; echo PWNED >&2 ; X\" = \"1\"\n",
        "CODEX_CONFIG = \"{}\"\n",
    ))
    .expect("the entry still loads");
    assert_eq!(
        entry,
        AgentEntry::Preset {
            preset: "codex-acp".to_string(),
            overrides: PresetOverrides {
                env: Some(vec![("CODEX_CONFIG".to_string(), "{}".to_string())]),
                ..PresetOverrides::default()
            },
        }
    );
}

#[test]
fn a_usable_env_name_is_the_posix_portable_charset() {
    for good in ["A", "_", "_x", "CODEX_CONFIG", "K1", "__a9Z"] {
        assert!(is_valid_env_name(good), "{good} must be usable");
    }
    for bad in [
        "",
        "1K",
        "9",
        "K; echo PWNED >&2 ; X",
        "MY VAR",
        "K-DASH",
        "K.DOT",
        "K$X",
        "K`pwd`",
        "K'q",
        "K\nL",
        "K=V",
        "Ké",
    ] {
        assert!(!is_valid_env_name(bad), "{bad:?} must be refused");
    }
}

/// Every environment daruda itself ships must satisfy the same rule the
/// config-load path enforces — otherwise a preset default would be silently
/// dropped the moment it round-tripped through disk.
#[test]
fn every_preset_env_default_name_is_usable() {
    for preset in agent_presets() {
        let Some(env) = preset.definition().and_then(|d| d.env) else {
            continue;
        };
        for (name, _) in &env {
            assert!(
                is_valid_env_name(name),
                "{}'s default env names {name:?}",
                preset.id
            );
        }
    }
}
