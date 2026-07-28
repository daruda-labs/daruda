//! Per-pane account switching + `Workspace::clear_account_override` (the
//! per-window account-delete hook, including its usage-cache prune).

use super::agent_chat::agent_view;
use super::*;
use crate::workspace::claude_session_ops::UsageKey;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_acp::ChatItem;
use daruda_store::accounts::{AccountId, AccountSelection};
use daruda_store::project::PaneCwd;

/// An Agent chat pane pinned to `selection`, pushed onto the active runtime.
fn seed_agent_pane(
    ws: &mut Workspace,
    selection: AccountSelection,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> PaneId {
    let mut pane = ws.create_agent_chat_pane(
        Some(PaneCwd::Local(std::env::temp_dir())),
        None,
        daruda_config::AgentDefinition::claude_default().id,
        None,
        window,
        cx,
    );
    pane.agent_chat_content_mut().unwrap().account = selection;
    let id = pane.id;
    ws.active_runtime_mut().panes.push(pane);
    id
}

/// A pane already switched to a managed account must be switchable back to
/// the system default — the bug this test guards: before the fix,
/// `switch_pane_account`'s target could not express "system default", so a
/// managed-account pane had no path back to `~/.claude`. With
/// [`AccountSelection`] the two states are distinct and both reachable.
#[gpui::test]
async fn switch_pane_account_reverts_a_managed_account_to_system_default(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let managed = AccountId::new();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                seed_agent_pane(ws, AccountSelection::Managed(managed), window, cx)
            })
        })
        .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        let pane = ws
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .unwrap();
        assert_eq!(
            pane.account_selection(),
            Some(AccountSelection::Managed(managed))
        );
    });

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.switch_pane_account(pane_id, AccountSelection::SystemDefault, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        let pane = ws
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .unwrap();
        assert_eq!(
            pane.account_selection(),
            Some(AccountSelection::SystemDefault),
            "switching to SystemDefault must revert the pane to the system default"
        );
    });
}

/// Picking the account the pane is already on must change nothing. Before the
/// guard, the click ran the full in-place switch — `/clear`'s teardown — so
/// re-selecting the checked entry in the status-bar dropdown destroyed the
/// conversation for no gain.
#[gpui::test]
async fn switch_pane_account_to_the_current_selection_is_a_no_op(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let managed = AccountId::new();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let id = seed_agent_pane(ws, AccountSelection::Managed(managed), window, cx);
                agent_view(ws, id).update(cx, |v, _| {
                    v.items.push(ChatItem::UserText("keep me".into()));
                });
                id
            })
        })
        .unwrap();
    cx.run_until_parked();

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.switch_pane_account(pane_id, AccountSelection::Managed(managed), window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert_eq!(
            agent_view(ws, pane_id).read(cx).items.len(),
            1,
            "re-selecting the account the pane already uses must not reset the session"
        );
    });
}

/// An idle pane holding a conversation must not be wiped by a switch. The new
/// account needs its own ACP session and the transcript cannot move to it, so
/// the switch confirms first and opens a *new* pane — until that confirmation
/// the source pane keeps both its conversation and its own account.
#[gpui::test]
async fn switch_pane_account_keeps_an_idle_conversation(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let managed = AccountId::new();
    let pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let id = seed_agent_pane(ws, AccountSelection::Managed(managed), window, cx);
                agent_view(ws, id).update(cx, |v, _| {
                    v.items
                        .push(ChatItem::UserText("a long investigation".into()));
                });
                id
            })
        })
        .unwrap();
    cx.run_until_parked();
    let panes_before = workspace.read_with(cx, |ws, _| ws.active_runtime().panes.len());

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.switch_pane_account(pane_id, AccountSelection::SystemDefault, window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, cx| {
        assert_eq!(
            agent_view(ws, pane_id).read(cx).items.len(),
            1,
            "an idle pane's conversation must survive an account switch"
        );
        let pane = ws
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == pane_id)
            .unwrap();
        assert_eq!(
            pane.account_selection(),
            Some(AccountSelection::Managed(managed)),
            "the source pane keeps its own account — the new one goes to the new pane"
        );
        assert_eq!(
            ws.active_runtime().panes.len(),
            panes_before,
            "the new pane only opens once the user confirms the dialog"
        );
    });
}

/// Add a managed account of `recipe` to the workspace's mirror.
fn push_account(
    ws: &mut Workspace,
    recipe: daruda_store::accounts::AccountRecipeId,
    id: AccountId,
) {
    ws.accounts
        .accounts
        .push(daruda_store::accounts::ManagedAccount {
            id,
            recipe,
            email: None,
            organization: None,
            config_dir: std::env::temp_dir(),
            created_at: 0,
            last_authenticated_at: 0,
        });
}

/// Register `default_id` as the configured default for `recipe`.
fn seed_default_account(
    ws: &mut Workspace,
    recipe: daruda_store::accounts::AccountRecipeId,
    default_id: AccountId,
) {
    push_account(ws, recipe, default_id);
    ws.accounts.default_by_recipe.insert(recipe, default_id);
}

/// A freshly created agent-chat pane must be seeded with its own auth
/// domain's configured default account at creation time, not left to a
/// resolve-time fallback (`SystemDefault` is the explicit "System" choice).
/// A terminal has no agent, so no domain default applies to it.
#[gpui::test]
async fn new_panes_seed_the_default_only_for_a_matching_agent(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let default_id = AccountId::new();
    workspace.update(cx, |ws, _| {
        seed_default_account(
            ws,
            daruda_store::accounts::AccountRecipeId::Claude,
            default_id,
        );
    });

    let (terminal, agent_chat) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let terminal = ws.create_pane(window, cx).expect("fresh pane spawn");
                let agent_chat = ws.create_new_agent_chat_pane(
                    daruda_config::AgentDefinition::claude_default().id,
                    Some(std::env::temp_dir()),
                    None,
                    window,
                    cx,
                );
                (terminal.account_selection(), agent_chat.account_selection())
            })
        })
        .unwrap();

    assert_eq!(
        agent_chat,
        Some(AccountSelection::Managed(default_id)),
        "a Claude agent-chat pane inherits the Claude default"
    );
    assert_eq!(
        terminal,
        Some(AccountSelection::SystemDefault),
        "a terminal has no agent, so no auth domain's default applies to it"
    );
}

/// With no configured default, every new pane starts on the system default.
#[gpui::test]
async fn new_panes_without_a_default_start_on_the_system_default(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let (terminal, agent_chat) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let terminal = ws.create_pane(window, cx).expect("fresh pane spawn");
                let agent_chat = ws.create_new_agent_chat_pane(
                    daruda_config::AgentDefinition::claude_default().id,
                    Some(std::env::temp_dir()),
                    None,
                    window,
                    cx,
                );
                (terminal.account_selection(), agent_chat.account_selection())
            })
        })
        .unwrap();

    assert_eq!(terminal, Some(AccountSelection::SystemDefault));
    assert_eq!(agent_chat, Some(AccountSelection::SystemDefault));
}

/// Write a Codex credential fixture (`auth.json` with an `id_token` JWT) into
/// `dir` — the plaintext file `CodexRecipe::has_credentials` reads. Codex has
/// no Keychain item, so this is the whole credential surface.
fn write_codex_auth_json(dir: &std::path::Path, email: &str) {
    use base64::Engine as _;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::json!({ "email": email }).to_string());
    let auth = serde_json::json!({
        "tokens": { "id_token": format!("header.{payload}.signature") },
    });
    std::fs::write(dir.join("auth.json"), auth.to_string()).expect("write auth.json");
}

/// Drive `finish_login`'s success path for a Codex login into `config_dir`.
/// Mirrors the real flow: the pending login must be `InProgress` for the
/// finish callback to be accepted.
fn finish_codex_login(
    ws: &mut Workspace,
    config_dir: std::path::PathBuf,
    cx: &mut Context<Workspace>,
) -> AccountId {
    use crate::workspace::PendingLogin;
    use crate::workspace::account_login_ops::LoginMode;
    // A real (near-instant) child process is the only way to build a
    // `LoginProcessHandle` — it has no other public constructor.
    let login = daruda_claude::accounts::spawn_login(
        "/usr/bin/true",
        &[],
        &[],
        std::time::Duration::from_secs(5),
    )
    .expect("spawn a trivial process for the test handle");
    let account_id = AccountId::new();
    ws.pending_login = PendingLogin::InProgress {
        account_id,
        recipe: daruda_store::accounts::AccountRecipeId::Codex,
        handle: login.handle(),
        mode: LoginMode::Add,
    };
    ws.finish_login(
        account_id,
        config_dir,
        daruda_store::accounts::AccountRecipeId::Codex,
        daruda_claude::accounts::LoginOutcome::Success,
        cx,
    );
    account_id
}

/// Adding an account must never change which account new panes get. Before
/// the fix the first login for a domain silently became its default, so a
/// user who only wanted a second account found every new pane switched off
/// System with nothing in the UI saying so.
#[gpui::test]
async fn login_success_does_not_promote_the_new_account_to_default(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let dir = tempfile::tempdir().expect("tempdir");
    write_codex_auth_json(dir.path(), "alice@openai.com");

    let account_id = workspace.update(cx, |ws, cx| {
        finish_codex_login(ws, dir.path().to_path_buf(), cx)
    });
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        assert!(
            ws.accounts.accounts.iter().any(|a| a.id == account_id),
            "the account itself is filed"
        );
        assert!(
            ws.accounts.default_by_recipe.is_empty(),
            "the first account for a domain must not become its default"
        );
    });
}

/// The login-success path must accept a Codex login, whose credentials are a
/// plaintext `auth.json` and never a Keychain item — i.e. it asks the account's
/// recipe rather than calling Claude's scoped-Keychain read directly.
#[gpui::test]
async fn login_success_accepts_a_codex_account_from_its_auth_json(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let dir = tempfile::tempdir().expect("tempdir");
    write_codex_auth_json(dir.path(), "alice@openai.com");

    let account_id = workspace.update(cx, |ws, cx| {
        finish_codex_login(ws, dir.path().to_path_buf(), cx)
    });
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        let account = ws
            .accounts
            .find(account_id)
            .expect("a Codex login with an auth.json is kept, not discarded");
        assert_eq!(
            account.recipe,
            daruda_store::accounts::AccountRecipeId::Codex
        );
        assert_eq!(
            account.email.as_deref(),
            Some("alice@openai.com"),
            "identity comes from the recipe's own reader"
        );
    });
    assert!(
        dir.path().join("auth.json").exists(),
        "a kept account's config dir must survive"
    );
}

/// A Codex login that produced no credentials must still be rejected — the
/// recipe check has to actually gate, not wave everything through.
#[gpui::test]
async fn login_success_without_credentials_is_rejected(cx: &mut TestAppContext) {
    let (_wh, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let dir = tempfile::tempdir().expect("tempdir").keep();
    let account_id = workspace.update(cx, |ws, cx| finish_codex_login(ws, dir.clone(), cx));
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        assert!(ws.accounts.find(account_id).is_none());
    });
    assert!(!dir.exists(), "the throwaway dir is cleaned up");
}

#[gpui::test]
fn clear_account_override_prunes_usage_caches_for_deleted_account_only(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    let deleted = AccountId::new();
    let kept = AccountId::new();

    ws.update(cx, |ws, cx| {
        for key in [
            AccountSelection::Managed(deleted),
            AccountSelection::Managed(kept),
            AccountSelection::SystemDefault,
        ] {
            // Seed both domains: a deleted account must be pruned from every
            // one of them, not just the domain that happened to be focused.
            for recipe in daruda_store::accounts::AccountRecipeId::ALL {
                ws.claude.usage_by_account.advance_usage(
                    UsageKey {
                        recipe,
                        account: key,
                    },
                    Ok(daruda_claude::ProviderUsage::new(recipe, Vec::new(), None)),
                );
            }
            ws.claude
                .usage_by_account
                .set_activity(key, daruda_claude::ActivityStats::default());
        }

        ws.clear_account_override(deleted, cx);

        let usage = &ws.claude.usage_by_account;
        let outcome = |account| {
            usage.usage(UsageKey {
                recipe: daruda_store::accounts::AccountRecipeId::Claude,
                account,
            })
        };
        assert_eq!(
            outcome(AccountSelection::Managed(deleted)),
            daruda_claude::UsageOutcome::Pending,
            "a deleted account's usage must be gone, not merely stale"
        );
        assert!(outcome(AccountSelection::Managed(kept)).has_numbers());
        assert!(outcome(AccountSelection::SystemDefault).has_numbers());
        // The other domain's entry for the deleted account is gone too.
        assert_eq!(
            usage.usage(UsageKey {
                recipe: daruda_store::accounts::AccountRecipeId::Codex,
                account: AccountSelection::Managed(deleted),
            }),
            daruda_claude::UsageOutcome::Pending
        );

        assert!(usage.activity(AccountSelection::Managed(deleted)).is_none());
        assert!(usage.activity(AccountSelection::Managed(kept)).is_some());
        assert!(usage.activity(AccountSelection::SystemDefault).is_some());
    });
}

/// Restore must not keep an agent-chat pane pinned to an account from another
/// auth domain: resolution refuses such a pin, so leaving it on the pane would
/// re-serialize an account that pane can never use. Terminal panes have no
/// agent constraining them and are left alone.
#[gpui::test]
async fn restore_resets_only_a_cross_domain_agent_chat_pin(cx: &mut TestAppContext) {
    use daruda_store::accounts::AccountRecipeId;
    use daruda_store::project::{
        SerializedAgentChatContent, SerializedLayout, SplitDirectionSerde,
    };

    let mut config = daruda_config::Config::default();
    config
        .agents
        .push(daruda_config::AgentDefinition::codex_default());
    let (window_handle, workspace) = build_workspace_with(cx, &config, None);
    cx.run_until_parked();

    let claude_account = AccountId::new();
    workspace.update(cx, |ws, _| {
        push_account(ws, AccountRecipeId::Claude, claude_account);
    });

    let agent_leaf = |pane_id: u64, agent_id: String| SerializedLayout::Leaf {
        pane_id,
        cwd: None,
        file: None,
        agent_chat: Some(SerializedAgentChatContent {
            cwd: Some(PaneCwd::Local(std::env::temp_dir())),
            session_id: None,
            title: None,
            agent_id: Some(agent_id),
            account_id: Some(claude_account),
            mode_id: None,
        }),
        account_id: None,
    };
    let layout = SerializedLayout::Split {
        direction: SplitDirectionSerde::Horizontal,
        children: vec![
            agent_leaf(1, daruda_config::AgentDefinition::codex_default().id),
            agent_leaf(2, daruda_config::AgentDefinition::claude_default().id),
            SerializedLayout::Leaf {
                pane_id: 3,
                cwd: Some(std::env::temp_dir()),
                file: None,
                agent_chat: None,
                account_id: Some(claude_account),
            },
        ],
        ratios: vec![1.0 / 3.0; 3],
    };

    let mut id_map = std::collections::HashMap::new();
    let mut scratch = Vec::new();
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.rebuild_layout(&layout, None, &mut id_map, &mut scratch, window, cx)
                .expect("rebuild");
        })
    })
    .unwrap();
    cx.run_until_parked();

    assert_eq!(
        scratch[0].account_selection(),
        Some(AccountSelection::SystemDefault),
        "a Codex pane pinned to a Claude account must fall back to System"
    );
    assert_eq!(
        scratch[1].account_selection(),
        Some(AccountSelection::Managed(claude_account)),
        "a same-domain pin survives restore"
    );
    assert_eq!(
        scratch[2].account_selection(),
        Some(AccountSelection::Managed(claude_account)),
        "a terminal's own account governs — never reset"
    );
}

/// A pin whose account row is gone, and a pane whose agent supports no managed
/// account at all (remote adapter), both restore to the system default.
#[test]
fn restore_resets_a_dangling_or_unsupported_agent_pin() {
    use crate::workspace::persistence::restored_agent_account;
    use daruda_store::accounts::{AccountRecipeId, AccountsState, ManagedAccount};

    let id = AccountId::new();
    let mut state = AccountsState::default();
    state.accounts.push(ManagedAccount {
        id,
        recipe: AccountRecipeId::Claude,
        email: None,
        organization: None,
        config_dir: std::env::temp_dir(),
        created_at: 0,
        last_authenticated_at: 0,
    });

    assert_eq!(
        restored_agent_account(Some(id), Some(AccountRecipeId::Claude), &state),
        AccountSelection::Managed(id)
    );
    assert_eq!(
        restored_agent_account(
            Some(AccountId::new()),
            Some(AccountRecipeId::Claude),
            &state
        ),
        AccountSelection::SystemDefault,
        "an account that no longer exists"
    );
    assert_eq!(
        restored_agent_account(Some(id), None, &state),
        AccountSelection::SystemDefault,
        "an agent with no auth domain (remote adapter) holds no account"
    );
    assert_eq!(
        restored_agent_account(None, Some(AccountRecipeId::Claude), &state),
        AccountSelection::SystemDefault
    );
}

/// A terminal pane must materialize its managed account's config dir, not just
/// inject the env var pointing at it: `CODEX_HOME` on a bare directory gives a
/// Codex CLI run from that shell none of the symlinked system resources.
/// `create_pane_with_cwd` is the one terminal spawn funnel, so prepping there
/// covers every path that reaches it.
#[gpui::test]
async fn a_terminal_pane_prepares_its_managed_accounts_config_dir(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    cx.run_until_parked();

    let id = AccountId::new();
    workspace.update(cx, |ws, _| {
        push_account(ws, daruda_store::accounts::AccountRecipeId::Codex, id);
    });

    let config_dir = workspace.read_with(cx, |ws, _| {
        daruda_claude::accounts::account_config_dir(&ws.data_dir, id)
    });
    // Negative control: nothing before the spawn creates the dir, so its
    // existence afterwards can only come from `prepare_dir`.
    assert!(
        !config_dir.exists(),
        "the account dir must not exist before the pane spawns"
    );

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let selection = AccountSelection::Managed(id);
            let prepared = crate::workspace::main_area::pane::resolve_pane_account(
                &ws.accounts,
                &ws.data_dir,
                selection,
                crate::workspace::main_area::pane::AccountDomain::Any,
            )
            .expect("the seeded Codex account resolves");
            ws.create_pane_with_cwd(
                Some(std::env::temp_dir()),
                selection,
                Some(&prepared),
                window,
                cx,
            )
            .expect("terminal pane spawn");
        })
    })
    .unwrap();

    assert!(
        config_dir.is_dir(),
        "create_pane_with_cwd must run AccountRecipe::prepare_dir"
    );
}

/// The Usage tab stacks one block per signed-in domain. A domain nobody is
/// signed into contributes none — that is what keeps a Claude-only user from
/// seeing a permanently empty Codex block, and vice versa.
#[gpui::test]
async fn usage_sections_cover_signed_in_domains_only(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);

    let sections = |ws: &mut Workspace, cx: &mut gpui::Context<Workspace>| {
        ws.prepare_right_dock_snapshot(cx)
            .usage
            .iter()
            .map(|section| section.recipe)
            .collect::<Vec<_>>()
    };

    ws.update(cx, |ws, cx| {
        // Before any poll lands both domains are `Pending`: a section each,
        // holding the layout rather than popping in a beat later.
        assert_eq!(
            sections(ws, cx),
            daruda_store::accounts::AccountRecipeId::ALL.to_vec()
        );

        let claude_key = UsageKey {
            recipe: daruda_store::accounts::AccountRecipeId::Claude,
            account: AccountSelection::SystemDefault,
        };
        let codex_key = UsageKey {
            recipe: daruda_store::accounts::AccountRecipeId::Codex,
            account: AccountSelection::SystemDefault,
        };

        ws.claude.usage_by_account.advance_usage(
            claude_key,
            Ok(daruda_claude::ProviderUsage::new(
                daruda_store::accounts::AccountRecipeId::Claude,
                Vec::new(),
                None,
            )),
        );
        ws.claude
            .usage_by_account
            .advance_usage(codex_key, Err(daruda_claude::FetchError::NoToken));

        assert_eq!(
            sections(ws, cx),
            vec![daruda_store::accounts::AccountRecipeId::Claude],
            "a signed-out domain must leave no section behind"
        );

        // Signing out of both leaves nothing, which the renderer replaces with
        // a single "no provider" notice.
        ws.claude
            .usage_by_account
            .advance_usage(claude_key, Err(daruda_claude::FetchError::NoToken));
        assert!(sections(ws, cx).is_empty());
    });
}

/// A failed refresh is not a sign-out: the section stays, showing the last
/// numbers, so a network blip doesn't collapse the dashboard.
#[gpui::test]
async fn a_failed_refresh_keeps_its_usage_section(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        let key = UsageKey {
            recipe: daruda_store::accounts::AccountRecipeId::Claude,
            account: AccountSelection::SystemDefault,
        };
        ws.claude.usage_by_account.advance_usage(
            key,
            Ok(daruda_claude::ProviderUsage::new(
                daruda_store::accounts::AccountRecipeId::Claude,
                Vec::new(),
                None,
            )),
        );
        ws.claude
            .usage_by_account
            .advance_usage(key, Err(daruda_claude::FetchError::Http("500".into())));

        let snap = ws.prepare_right_dock_snapshot(cx);
        let claude = snap
            .usage
            .iter()
            .find(|s| s.recipe == daruda_store::accounts::AccountRecipeId::Claude)
            .expect("the section survives a failed refresh");
        assert!(claude.outcome.is_stale());
        assert!(claude.outcome.snapshot().is_some());
    });
}
