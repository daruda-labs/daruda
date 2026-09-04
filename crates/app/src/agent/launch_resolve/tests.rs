use super::*;
use daruda_store::project::{LaneKind, LaneStatus};

#[test]
fn connect_wire_path_unwraps_a_remote_path_verbatim() {
    // `PaneCwd::Remote` carries a `String`, `Local` a `PathBuf`.
    let cwd = PaneCwd::Remote("/srv/work".to_string());
    assert_eq!(connect_wire_path(&cwd), PathBuf::from("/srv/work"));
}

/// A minimal lane at `path` for `resolve_session_command` tests — only
/// `path` / `session_host` / `remote_cwd` matter to that function.
fn lane_at(path: &str, session_host: Option<LaneSessionHost>, remote_cwd: Option<&str>) -> Lane {
    Lane {
        id: 1,
        kind: LaneKind::Default,
        path: PathBuf::from(path),
        name: None,
        tab_order: 0,
        is_unread: false,
        last_activity: 0,
        status: LaneStatus::Idle,
        is_main: false,
        base_ref: None,
        description: None,
        remote_cwd: remote_cwd.map(str::to_string),
        session_host,
        availability: crate::lane::availability::LaneAvailability::Present,
    }
}

fn raw(command: &str) -> AgentLaunch {
    AgentLaunch::Raw(command.to_string())
}

/// Mirrors the caller's own `resolved_host` computation
/// (`connect_agent_chat`'s `owning_lane.map(|lane|
/// lane.effective_session_host(...))`), so tests can pass
/// `resolve_session_command` the same pre-resolved value it receives in
/// production instead of catalog/tombstone slices.
fn resolved(launch: &AgentLaunch, lane: Option<&Lane>) -> Option<LaneSessionHost> {
    lane.map(|lane| {
        lane.effective_session_host(launch, &[], &[])
            .expect("a form-checked host stays usable")
    })
}

/// The rev2 design's B′ regression: a restored `PaneCwd::Remote` pane
/// (a stale path with no host of its own) whose launch is a
/// host-agnostic `Raw` command must still attach to its lane's `Ssh`
/// host — not silently fall back to local because the launch no longer
/// names a host itself.
#[test]
fn restored_remote_pane_follows_the_lanes_ssh_host_not_a_local_fallback() {
    let lane = lane_at(
        "/local/checkout",
        Some(LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
            registry_id: None,
        }),
        None,
    );
    let stale_cwd = PaneCwd::Remote("stale-unrelated-path".into());
    let launch = raw("npx acp-adapter");
    let (command, connect_cwd) = resolve_session_command(
        &launch,
        Some(&lane),
        &stale_cwd,
        &AccountEnv::ambient(),
        resolved(&launch, Some(&lane)).as_ref(),
    );
    assert_eq!(
        command,
        Ok("ssh vm-work sh -c 'cd \"/srv/app\" && npx acp-adapter'".to_string())
    );
    assert_eq!(
        connect_cwd,
        PaneCwd::Remote("/srv/app".to_string()),
        "connect cwd follows the lane's current session path, not the pane's stale one"
    );
}

/// The other half of B′: the lane answering `Local` (explicitly retiring
/// a prior remote setup) must also override a stale `PaneCwd::Remote`.
#[test]
fn restored_remote_pane_follows_the_lane_back_to_local() {
    let lane = lane_at("/local/checkout", Some(LaneSessionHost::Local), None);
    let stale_cwd = PaneCwd::Remote("stale-unrelated-path".into());
    let launch = raw("npx acp-adapter");
    let (command, connect_cwd) = resolve_session_command(
        &launch,
        Some(&lane),
        &stale_cwd,
        &AccountEnv::ambient(),
        resolved(&launch, Some(&lane)).as_ref(),
    );
    assert_eq!(command, Ok("npx acp-adapter".to_string()));
    assert_eq!(
        connect_cwd,
        PaneCwd::Local(PathBuf::from("/local/checkout"))
    );
}

/// Owning lane not found (pane outlived it) — falls back to local,
/// never promoted to remote off a launch that names no host of its own.
#[test]
fn missing_owning_lane_falls_back_to_local_never_promotes_to_remote() {
    let local_cwd = PaneCwd::Local(PathBuf::from("/pane/own/local/path"));
    let (command, connect_cwd) = resolve_session_command(
        &raw("npx acp-adapter"),
        None,
        &local_cwd,
        &AccountEnv::ambient(),
        None,
    );
    assert_eq!(command, Ok("npx acp-adapter".to_string()));
    assert_eq!(
        connect_cwd,
        PaneCwd::Local(PathBuf::from("/pane/own/local/path"))
    );
}

/// The legacy combo (lane never answered `session_host`, agent-side
/// `Ssh`/`Docker` + `remote_cwd`) must keep assembling remotely, and the
/// assembled string must stay byte-identical to what `AgentLaunch::wrap`
/// alone produced before this axis moved to the lane.
#[test]
fn legacy_combo_still_assembles_remotely_byte_identical() {
    let lane = lane_at("/local/checkout", None, Some("/srv/legacy"));
    let launch = AgentLaunch::Ssh {
        adapter_command: "npx acp-adapter".into(),
        host: "old-box".into(),
    };
    let cwd = PaneCwd::Remote("/srv/legacy".into());
    let (command, connect_cwd) = resolve_session_command(
        &launch,
        Some(&lane),
        &cwd,
        &AccountEnv::ambient(),
        resolved(&launch, Some(&lane)).as_ref(),
    );
    assert_eq!(
        command,
        launch
            .wrap(Some("/srv/legacy"))
            .map_err(|()| ConnectCommandError::NoRemotePath),
        "must match AgentLaunch::wrap's own assembly for the same inputs"
    );
    assert_eq!(connect_cwd, PaneCwd::Remote("/srv/legacy".to_string()));
}

/// The legacy `Raw` + `{{cwd}}` token escape hatch predates the lane
/// session-host axis and must be untouched by it: even when the lane has
/// its own `session_host` set to something else, the token path keeps
/// resolving purely from the pane's own persisted `cwd`, exactly as
/// `AgentLaunch::wrap` already did.
#[test]
fn raw_token_escape_hatch_ignores_the_lanes_session_host() {
    let lane = lane_at(
        "/local/checkout",
        Some(LaneSessionHost::Ssh {
            target: "other-host".into(),
            session_path: "/other/path".into(),
            registry_id: None,
        }),
        None,
    );
    let launch = raw("ssh vm-work \"cd {{cwd}} && run\"");
    let cwd = PaneCwd::Remote("/home/user/project".into());
    let (command, connect_cwd) = resolve_session_command(
        &launch,
        Some(&lane),
        &cwd,
        &AccountEnv::ambient(),
        resolved(&launch, Some(&lane)).as_ref(),
    );
    assert_eq!(
        command,
        Ok("ssh vm-work \"cd /home/user/project && run\"".to_string())
    );
    assert_eq!(
        connect_cwd,
        PaneCwd::Remote("/home/user/project".to_string())
    );
}

fn json_stdio_launch() -> AgentLaunch {
    AgentLaunch::Raw(r#"{"command":"/opt/adapters/claude-agent-acp","args":[]}"#.to_string())
}

/// A JSON stdio config can't be folded into `ssh`/`docker` — `wrap` only
/// knows how to splice a shell command line in there, and raw JSON isn't
/// one. Must fail with the dedicated reason, not silently produce a
/// broken command.
#[test]
fn json_stdio_launch_on_a_remote_lane_is_rejected() {
    let lane = lane_at(
        "/local/checkout",
        Some(LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
            registry_id: None,
        }),
        None,
    );
    let cwd = PaneCwd::Local(PathBuf::from("/local/checkout"));
    let launch = json_stdio_launch();
    let (command, resolved_cwd) = resolve_session_command(
        &launch,
        Some(&lane),
        &cwd,
        &AccountEnv::ambient(),
        resolved(&launch, Some(&lane)).as_ref(),
    );
    assert_eq!(command, Err(ConnectCommandError::JsonStdioRemote));
    // The resolved cwd is still reported (used to sync the pane even on
    // a rejected connect), even though the command itself failed.
    assert_eq!(resolved_cwd, PaneCwd::Remote("/srv/app".to_string()));
}

/// The same JSON stdio config on a lane that resolves `Local`, with no
/// environment to fold in, still passes through unchanged — `wrap`
/// returns a `Local` command as-is and `daruda_acp` parses it exactly as
/// it always has.
#[test]
fn json_stdio_launch_on_a_local_lane_with_no_env_is_unaffected() {
    let lane = lane_at("/local/checkout", Some(LaneSessionHost::Local), None);
    let cwd = PaneCwd::Local(PathBuf::from("/local/checkout"));
    let launch = json_stdio_launch();
    let (command, resolved_cwd) = resolve_session_command(
        &launch,
        Some(&lane),
        &cwd,
        &AccountEnv::ambient(),
        resolved(&launch, Some(&lane)).as_ref(),
    );
    assert_eq!(
        command,
        Ok(r#"{"command":"/opt/adapters/claude-agent-acp","args":[]}"#.to_string())
    );
    assert_eq!(
        resolved_cwd,
        PaneCwd::Local(PathBuf::from("/local/checkout"))
    );
}

/// An environment on that same local JSON stdio launch would prepend a shell
/// assignment in front of the JSON, which both downstream discriminators then
/// read as a shell command line whose program is named `{command:…}`. Refused
/// instead.
///
/// Asserted through `resolve_launch` — the real entry point, which is
/// what turns an `AgentDefinition`'s declared `env` into the
/// `AccountEnv` this gate sees.
#[test]
fn json_stdio_launch_on_a_local_lane_with_a_declared_env_is_rejected() {
    let lane = lane_at("/local/checkout", Some(LaneSessionHost::Local), None);
    let launch = json_stdio_launch();
    let resolved = resolve_launch(
        &launch,
        &declared(&[("CODEX_CONFIG", r#"{"features":{"multi_agent_v2":true}}"#)]),
        Some(&lane),
        &PaneCwd::Local(PathBuf::from("/local/checkout")),
        None,
        None,
        Some(&LaneSessionHost::Local),
        &[],
        &[],
    );
    assert_eq!(resolved.spec, Err(ConnectCommandError::JsonStdioEnv));
}

/// The `{{cwd}}`-token escape hatch reaches the same assembly one branch
/// earlier, so it needs the same gate: a JSON config whose `args` carry
/// the token is still a JSON config.
#[test]
fn a_json_stdio_cwd_token_launch_with_a_declared_env_is_rejected() {
    let launch = raw(r#"{"command":"/opt/adapters/x","args":["--cwd","{{cwd}}"]}"#);
    let env = AccountEnv {
        inject: vec![("CODEX_CONFIG".into(), "{}".into())],
        strip: vec![],
    };
    let (command, _) = resolve_session_command(
        &launch,
        None,
        &PaneCwd::Remote("/srv/app".into()),
        &env,
        None,
    );
    assert_eq!(command, Err(ConnectCommandError::JsonStdioEnv));

    // Without an environment it still resolves, token substituted.
    let (command, _) = resolve_session_command(
        &launch,
        None,
        &PaneCwd::Remote("/srv/app".into()),
        &AccountEnv::ambient(),
        None,
    );
    assert_eq!(
        command,
        Ok(r#"{"command":"/opt/adapters/x","args":["--cwd","/srv/app"]}"#.to_string())
    );
}

/// A remote lane keeps reporting the remote reason even when an
/// environment is also declared: "this cannot run remotely at all" is the
/// blocker the user has to clear first.
#[test]
fn a_remote_json_stdio_launch_reports_the_remote_reason_not_the_env_one() {
    let lane = lane_at(
        "/local/checkout",
        Some(LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
            registry_id: None,
        }),
        None,
    );
    let launch = json_stdio_launch();
    let env = AccountEnv {
        inject: vec![("CODEX_CONFIG".into(), "{}".into())],
        strip: vec![],
    };
    let (command, _) = resolve_session_command(
        &launch,
        Some(&lane),
        &PaneCwd::Local(PathBuf::from("/local/checkout")),
        &env,
        resolved(&launch, Some(&lane)).as_ref(),
    );
    assert_eq!(command, Err(ConnectCommandError::JsonStdioRemote));
}

/// Finding: a deprecated `Ssh`-typed launch that a lane now resolves to
/// `Local` (the user picked Local in the Session Host dialog, retiring
/// the launch's own embedded host) must still be eligible for a managed
/// account, exactly like a `Raw` launch running the same command would.
#[test]
fn account_recipe_for_connect_recognizes_a_locally_resolved_ssh_launch() {
    let launch = AgentLaunch::Ssh {
        adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
        host: "old-box".into(),
    };
    assert_eq!(
        account_recipe_for_connect(&launch, false),
        Some(daruda_store::accounts::AccountRecipeId::Claude)
    );
    // Still excluded once it's actually remote.
    assert_eq!(account_recipe_for_connect(&launch, true), None);
}

/// Same for `Docker`.
#[test]
fn account_recipe_for_connect_recognizes_a_locally_resolved_docker_launch() {
    let launch = AgentLaunch::Docker {
        adapter_command: "npx -y @agentclientprotocol/codex-acp@latest".into(),
        container: "dev-1".into(),
    };
    assert_eq!(
        account_recipe_for_connect(&launch, false),
        Some(daruda_store::accounts::AccountRecipeId::Codex)
    );
}

/// A locally-resolved `Ssh`/`Docker` launch whose command is a JSON
/// stdio config is still excluded — the shape that bars a managed
/// account for `Raw` bars it here too.
#[test]
fn account_recipe_for_connect_still_excludes_json_stdio() {
    let launch = AgentLaunch::Ssh {
        adapter_command: r#"{"command":"x"}"#.into(),
        host: "old-box".into(),
    };
    assert_eq!(account_recipe_for_connect(&launch, false), None);
}

/// A plain `Raw` launch is unaffected by the new match arm — behaves
/// exactly as `account_recipe` itself already does.
#[test]
fn account_recipe_for_connect_matches_account_recipe_for_raw_launches() {
    let launch = raw("npx -y @agentclientprotocol/claude-agent-acp@latest");
    assert_eq!(
        account_recipe_for_connect(&launch, false),
        launch.account_recipe(false)
    );
    assert_eq!(
        account_recipe_for_connect(&launch, true),
        launch.account_recipe(true)
    );
}

#[test]
fn env_is_folded_in_for_the_lane_resolved_path() {
    let lane = lane_at(
        "/local/checkout",
        Some(LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
            registry_id: None,
        }),
        None,
    );
    let env = AccountEnv {
        inject: vec![("CLAUDE_CONFIG_DIR".into(), "/remote/acc".into())],
        strip: vec!["ANTHROPIC_API_KEY"],
    };
    let cwd = PaneCwd::Local(PathBuf::from("/unused"));
    let launch = raw("npx acp-adapter");
    let (command, _) = resolve_session_command(
        &launch,
        Some(&lane),
        &cwd,
        &env,
        resolved(&launch, Some(&lane)).as_ref(),
    );
    let command = command.unwrap();
    // Asserted on the script the remote shell actually receives: the
    // outer layer quotes it as a whole, so the inner `'…'` shows up
    // escaped in the command text.
    let tokens = shell_words::split(&command).expect("tokenizes as one POSIX command");
    let script = tokens
        .iter()
        .position(|t| t == "-c")
        .and_then(|i| tokens.get(i + 1))
        .unwrap_or_else(|| panic!("no `-c <script>` in {tokens:?}"));
    assert!(
        script.contains("export CLAUDE_CONFIG_DIR='/remote/acc'"),
        "{script}"
    );
    assert!(script.contains("unset ANTHROPIC_API_KEY"), "{script}");
}

/// The path a `codex-acp` pane actually takes: a non-`{{cwd}}` `Raw`
/// launch, which `resolve_session_command` routes through
/// `session_host::wrap_with_env` — not `AgentLaunch::wrap_with_env`. A
/// value holding `"`, `'`, `$`, a backtick and `\` must come back out of
/// the downstream re-tokenization (`AcpAgent::from_str` /
/// `daruda_acp::node::split_env_prefixed_tokens`) byte-identical.
#[test]
fn a_hostile_env_value_survives_the_non_token_raw_path() {
    let value = r#"{"features":{"multi_agent_v2":true}} it's $HOME `pwd` \ "q""#;
    let env = AccountEnv {
        inject: vec![("CODEX_CONFIG".into(), value.to_string())],
        strip: vec![],
    };
    let lane = lane_at("/local/checkout", Some(LaneSessionHost::Local), None);
    let launch = raw("npx -y @agentclientprotocol/codex-acp@latest");
    let (command, _) = resolve_session_command(
        &launch,
        Some(&lane),
        &PaneCwd::Local(PathBuf::from("/unused")),
        &env,
        resolved(&launch, Some(&lane)).as_ref(),
    );
    let command = command.unwrap();
    let tokens = shell_words::split(&command).expect("stays one shell-parseable command");
    let (name, got) = tokens
        .first()
        .expect("env prefix token present")
        .split_once('=')
        .expect("assignment has a value");
    assert_eq!(name, "CODEX_CONFIG");
    assert_eq!(got, value);
    // Asserted on the real consumer rather than on a token index: the
    // index is a proxy that stays green for a prefix `command_needs_node`
    // cannot actually parse (an unusable env name, for one, makes the bad
    // token the launcher instead).
    assert!(
        daruda_acp::node::command_needs_node(&command),
        "Node provisioning must still see the launcher: {command}"
    );
}

#[test]
fn account_strip_env_carries_the_recipes_auth_overrides() {
    use crate::agent::account::PreparedAccount;
    use daruda_store::accounts::AccountRecipeId;
    use std::path::PathBuf;

    let recipe = daruda_agent::accounts::recipe_for(AccountRecipeId::Claude);
    let config_dir = PathBuf::from("/data/acc/alice");
    let prepared = PreparedAccount {
        recipe: AccountRecipeId::Claude,
        env: daruda_config::account_env(recipe.config_dir_env(), &config_dir, recipe.strip_env()),
        config_dir,
    };
    let strip = account_strip_env(Some(&prepared));
    assert_eq!(strip, recipe.strip_env());
    assert!(strip.iter().any(|n| n == "ANTHROPIC_API_KEY"));

    // Codex strips nothing, and the System account (no managed account)
    // runs under the user's own environment by definition.
    let codex = daruda_agent::accounts::recipe_for(AccountRecipeId::Codex);
    assert!(codex.strip_env().is_empty());
    assert!(account_strip_env(None).is_empty());
}

// Task 3: connect-path write-back — a registry `target`/`container` edit,
// or a tombstone redirect, syncs back onto the lane's cached
// `session_host` so a future connect (and any UI reading it) sees the
// fresh value instead of re-deriving it every time.

fn write_back_tombstone(
    old_id: daruda_store::project::SessionHostId,
    redirected_to: Option<daruda_store::project::SessionHostId>,
) -> daruda_config::SessionHostTombstone {
    daruda_config::SessionHostTombstone {
        old_id,
        kind: daruda_config::SessionHostKind::Ssh {
            target: "old-box".into(),
        },
        value: "old-box".into(),
        removed_at: 0,
        redirected_to,
    }
}

/// The Step 1 scenario: a catalog `target` edit surfaces through
/// `effective_session_host` already (Task 2) — this asserts the
/// write-back actually persists it onto the lane's cache.
#[test]
fn catalog_target_change_writes_back_the_lanes_cached_target() {
    let id = daruda_store::project::SessionHostId::new();
    let cached = LaneSessionHost::Ssh {
        target: "old-target".into(),
        session_path: "/srv/app".into(),
        registry_id: Some(id),
    };
    let catalog = vec![daruda_config::SessionHostEntry {
        id,
        label: "Build box".into(),
        kind: daruda_config::SessionHostKind::Ssh {
            target: "new-target".into(),
        },
    }];
    // Task 2's resolver already folded the catalog's current target
    // in — this is what `connect_agent_chat` would pass as `resolved`.
    let resolved = LaneSessionHost::Ssh {
        target: "new-target".into(),
        session_path: "/srv/app".into(),
        registry_id: Some(id),
    };
    let written = session_host_write_back(Some(&cached), Some(&resolved), &catalog, &[])
        .expect("target changed — must write back");
    assert_eq!(
        written,
        LaneSessionHost::Ssh {
            target: "new-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(id),
        }
    );
}

/// A tombstone redirect also corrects the cached `registry_id` to the
/// live id — not just `target` — so the next connect resolves directly
/// against the catalog instead of re-chasing the tombstone.
#[test]
fn tombstone_redirect_writes_back_both_target_and_registry_id() {
    let old_id = daruda_store::project::SessionHostId::new();
    let new_id = daruda_store::project::SessionHostId::new();
    let cached = LaneSessionHost::Ssh {
        target: "old-box".into(),
        session_path: "/srv/app".into(),
        registry_id: Some(old_id),
    };
    let catalog = vec![daruda_config::SessionHostEntry {
        id: new_id,
        label: "Renamed box".into(),
        kind: daruda_config::SessionHostKind::Ssh {
            target: "merged-target".into(),
        },
    }];
    let tombstones = vec![write_back_tombstone(old_id, Some(new_id))];
    // `effective_session_host` (Task 2) resolves the target through the
    // redirect but leaves `registry_id` at the stale cached id — exactly
    // what `connect_agent_chat` would pass here.
    let resolved = LaneSessionHost::Ssh {
        target: "merged-target".into(),
        session_path: "/srv/app".into(),
        registry_id: Some(old_id),
    };
    let written = session_host_write_back(Some(&cached), Some(&resolved), &catalog, &tombstones)
        .expect("redirect resolved a new value — must write back");
    assert_eq!(
        written,
        LaneSessionHost::Ssh {
            target: "merged-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(new_id),
        }
    );
}

/// No drift → no write, so a connect that changes nothing doesn't
/// spuriously dirty + persist the workspace on every single connect.
#[test]
fn unchanged_value_writes_back_nothing() {
    let id = daruda_store::project::SessionHostId::new();
    let host = LaneSessionHost::Ssh {
        target: "vm-work".into(),
        session_path: "/srv/app".into(),
        registry_id: Some(id),
    };
    let catalog = vec![daruda_config::SessionHostEntry {
        id,
        label: "Build box".into(),
        kind: daruda_config::SessionHostKind::Ssh {
            target: "vm-work".into(),
        },
    }];
    assert_eq!(
        session_host_write_back(Some(&host), Some(&host), &catalog, &[]),
        None
    );
}

/// An unanswered lane (`session_host: None`) must never be silently
/// promoted to an explicit answer by this sync — that's a distinct,
/// user-driven action (`Lane::set_session_host`'s doc), not something a
/// background resolve should do on its own.
#[test]
fn unanswered_lane_is_never_written_to() {
    let resolved = LaneSessionHost::Ssh {
        target: "vm-work".into(),
        session_path: "/srv/app".into(),
        registry_id: None,
    };
    assert_eq!(
        session_host_write_back(None, Some(&resolved), &[], &[]),
        None
    );
}

/// No owning lane at all (pane with no resolvable lane) — nothing to
/// write back to.
#[test]
fn missing_owning_lane_writes_back_nothing() {
    let cached = LaneSessionHost::Local;
    assert_eq!(session_host_write_back(Some(&cached), None, &[], &[]), None);
}

/// `sync_registry_id` itself: a `Local` host has no registry link to
/// correct — passed through unchanged.
#[test]
fn sync_registry_id_is_a_noop_for_local() {
    assert_eq!(
        sync_registry_id(LaneSessionHost::Local, &[], &[]),
        LaneSessionHost::Local
    );
}

/// An orphaned link (deleted, no redirect) keeps its stale id — there is
/// nothing live to correct it to.
#[test]
fn sync_registry_id_keeps_an_orphaned_id() {
    let id = daruda_store::project::SessionHostId::new();
    let host = LaneSessionHost::Docker {
        container: "dev-1".into(),
        session_path: "/workspace".into(),
        registry_id: Some(id),
    };
    assert_eq!(sync_registry_id(host.clone(), &[], &[]), host);
}

/// The single entry point answers all four questions at once: what command
/// to run, where it is rooted, what path the wire protocol gets, and
/// whether the lane owes a session-host correction.
#[test]
fn resolve_launch_bundles_command_cwd_and_strip_env() {
    use daruda_store::accounts::AccountRecipeId;

    let launch = raw("npx acp-adapter");
    let local = PaneCwd::Local(PathBuf::from("/repo"));

    // Same construction the moved `account_strip_env` test uses, so the
    // strip list below is the recipe's real one rather than a stub.
    let recipe = daruda_agent::accounts::recipe_for(AccountRecipeId::Claude);
    let config_dir = PathBuf::from("/data/acc/alice");
    let prepared = PreparedAccount {
        recipe: AccountRecipeId::Claude,
        env: daruda_config::account_env(recipe.config_dir_env(), &config_dir, recipe.strip_env()),
        config_dir,
    };

    let resolved = resolve_launch(
        &launch,
        &[],
        None,
        &local,
        Some(&prepared),
        None,
        None,
        &[],
        &[],
    );

    let spec = resolved.spec.expect("a local raw launch always resolves");
    // `wrap_with_env` prefixes a `Local` command with an inject-only
    // `KEY='value' ` segment (see `wrap_with_env_local_matches_the_shipped_assembly`);
    // the bare adapter command still trails it verbatim.
    assert_eq!(
        spec.command,
        "CLAUDE_CONFIG_DIR='/data/acc/alice' npx acp-adapter"
    );
    assert!(spec.strip_env.iter().any(|n| n == "ANTHROPIC_API_KEY"));
    assert_eq!(resolved.wire_cwd, PathBuf::from("/repo"));
    assert_eq!(resolved.host_write_back, None);
}

/// `cached_host` and `resolved_host` are adjacent same-typed parameters
/// consumed by two different callees inside `resolve_launch`
/// (`resolve_session_command` takes only `resolved_host`;
/// `session_host_write_back` takes both, in `(cached, resolved)` order).
/// A `None, None` fixture can't tell a correct wiring from a swapped one
/// — both are vacuous. Here `cached` and `resolved` differ, so a swap
/// would flip every assertion below: the command would wrap for `Local`
/// (the cached host) instead of the freshly-resolved `Ssh`, and the
/// write-back would report no drift instead of the real one.
#[test]
fn resolve_launch_uses_resolved_host_for_command_and_cached_for_write_back_baseline() {
    let cached = LaneSessionHost::Local;
    let resolved_host = LaneSessionHost::Ssh {
        target: "vm".into(),
        session_path: "/srv/app".into(),
        registry_id: None,
    };
    let launch = raw("npx acp-adapter");
    let local = PaneCwd::Local(PathBuf::from("/repo"));

    let r = resolve_launch(
        &launch,
        &[],
        None,
        &local,
        None,
        Some(&cached),
        Some(&resolved_host),
        &[],
        &[],
    );

    // Command assembly follows `resolved_host` (the freshly-resolved
    // `Ssh`), not `cached` (`Local`) — byte-identical to
    // `session_host::wrap` for the same `(host, adapter_command)` pair.
    let spec = r.spec.expect("an ssh-resolved host still assembles");
    assert_eq!(
        spec.command,
        "ssh vm sh -c 'cd \"/srv/app\" && npx acp-adapter'"
    );
    assert_eq!(r.resolved_cwd, PaneCwd::Remote("/srv/app".to_string()));
    assert_eq!(r.wire_cwd, PathBuf::from("/srv/app"));
    // `cached` (`Local`) differs from the corrected `resolved_host`, so
    // the write-back fires with the resolved value.
    assert_eq!(r.host_write_back, Some(resolved_host));
}

fn declared(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn managed_account(env_name: &str, dir: &str, strip: &[&'static str]) -> AccountEnv {
    AccountEnv {
        inject: vec![(env_name.to_string(), dir.to_string())],
        strip: strip.to_vec(),
    }
}

#[test]
fn launch_env_keeps_an_agent_only_declaration_in_declared_order() {
    let agent = declared(&[("CODEX_CONFIG", "{}"), ("RUST_LOG", "debug")]);
    let merged = launch_env(&agent, None);
    assert_eq!(merged.inject, agent);
    assert!(merged.strip.is_empty());
}

/// No agent env leaves whatever the account said, and nothing else — with no
/// account at all, the ambient environment.
#[test]
fn launch_env_passes_an_account_only_environment_through_unchanged() {
    let account = managed_account(
        "CLAUDE_CONFIG_DIR",
        "/data/acc/alice",
        &["ANTHROPIC_API_KEY"],
    );
    assert_eq!(launch_env(&[], Some(&account)), account);
    assert_eq!(launch_env(&[], None), AccountEnv::ambient());
}

/// The precedence rule: the account's vars scope credentials, so an agent
/// definition cannot redirect one. Deduplicated here — exactly one entry
/// survives for the contested key — rather than left to a shell's
/// last-assignment-wins.
#[test]
fn launch_env_lets_the_account_win_a_key_collision() {
    let agent = declared(&[("CLAUDE_CONFIG_DIR", "/agent/dir"), ("CODEX_CONFIG", "{}")]);
    let account = managed_account("CLAUDE_CONFIG_DIR", "/data/acc/alice", &[]);
    let merged = launch_env(&agent, Some(&account));
    assert_eq!(
        merged.inject,
        vec![
            ("CODEX_CONFIG".to_string(), "{}".to_string()),
            (
                "CLAUDE_CONFIG_DIR".to_string(),
                "/data/acc/alice".to_string()
            ),
        ]
    );
}

/// The other half of the same rule: a var the account strips is one it
/// scopes too, so an agent cannot inject it back.
#[test]
fn launch_env_drops_an_agent_key_the_account_strips() {
    let agent = declared(&[("ANTHROPIC_API_KEY", "sk-agent"), ("CODEX_CONFIG", "{}")]);
    let account = managed_account(
        "CLAUDE_CONFIG_DIR",
        "/data/acc/alice",
        &["ANTHROPIC_API_KEY"],
    );
    let merged = launch_env(&agent, Some(&account));
    assert!(
        !merged.inject.iter().any(|(k, _)| k == "ANTHROPIC_API_KEY"),
        "{merged:?}"
    );
    assert_eq!(merged.strip, vec!["ANTHROPIC_API_KEY"]);
}

/// The leading `KEY=value` assignments of an assembled `Local` command,
/// recovered the way the launcher itself reads them
/// (`daruda_acp::node::split_env_prefixed_tokens` re-splits this string).
fn local_assignments(command: &str) -> Vec<(String, String)> {
    shell_words::split(command)
        .expect("a Local command stays one POSIX command")
        .iter()
        .map_while(|token| {
            let (key, value) = token.split_once('=')?;
            (!key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                .then(|| (key.to_string(), value.to_string()))
        })
        .collect()
}

/// The value `name` is assigned in a remote `sh -c` script, through both
/// quoting layers: the outer tokenization the launcher does, then the
/// remote shell's own split of the script it received.
fn exported(command: &str, name: &str) -> Option<String> {
    let tokens = shell_words::split(command).expect("the outer command tokenizes");
    let script = tokens
        .iter()
        .position(|t| t == "-c")
        .and_then(|i| tokens.get(i + 1))
        .unwrap_or_else(|| panic!("no `-c <script>` in {tokens:?}"));
    shell_words::split(script)
        .expect("the remote shell tokenizes the script")
        .iter()
        .find_map(|token| {
            let value = token.strip_prefix(&format!("{name}="))?;
            Some(value.strip_suffix(';').unwrap_or(value).to_string())
        })
}

fn claude_account() -> PreparedAccount {
    use daruda_store::accounts::AccountRecipeId;
    let recipe = daruda_agent::accounts::recipe_for(AccountRecipeId::Claude);
    let config_dir = PathBuf::from("/data/acc/alice");
    PreparedAccount {
        recipe: AccountRecipeId::Claude,
        env: daruda_config::account_env(recipe.config_dir_env(), &config_dir, recipe.strip_env()),
        config_dir,
    }
}

/// The merge has to reach the command the adapter actually launches with,
/// not just the helper: asserted against `resolve_launch`'s real output,
/// re-tokenized rather than compared to a hand-written string.
#[test]
fn an_agents_declared_env_reaches_the_assembled_local_command() {
    let value = r#"{"features":{"multi_agent_v2":true}}"#;
    let launch = raw("npx -y @agentclientprotocol/codex-acp@latest");
    let resolved = resolve_launch(
        &launch,
        &declared(&[("CODEX_CONFIG", value)]),
        None,
        &PaneCwd::Local(PathBuf::from("/repo")),
        Some(&claude_account()),
        None,
        None,
        &[],
        &[],
    );
    let spec = resolved.spec.expect("a local raw launch always resolves");
    assert_eq!(
        local_assignments(&spec.command),
        vec![
            ("CODEX_CONFIG".to_string(), value.to_string()),
            (
                "CLAUDE_CONFIG_DIR".to_string(),
                "/data/acc/alice".to_string()
            ),
        ],
        "{}",
        spec.command
    );
    // Asserted on the real consumer rather than on a token index — see
    // `a_hostile_env_value_survives_the_non_token_raw_path`.
    assert!(
        daruda_acp::node::command_needs_node(&spec.command),
        "Node provisioning must still see the launcher: {}",
        spec.command
    );
}

/// Same merge, same precedence, over the transport that emits `export`
/// statements instead of a command prefix.
#[test]
fn an_agents_declared_env_reaches_the_assembled_remote_command() {
    let host = LaneSessionHost::Ssh {
        target: "vm".into(),
        session_path: "/srv/app".into(),
        registry_id: None,
    };
    let launch = raw("npx -y @agentclientprotocol/codex-acp@latest");
    let resolved = resolve_launch(
        &launch,
        &declared(&[
            ("CODEX_CONFIG", "{\"features\":{}}"),
            ("CLAUDE_CONFIG_DIR", "/agent/dir"),
        ]),
        None,
        &PaneCwd::Local(PathBuf::from("/repo")),
        Some(&claude_account()),
        None,
        Some(&host),
        &[],
        &[],
    );
    let command = resolved
        .spec
        .expect("an ssh-resolved host still assembles")
        .command;
    assert_eq!(
        exported(&command, "CODEX_CONFIG").as_deref(),
        Some("{\"features\":{}}"),
        "{command}"
    );
    // The account wins the collision on this transport too — the rule
    // lives in the merge, not in either shell's assignment order.
    assert_eq!(
        exported(&command, "CLAUDE_CONFIG_DIR").as_deref(),
        Some("/data/acc/alice"),
        "{command}"
    );
}

/// An agent that declares nothing and a pane with no managed account
/// still assemble the bare command, byte for byte.
#[test]
fn no_declared_env_and_no_account_assembles_the_bare_command() {
    let launch = raw("npx acp-adapter");
    let local = resolve_launch(
        &launch,
        &[],
        None,
        &PaneCwd::Local(PathBuf::from("/repo")),
        None,
        None,
        None,
        &[],
        &[],
    );
    assert_eq!(local.spec.expect("resolves").command, "npx acp-adapter");

    let host = LaneSessionHost::Ssh {
        target: "vm".into(),
        session_path: "/srv/app".into(),
        registry_id: None,
    };
    let remote = resolve_launch(
        &launch,
        &[],
        None,
        &PaneCwd::Local(PathBuf::from("/repo")),
        None,
        None,
        Some(&host),
        &[],
        &[],
    );
    assert_eq!(
        remote.spec.expect("resolves").command,
        "ssh vm sh -c 'cd \"/srv/app\" && npx acp-adapter'"
    );
}

/// Pinned across a seam two other tests each cover only one side of:
/// `preset.rs` asserts the preset carries the overlay, and the tests above
/// assert a *declared* environment reaches the command — neither notices if
/// the preset's own value stops flowing between them. Starts from the
/// shipped definition, ends at the string the adapter is launched with.
#[test]
fn the_shipped_codex_row_launches_with_the_subagent_overlay() {
    let definition = daruda_config::AgentDefinition::codex_default();
    let env = definition.env.clone().expect("the preset states one");
    let resolved = resolve_launch(
        &definition.launch,
        &env,
        None,
        &PaneCwd::Local(PathBuf::from("/repo")),
        None,
        None,
        None,
        &[],
        &[],
    );
    let command = resolved.spec.expect("a local codex row resolves").command;
    assert_eq!(
        local_assignments(&command)
            .into_iter()
            .find(|(key, _)| key == daruda_config::CODEX_CONFIG_ENV)
            .map(|(_, value)| value),
        Some(r#"{"features":{"multi_agent_v2":true}}"#.to_string()),
        "{command}"
    );
    // The adapter still leads the line once the assignments are stripped —
    // Node.js provisioning reads that token.
    assert!(daruda_acp::node::command_needs_node(&command), "{command}");
}
