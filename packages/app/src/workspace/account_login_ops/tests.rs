use super::*;

#[test]
fn can_start_login_none_is_startable() {
    assert!(can_start_login(&PendingLogin::None));
}

#[test]
fn can_start_login_preparing_is_not_startable() {
    let pending = PendingLogin::Preparing {
        target: LoginTarget::Managed {
            id: AccountId::new(),
            recipe: AccountRecipeId::Claude,
        },
        attempt: next_login_attempt(),
        finish: LoginFinish::Add,
        cancellation: Default::default(),
    };
    assert!(!can_start_login(&pending));
}

mod attempt_identity {
    use super::*;

    fn in_progress(target: LoginTarget, attempt: LoginAttempt) -> PendingLogin {
        let login = spawn_login(
            &daruda_agent::accounts::CliCommand {
                program: test_process::executable().to_path_buf(),
                args: Vec::new(),
                env: Vec::new(),
            },
            Duration::from_secs(5),
        )
        .expect("spawn a trivial process for the test handle");
        PendingLogin::InProgress {
            target,
            attempt,
            ambient_before: None,
            handle: login.handle(),
            finish: LoginFinish::System,
        }
    }

    /// The race a restart opens. Taking over an attempt leaves its
    /// background wait still running, and it resolves against whatever is
    /// pending *then* — which is the replacement, for the same target. Keyed
    /// on the target alone the stale wait claims the new attempt's slot,
    /// clears it, and reports the failure of a login the user already
    /// abandoned; the attempt they are waiting on is then orphaned and dies
    /// silently.
    #[test]
    fn a_replaced_attempt_does_not_claim_its_replacement() {
        let target = LoginTarget::System {
            recipe: AccountRecipeId::Claude,
        };
        let stale = next_login_attempt();
        let live = next_login_attempt();
        assert_ne!(stale, live, "each attempt is its own");

        let pending = in_progress(target, live);
        assert!(!claims_pending(&pending, stale));
        assert!(claims_pending(&pending, live));
    }

    /// Nothing pending is nothing to claim — the ordinary
    /// already-cancelled case.
    #[test]
    fn nothing_pending_is_claimed_by_nobody() {
        assert!(!claims_pending(&PendingLogin::None, next_login_attempt()));
    }

    /// A `Preparing` attempt has not produced a process yet, so a finish
    /// callback cannot belong to it.
    #[test]
    fn a_preparing_attempt_is_not_a_finished_one() {
        let attempt = next_login_attempt();
        let pending = PendingLogin::Preparing {
            target: LoginTarget::System {
                recipe: AccountRecipeId::Claude,
            },
            attempt,
            finish: LoginFinish::System,
            cancellation: Default::default(),
        };
        assert!(!claims_pending(&pending, attempt));
    }
}

/// Stopping an attempt still preparing interrupts the install it is
/// waiting on, rather than leaving a download running for nobody.
#[test]
fn stopping_a_preparing_attempt_cancels_its_preparation() {
    let cancellation = daruda_acp::preparation::PreparationCancellation::default();
    let target = LoginTarget::System {
        recipe: AccountRecipeId::Claude,
    };
    let stopped = stop_pending(PendingLogin::Preparing {
        target,
        attempt: next_login_attempt(),
        finish: LoginFinish::System,
        cancellation: cancellation.clone(),
    });
    assert_eq!(stopped.map(|(t, _, _)| t), Some(target));
    assert!(cancellation.is_canceled());
    assert!(stop_pending(PendingLogin::None).is_none());
}

mod reclick {
    use super::*;

    fn system(recipe: AccountRecipeId) -> LoginTarget {
        LoginTarget::System { recipe }
    }

    fn preparing(target: LoginTarget) -> PendingLogin {
        PendingLogin::Preparing {
            target,
            attempt: next_login_attempt(),
            finish: LoginFinish::System,
            cancellation: Default::default(),
        }
    }

    /// The report this exists for: sign in, a browser opens, the user
    /// closes it without finishing. The login process outlives that — its
    /// callback server waits on stdin — so the slot stays taken for the
    /// whole login timeout. Clicking sign-in again is the user saying that
    /// attempt is not going to finish, not asking for a second one.
    #[test]
    fn clicking_sign_in_again_takes_over_the_attempt_it_repeats() {
        let target = system(AccountRecipeId::Claude);
        assert!(reclick_restarts(&preparing(target), target));
    }

    /// A login for something else is not the user's to discard — the slot
    /// is single, so this still has to be refused.
    #[test]
    fn a_login_for_another_target_is_not_taken_over() {
        assert!(!reclick_restarts(
            &preparing(system(AccountRecipeId::Codex)),
            system(AccountRecipeId::Claude)
        ));
        assert!(!reclick_restarts(
            &preparing(LoginTarget::Managed {
                id: AccountId::new(),
                recipe: AccountRecipeId::Claude
            }),
            system(AccountRecipeId::Claude)
        ));
    }

    #[test]
    fn nothing_in_flight_is_nothing_to_take_over() {
        assert!(!reclick_restarts(
            &PendingLogin::None,
            system(AccountRecipeId::Claude)
        ));
    }
}

#[test]
fn a_login_target_always_names_its_auth_domain() {
    for recipe in AccountRecipeId::all() {
        assert_eq!(
            LoginTarget::Managed {
                id: AccountId::new(),
                recipe
            }
            .recipe(),
            recipe
        );
        assert_eq!(LoginTarget::System { recipe }.recipe(), recipe);
    }
}

/// A system login writes into the user's real home. Deleting it on cancel
/// would destroy credentials this app never created — the same reason a
/// reauth's permanent dir is spared, one step more severe.
#[test]
fn only_an_add_login_may_delete_its_directory_on_cancel() {
    assert!(cleanup_dir_on_cancel(LoginFinish::Add));
    assert!(!cleanup_dir_on_cancel(LoginFinish::Reauth));
    assert!(!cleanup_dir_on_cancel(LoginFinish::System));
}

/// The slot is single and process-wide, so a second attempt — whatever it
/// signs into — has to be refused while one is live. Before the login was
/// keyed at all there was nothing to store for an ambient sign-in, which is
/// exactly the case where two windows would have raced over one `~/.claude`.
#[gpui::test]
fn the_login_slot_admits_one_attempt_at_a_time(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        accounts_global::install_if_absent(cx, AccountsState::default());
        let first = next_login_attempt();
        assert!(accounts_global::begin_login(cx, first));
        assert!(
            !accounts_global::begin_login(cx, next_login_attempt()),
            "a second window must not sign in while one attempt is live"
        );
        accounts_global::finish_login(cx, first);
        let second = next_login_attempt();
        assert!(accounts_global::begin_login(cx, second));
        accounts_global::finish_login(cx, second);
    });
}

/// A replaced or orphaned attempt resolves late and must not release the
/// slot its replacement now holds — the replacement signs into the same
/// place, so anything keyed on that would free it.
#[gpui::test]
fn a_stale_attempt_does_not_free_a_newer_one(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        accounts_global::install_if_absent(cx, AccountsState::default());
        let stale = next_login_attempt();
        let live = next_login_attempt();
        assert!(accounts_global::begin_login(cx, live));
        accounts_global::finish_login(cx, stale);
        assert!(
            accounts_global::login_busy(cx),
            "the live attempt still owns the slot"
        );
        accounts_global::finish_login(cx, live);
        assert!(!accounts_global::login_busy(cx));
    });
}

#[test]
fn can_start_login_in_progress_is_not_startable() {
    // A real (near-instant, no-op) child process is the only way to
    // build a `LoginProcessHandle` — it has no public constructor
    // other than `LoginProcess::handle()`.
    let login = spawn_login(
        &daruda_agent::accounts::CliCommand {
            program: test_process::executable().to_path_buf(),
            args: Vec::new(),
            env: Vec::new(),
        },
        Duration::from_secs(5),
    )
    .expect("spawn a trivial process for the test handle");
    let pending = PendingLogin::InProgress {
        target: LoginTarget::Managed {
            id: AccountId::new(),
            recipe: AccountRecipeId::Claude,
        },
        attempt: next_login_attempt(),
        ambient_before: None,
        handle: login.handle(),
        finish: LoginFinish::Add,
    };
    assert!(!can_start_login(&pending));
    // Best-effort: the process has almost certainly already exited on
    // its own; this just avoids relying on that timing.
    if let PendingLogin::InProgress { handle, .. } = &pending {
        handle.cancel();
    }
}

fn agent(id: &str, command: &str) -> daruda_config::AgentDefinition {
    daruda_config::AgentDefinition::new(
        id.to_string(),
        id.to_string(),
        daruda_config::AgentLaunch::Raw(command.to_string()),
    )
}

#[test]
fn resolve_login_cli_prefers_the_active_agent_of_that_recipe() {
    let agents = vec![
        agent(
            "pinned",
            "npx -y @agentclientprotocol/claude-agent-acp@1.2.3",
        ),
        agent(
            "other",
            "npx -y @agentclientprotocol/claude-agent-acp@latest",
        ),
    ];
    let request = resolve_login_cli(&agents, "pinned", AccountRecipeId::Claude);
    assert_eq!(
        request.adapter,
        "npx -y @agentclientprotocol/claude-agent-acp@1.2.3"
    );
    assert_eq!(request.words, ["--cli", "auth", "login", "--claudeai"]);
}

/// The login prepares the adapter with the agent's own environment (its
/// `PATH`, its registry), as that agent's sessions do.
#[test]
fn resolve_login_cli_carries_the_agents_environment() {
    let mut pinned = agent(
        "pinned",
        "npx -y @agentclientprotocol/claude-agent-acp@latest",
    );
    pinned.env = Some(vec![(
        "npm_config_registry".into(),
        "https://mirror".into(),
    )]);
    let request = resolve_login_cli(&[pinned], "pinned", AccountRecipeId::Claude);
    assert_eq!(
        request.agent_env,
        [(
            "npm_config_registry".to_string(),
            "https://mirror".to_string()
        )]
    );
}

#[test]
fn resolve_login_cli_scans_the_catalog_when_the_active_agent_is_another_recipe() {
    let agents = vec![
        agent(
            "claude",
            "npx -y @agentclientprotocol/claude-agent-acp@latest",
        ),
        agent("codex", "npx -y @agentclientprotocol/codex-acp@9.9.9"),
    ];
    let request = resolve_login_cli(&agents, "claude", AccountRecipeId::Codex);
    assert_eq!(
        request.adapter,
        "npx -y @agentclientprotocol/codex-acp@9.9.9"
    );
    assert_eq!(request.words, ["cli", "login"]);
}

#[test]
fn resolve_login_cli_falls_back_to_the_builtin_for_an_empty_catalog() {
    let request = resolve_login_cli(&[], "", AccountRecipeId::Codex);
    let builtin = daruda_config::AgentDefinition::codex_default();
    let daruda_config::AgentLaunch::Raw(raw) = builtin.launch else {
        panic!("the built-in Codex launch is Raw");
    };
    assert_eq!(request.adapter, raw);
}

#[test]
fn resolve_login_cli_falls_back_when_the_catalog_is_remote_only() {
    // Ssh/Docker/`{{cwd}}` launches derive to no recipe at all, so none
    // of them can serve the request — the built-in must still answer it.
    let agents = vec![
        daruda_config::AgentDefinition::new(
            "remote-ssh".to_string(),
            "remote-ssh".to_string(),
            daruda_config::AgentLaunch::Ssh {
                adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".to_string(),
                host: "box".to_string(),
            },
        ),
        daruda_config::AgentDefinition::new(
            "remote-docker".to_string(),
            "remote-docker".to_string(),
            daruda_config::AgentLaunch::Docker {
                adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".to_string(),
                container: "dev".to_string(),
            },
        ),
        agent(
            "remote-cwd",
            "npx -y @agentclientprotocol/claude-agent-acp@latest --cwd {{cwd}}",
        ),
    ];
    let request = resolve_login_cli(&agents, "remote-ssh", AccountRecipeId::Claude);
    let daruda_config::AgentLaunch::Raw(raw) =
        daruda_config::AgentDefinition::claude_default().launch
    else {
        panic!("the built-in Claude launch is Raw");
    };
    assert_eq!(request.adapter, raw);
}

mod pane_login {
    use super::*;
    use crate::workspace::main_area::pane::AccountDomain;
    use daruda_store::accounts::AccountSelection;

    fn state_with(id: AccountId, recipe: AccountRecipeId) -> AccountsState {
        let mut st = AccountsState::default();
        st.accounts.push(ManagedAccount {
            id,
            recipe,
            email: None,
            organization: None,
            config_dir: "/x".into(),
            created_at: 0,
            last_authenticated_at: 0,
        });
        st
    }

    /// A managed pane signs back into its own account, in that account's
    /// own domain — read from the row rather than from the pane's agent,
    /// so a pane pointed at another domain's account can't reauthenticate
    /// the wrong credentials.
    #[test]
    fn a_managed_pane_signs_back_into_its_own_account() {
        let id = AccountId::new();
        let st = state_with(id, AccountRecipeId::Codex);
        assert_eq!(
            pane_login_target(
                AccountSelection::Managed(id),
                AccountDomain::Exactly(AccountRecipeId::Claude),
                &st
            ),
            Some(LoginTarget::Managed {
                id,
                recipe: AccountRecipeId::Codex
            })
        );
    }

    /// The ambient home is per-domain and the selection names no domain,
    /// so the pane's own agent has to supply it.
    #[test]
    fn a_system_pane_signs_into_its_agents_domain() {
        for recipe in AccountRecipeId::all() {
            assert_eq!(
                pane_login_target(
                    AccountSelection::SystemDefault,
                    AccountDomain::Exactly(recipe),
                    &AccountsState::default()
                ),
                Some(LoginTarget::System { recipe })
            );
        }
    }

    /// An agent with no local auth domain (remote, JSON stdio, or one this
    /// build does not recognize) has no login to run — offering a button
    /// would spawn a command for a domain that was never resolved.
    #[test]
    fn a_pane_with_no_auth_domain_offers_no_login() {
        assert_eq!(
            pane_login_target(
                AccountSelection::SystemDefault,
                AccountDomain::Unsupported,
                &AccountsState::default()
            ),
            None
        );
    }

    /// `Any` is the terminal case: no agent, so no single ambient home to
    /// sign into.
    #[test]
    fn an_unscoped_pane_offers_no_system_login() {
        assert_eq!(
            pane_login_target(
                AccountSelection::SystemDefault,
                AccountDomain::Any,
                &AccountsState::default()
            ),
            None
        );
    }

    /// The row can be deleted from the Settings window while a pane still
    /// points at it; there is nothing left to sign into.
    #[test]
    fn a_deleted_account_offers_no_login() {
        assert_eq!(
            pane_login_target(
                AccountSelection::Managed(AccountId::new()),
                AccountDomain::Exactly(AccountRecipeId::Claude),
                &AccountsState::default()
            ),
            None
        );
    }
}

mod billing {
    use super::*;

    /// The two ways the Claude agent advertises signing in, restated here
    /// rather than shared with `daruda_acp`'s own fixture: this asserts a
    /// contract *between* the crates, so it has to describe the agent
    /// independently of the code that parses it.
    ///
    /// Only the fields the assertion reads are kept — the id that decides
    /// the billing kind, and the runnable command whose trailing flags
    /// select the flow.
    fn advertised_claude_logins() -> Vec<daruda_acp::LoginMethod> {
        let response: agent_client_protocol::schema::v1::InitializeResponse =
            serde_json::from_value(serde_json::json!({
                "protocolVersion": 1,
                "agentCapabilities": {},
                "authMethods": [
                    {
                        "id": "claude-ai-login",
                        "name": "Claude Subscription",
                        "type": "terminal",
                        "_meta": {"terminal-auth": {
                            "command": "/opt/node/bin/node",
                            "args": ["/cache/claude-agent-acp", "--cli", "auth",
                                     "login", "--claudeai"]
                        }}
                    },
                    {
                        "id": "console-login",
                        "name": "Anthropic Console",
                        "type": "terminal",
                        "_meta": {"terminal-auth": {
                            "command": "/opt/node/bin/node",
                            "args": ["/cache/claude-agent-acp", "--cli", "auth",
                                     "login", "--console"]
                        }}
                    }
                ]
            }))
            .expect("the advertised payload parses");
        daruda_acp::parse_login_methods(&response.auth_methods)
    }

    /// The flags that pick a flow: everything after the adapter path the
    /// agent resolved for us.
    fn login_flags(method: &daruda_acp::LoginMethod) -> String {
        method
            .command
            .as_ref()
            .expect("the advertised method carries a runnable command")
            .args[1..]
            .join(" ")
    }

    /// daruda has to sign in on the flow that spends the plan the user
    /// already pays for, and the guard that says which one that is comes
    /// from the agent's own classification rather than a flag spelled out
    /// here. Pointing the login command at the other one would move a user
    /// onto per-token billing, and they would find out on an invoice.
    #[test]
    fn the_claude_login_runs_the_subscription_flow_not_the_metered_one() {
        let methods = advertised_claude_logins();
        let mut safe = methods.iter().filter(|m| m.kind.is_safe_default());
        let subscription = safe.next().expect("one flow bills against the plan");
        assert!(
            safe.next().is_none(),
            "more than one flow claims to be free — which one daruda runs is no longer decidable"
        );
        let metered = methods
            .iter()
            .find(|m| !m.kind.is_safe_default())
            .expect("the agent also advertises a metered flow");

        let words = resolve_login_cli(&[], "", AccountRecipeId::Claude)
            .words
            .join(" ");
        assert_eq!(
            words,
            login_flags(subscription),
            "the login command must run the subscription flow"
        );
        assert_ne!(
            words,
            login_flags(metered),
            "the login command runs the per-token billed flow"
        );
    }
}

mod status_probe {
    use super::*;
    use daruda_agent::accounts::auth_status::AuthStatusFormat;

    fn account(recipe: AccountRecipeId) -> AccountsState {
        let mut st = AccountsState::default();
        st.accounts.push(ManagedAccount {
            id: AccountId::new(),
            recipe,
            email: None,
            organization: None,
            config_dir: "/x".into(),
            created_at: 0,
            last_authenticated_at: 0,
        });
        st
    }

    /// The status read and the login it describes must come from one
    /// adapter, so this shares `resolve_login_cli`'s resolution — and
    /// pairs it with the format that adapter's CLI actually prints.
    #[test]
    fn the_status_command_reuses_the_agent_that_serves_the_domain() {
        let agents = vec![agent(
            "pinned",
            "npx -y @agentclientprotocol/claude-agent-acp@1.2.3",
        )];
        let (request, format) = resolve_status_cli(&agents, "pinned", AccountRecipeId::Claude)
            .expect("claude reports status");
        assert_eq!(
            request.adapter,
            "npx -y @agentclientprotocol/claude-agent-acp@1.2.3"
        );
        assert_eq!(request.words, ["--cli", "auth", "status", "--json"]);
        assert_eq!(format, AuthStatusFormat::Json);
    }

    /// A domain whose CLI prints a sentence has to be read as one — JSON
    /// parsing of prose yields no method, which is indistinguishable from
    /// being signed out.
    #[test]
    fn each_domain_gets_the_format_its_cli_prints() {
        let (request, format) =
            resolve_status_cli(&[], "", AccountRecipeId::Codex).expect("codex reports status");
        assert_eq!(request.words, ["cli", "login", "status"]);
        assert_eq!(format, AuthStatusFormat::Prose);
    }

    /// Nothing here has a stake in a domain with no account and no agent,
    /// and a probe would prepare the adapter — possibly downloading it and a
    /// Node runtime — for a row the user does not care about.
    #[test]
    fn an_untouched_domain_is_not_probed() {
        for recipe in AccountRecipeId::all() {
            assert!(
                !stake_in_domain(&AccountsState::default(), &[], recipe),
                "{recipe:?}"
            );
        }
    }

    #[test]
    fn an_account_in_a_domain_is_a_stake_in_it() {
        assert!(stake_in_domain(
            &account(AccountRecipeId::Codex),
            &[],
            AccountRecipeId::Codex
        ));
        assert!(!stake_in_domain(
            &account(AccountRecipeId::Codex),
            &[],
            AccountRecipeId::Claude
        ));
    }

    /// A configured agent counts even with no account yet — that is the
    /// domain whose ambient home the user is signing in to.
    #[test]
    fn a_configured_agent_is_a_stake_in_its_domain() {
        let agents = vec![agent(
            "claude",
            "npx -y @agentclientprotocol/claude-agent-acp@latest",
        )];
        assert!(stake_in_domain(
            &AccountsState::default(),
            &agents,
            AccountRecipeId::Claude
        ));
        assert!(!stake_in_domain(
            &AccountsState::default(),
            &agents,
            AccountRecipeId::Codex
        ));
    }

    /// The built-in fallback keeps a *requested* login startable, but must
    /// not make a passive refresh look like a stake.
    #[test]
    fn the_builtin_fallback_is_not_a_stake() {
        assert!(!stake_in_domain(
            &AccountsState::default(),
            &[],
            AccountRecipeId::Claude
        ));
        // …while the command itself still resolves, via that fallback.
        assert!(resolve_status_cli(&[], "", AccountRecipeId::Claude).is_some());
    }
}

#[test]
fn find_duplicate_matches_email_and_org() {
    let mut st = AccountsState::default();
    let id = AccountId::new();
    st.accounts.push(ManagedAccount {
        id,
        recipe: AccountRecipeId::Claude,
        email: Some("a@x.com".into()),
        organization: Some("Org".into()),
        config_dir: "/x".into(),
        created_at: 0,
        last_authenticated_at: 0,
    });
    let claude = AccountRecipeId::Claude;
    assert_eq!(
        find_duplicate(&st, claude, Some("a@x.com"), Some("Org")),
        Some(id)
    );
    // Same email, different org — a distinct account.
    assert_eq!(
        find_duplicate(&st, claude, Some("a@x.com"), Some("Other")),
        None
    );
    assert_eq!(
        find_duplicate(&st, claude, Some("b@x.com"), Some("Org")),
        None
    );
    // Same identity in another auth domain is separate credentials.
    assert_eq!(
        find_duplicate(&st, AccountRecipeId::Codex, Some("a@x.com"), Some("Org")),
        None
    );
}
