use super::*;

#[test]
fn apply_account_env_injects_and_strips() {
    use daruda_terminal::pty::PtyConfig;
    let mut cfg = PtyConfig::default();
    cfg.env.push(("ANTHROPIC_API_KEY".into(), "leak".into()));
    let env = daruda_config::account_env(
        "CLAUDE_CONFIG_DIR",
        std::path::Path::new("/data/acc/alice"),
        &["ANTHROPIC_API_KEY"],
    );
    apply_account_env(&mut cfg, &env);
    assert!(
        cfg.env
            .iter()
            .any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == "/data/acc/alice")
    );
    assert!(
        !cfg.env.iter().any(|(k, _)| k == "ANTHROPIC_API_KEY"),
        "auth override stripped"
    );
}

#[test]
fn terminal_pane_takes_every_domain() {
    assert_eq!(
        AccountDomain::for_pane(&AccountPane::Terminal),
        AccountDomain::Any
    );
}

#[test]
fn agent_chat_pane_scopes_to_its_own_adapter() {
    let claude = AccountPane::AgentChat {
        launch: Some(daruda_config::AgentLaunch::Raw(
            "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
        )),
        is_remote: false,
    };
    assert_eq!(
        AccountDomain::for_pane(&claude),
        AccountDomain::Exactly(daruda_store::accounts::AccountRecipeId::Claude)
    );

    let codex = AccountPane::AgentChat {
        launch: Some(daruda_config::AgentLaunch::Raw(
            "npx -y @agentclientprotocol/codex-acp@latest".into(),
        )),
        is_remote: false,
    };
    assert_eq!(
        AccountDomain::for_pane(&codex),
        AccountDomain::Exactly(daruda_store::accounts::AccountRecipeId::Codex)
    );
}

#[test]
fn agent_chat_pane_on_a_remote_launch_has_no_domain() {
    let remote = [
        AccountPane::AgentChat {
            launch: Some(daruda_config::AgentLaunch::Ssh {
                adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
                host: "build-box".into(),
            }),
            is_remote: true,
        },
        AccountPane::AgentChat {
            launch: Some(daruda_config::AgentLaunch::Docker {
                adapter_command: "npx -y @agentclientprotocol/codex-acp@latest".into(),
                container: "dev".into(),
            }),
            is_remote: true,
        },
        AccountPane::AgentChat {
            launch: Some(daruda_config::AgentLaunch::Raw(
                "ssh box sh -c 'cd \"{{cwd}}\" && npx -y @agentclientprotocol/claude-agent-acp@latest'"
                    .into(),
            )),
            // The `{{cwd}}` token isn't reflected in `is_remote` at all
            // (see `crate::agent::launch_resolve::account_recipe_for_connect`'s
            // doc on why `Raw` stays on `account_recipe`'s own,
            // `needs_remote_cwd()`-based exclusion) — excluded either way.
            is_remote: false,
        },
        AccountPane::AgentChat {
            launch: None,
            is_remote: false,
        },
    ];
    for pane in &remote {
        assert_eq!(AccountDomain::for_pane(pane), AccountDomain::Unsupported);
    }
}

/// Finding: a deprecated `Ssh`/`Docker` launch that `is_remote` reports
/// as resolving `Local` right now (the lane's Session Host picked
/// Local) must still offer a managed account, exactly like a `Raw`
/// launch running the same command would — the account-switcher display
/// must agree with what the actual connect now does (see
/// `crate::agent::launch_resolve::account_recipe_for_connect`).
#[test]
fn agent_chat_pane_on_a_locally_resolved_legacy_launch_still_has_a_domain() {
    let pane = AccountPane::AgentChat {
        launch: Some(daruda_config::AgentLaunch::Ssh {
            adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
            host: "old-box".into(),
        }),
        is_remote: false,
    };
    assert_eq!(
        AccountDomain::for_pane(&pane),
        AccountDomain::Exactly(daruda_store::accounts::AccountRecipeId::Claude)
    );
}

/// The case `Ssh`/`Docker`/the `{{cwd}}` token can't cover on their own:
/// a plain `Raw` command carries no host, so a pane whose *lane* is
/// remote must be excluded via `is_remote`, not the launch shape.
#[test]
fn agent_chat_pane_on_a_remote_lane_has_no_domain_even_for_a_bare_raw_launch() {
    let pane = AccountPane::AgentChat {
        launch: Some(daruda_config::AgentLaunch::Raw(
            "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
        )),
        is_remote: true,
    };
    assert_eq!(AccountDomain::for_pane(&pane), AccountDomain::Unsupported);
}

#[test]
fn resolve_system_default_ignores_the_domain_default_even_when_set() {
    use daruda_store::accounts::{AccountRecipeId, AccountSelection, AccountsState};
    let (id, st) = accounts_with(AccountRecipeId::Claude);
    let data = std::path::Path::new("/data");
    // `SystemDefault` is the explicit "System (~/.claude)" choice — it
    // must NOT fall back to the domain's default, even when one is
    // configured. (Seeding a fresh pane with that default happens
    // once, at creation time — see
    // `Workspace::default_account_selection_for_new_pane`.)
    for domain in [
        AccountDomain::Any,
        AccountDomain::Exactly(AccountRecipeId::Claude),
        AccountDomain::Exactly(AccountRecipeId::Codex),
    ] {
        assert_eq!(
            resolve_pane_account(&st, data, AccountSelection::SystemDefault, domain),
            None
        );
    }
    // An explicit `Managed(id)` still resolves normally.
    let resolved = resolve_pane_account(
        &st,
        data,
        AccountSelection::Managed(id),
        AccountDomain::Exactly(AccountRecipeId::Claude),
    )
    .expect("a Claude account under a Claude-scoped pane resolves");
    assert_eq!(
        resolved.config_dir,
        daruda_agent::accounts::account_config_dir(data, id)
    );
    assert!(
        resolved
            .env
            .inject
            .iter()
            .any(|(k, _)| k == "CLAUDE_CONFIG_DIR")
    );
    // no default set → None either way (uses system ~/.claude)
    let empty = AccountsState::default();
    assert_eq!(
        resolve_pane_account(
            &empty,
            data,
            AccountSelection::SystemDefault,
            AccountDomain::Any
        ),
        None
    );
}

#[test]
fn resolve_pane_account_refuses_every_account_when_no_domain_applies() {
    use daruda_store::accounts::{AccountRecipeId, AccountSelection};
    // A remote / JSON-stdio / unrecognized adapter can hold no managed
    // account. `Unsupported` must refuse, where "no constraint" would
    // have let every domain through.
    for recipe in AccountRecipeId::all() {
        let (id, st) = accounts_with(recipe);
        assert_eq!(
            resolve_pane_account(
                &st,
                std::path::Path::new("/data"),
                AccountSelection::Managed(id),
                AccountDomain::Unsupported
            ),
            None,
            "{recipe:?} must not reach a pane with no auth domain"
        );
    }
}

#[test]
fn resolve_pane_account_refuses_an_account_from_another_auth_domain() {
    use daruda_store::accounts::{AccountRecipeId, AccountSelection};
    let (id, st) = accounts_with(AccountRecipeId::Claude);
    let data = std::path::Path::new("/data");
    // A codex pane must never receive a Claude account's config dir.
    assert_eq!(
        resolve_pane_account(
            &st,
            data,
            AccountSelection::Managed(id),
            AccountDomain::Exactly(AccountRecipeId::Codex)
        ),
        None
    );
}

#[test]
fn resolve_pane_account_injects_the_accounts_own_env_var() {
    use daruda_store::accounts::{AccountRecipeId, AccountSelection};
    let (id, st) = accounts_with(AccountRecipeId::Codex);
    let data = std::path::Path::new("/data");
    let codex_env = |domain| {
        resolve_pane_account(&st, data, AccountSelection::Managed(id), domain)
            .expect("a Codex account resolves")
            .env
            .inject
    };
    assert!(
        codex_env(AccountDomain::Exactly(AccountRecipeId::Codex))
            .iter()
            .any(|(k, _)| k == "CODEX_HOME")
    );
    // A terminal pane passes no constraint — the account's own recipe
    // still picks the env var.
    assert!(
        codex_env(AccountDomain::Any)
            .iter()
            .any(|(k, _)| k == "CODEX_HOME")
    );
}

#[test]
fn resolve_pane_account_managed_but_deleted_resolves_to_none() {
    use daruda_store::accounts::{AccountId, AccountSelection, AccountsState};
    let empty = AccountsState::default();
    assert_eq!(
        resolve_pane_account(
            &empty,
            std::path::Path::new("/data"),
            AccountSelection::Managed(AccountId::new()),
            AccountDomain::Any
        ),
        None
    );
}

#[test]
fn focused_account_resolves_to_the_selected_account() {
    use daruda_store::accounts::{AccountRecipeId, AccountSelection};
    let (id, st) = accounts_with(AccountRecipeId::Claude);
    let data = std::path::Path::new("/data");

    let focused = resolve_focused_account(AccountSelection::Managed(id), &st, data);
    assert_eq!(focused.key(), AccountSelection::Managed(id));
    assert_eq!(
        focused.into_config_dir(),
        Some(daruda_agent::accounts::account_config_dir(data, id))
    );
}

#[test]
fn focused_account_system_default_ignores_the_domain_default() {
    use daruda_store::accounts::{AccountRecipeId, AccountSelection};
    let (_id, st) = accounts_with(AccountRecipeId::Claude);
    let data = std::path::Path::new("/data");

    // `SystemDefault` on the pane must not resolve to the domain's
    // default even though one is configured, matching
    // `resolve_pane_account`.
    assert_eq!(
        resolve_focused_account(AccountSelection::SystemDefault, &st, data),
        FocusedAccount::SystemDefault
    );
}

#[test]
fn focused_account_managed_but_deleted_collapses_to_system_default() {
    use daruda_store::accounts::{AccountId, AccountSelection, AccountsState};
    // A pane pinned to an account that no longer exists caches its usage
    // under the system slot rather than a dangling account key.
    let empty = AccountsState::default();
    let data = std::path::Path::new("/data");
    let stale = AccountId::new();
    assert_eq!(
        resolve_focused_account(AccountSelection::Managed(stale), &empty, data),
        FocusedAccount::SystemDefault
    );
}

#[test]
fn focused_account_system_default_without_managed_accounts() {
    use daruda_store::accounts::{AccountSelection, AccountsState};
    // Zero managed accounts (today's default state): system-default
    // Keychain fetch, identical to pre-account behavior.
    let empty = AccountsState::default();
    let data = std::path::Path::new("/data");
    assert_eq!(
        resolve_focused_account(AccountSelection::SystemDefault, &empty, data),
        FocusedAccount::SystemDefault
    );
}
