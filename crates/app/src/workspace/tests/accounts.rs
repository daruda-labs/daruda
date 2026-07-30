//! Per-pane account switching + `Workspace::clear_account_override` (the
//! per-window account-delete hook, including its usage-cache prune).

use super::agent_chat::agent_view;
use super::*;
use crate::workspace::claude_session_ops::UsageKey;
use crate::workspace::main_area::pane::{AccountDomain, FocusedAccount};
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_acp::ChatItem;
use daruda_store::accounts::{AccountId, AccountRecipeId, AccountSelection};
use daruda_store::project::PaneCwd;

/// An Agent chat pane pinned to `selection`, pushed onto the active runtime.
/// These account tests inspect host-side routing only; no cwd keeps the pane
/// offline so account switches don't spawn a live ACP reconnect.
fn seed_agent_pane(
    ws: &mut Workspace,
    selection: AccountSelection,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> PaneId {
    let mut pane = ws.create_agent_chat_pane(
        None,
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

/// An agent-chat pane running `agent_id`, pushed onto the active runtime
/// *and* focused — unlike `seed_agent_pane`, this needs to be the actually
/// focused pane so `AccountDomain::for_pane` (read by
/// `prepare_right_dock_snapshot` via `focused_account_pane`) resolves it.
fn seed_and_focus_agent_pane(
    ws: &mut Workspace,
    agent_id: String,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> PaneId {
    let pane = ws.create_agent_chat_pane(None, None, agent_id, None, window, cx);
    let id = pane.id;
    ws.active_runtime_mut().panes.push(pane);
    ws.active_runtime_mut().focused_pane_id = id;
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
    let login = daruda_agent::accounts::spawn_login(
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
        daruda_agent::accounts::LoginOutcome::Success,
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
            for recipe in daruda_store::accounts::AccountRecipeId::all() {
                ws.claude.usage_by_account.advance_usage(
                    UsageKey {
                        recipe,
                        account: key,
                    },
                    Ok(daruda_agent::ProviderUsage::new(recipe, Vec::new(), None)),
                );
            }
            ws.claude.usage_by_account.set_activity(
                UsageKey {
                    recipe: daruda_store::accounts::AccountRecipeId::Claude,
                    account: key,
                },
                daruda_agent::ActivityStats::default(),
            );
        }
        ws.claude.sticky_focus_by_recipe.insert(
            daruda_store::accounts::AccountRecipeId::Claude,
            crate::workspace::main_area::pane::FocusedAccount::Managed {
                id: deleted,
                recipe: daruda_store::accounts::AccountRecipeId::Claude,
                config_dir: std::path::PathBuf::from("/tmp/deleted"),
            },
        );
        ws.claude.sticky_focus_by_recipe.insert(
            daruda_store::accounts::AccountRecipeId::Codex,
            crate::workspace::main_area::pane::FocusedAccount::Managed {
                id: kept,
                recipe: daruda_store::accounts::AccountRecipeId::Codex,
                config_dir: std::path::PathBuf::from("/tmp/kept"),
            },
        );

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
            daruda_agent::UsageOutcome::Pending,
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
            daruda_agent::UsageOutcome::Pending
        );

        let activity_key = |account| UsageKey {
            recipe: daruda_store::accounts::AccountRecipeId::Claude,
            account,
        };
        assert!(
            usage
                .activity(activity_key(AccountSelection::Managed(deleted)))
                .is_none()
        );
        assert!(
            usage
                .activity(activity_key(AccountSelection::Managed(kept)))
                .is_some()
        );
        assert!(
            usage
                .activity(activity_key(AccountSelection::SystemDefault))
                .is_some()
        );
        assert!(
            !ws.claude
                .sticky_focus_by_recipe
                .values()
                .any(|focused| matches!(
                    focused,
                    crate::workspace::main_area::pane::FocusedAccount::Managed { id, .. }
                        if *id == deleted
                )),
            "deleted accounts must not survive in sticky usage focus"
        );
        assert!(
            ws.claude
                .sticky_focus_by_recipe
                .values()
                .any(|focused| matches!(
                    focused,
                    crate::workspace::main_area::pane::FocusedAccount::Managed { id, .. }
                        if *id == kept
                )),
            "unrelated sticky usage focus entries are preserved"
        );
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
        daruda_agent::accounts::account_config_dir(&ws.data_dir, id)
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
            daruda_store::accounts::AccountRecipeId::all().collect::<Vec<_>>()
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
            Ok(daruda_agent::ProviderUsage::new(
                daruda_store::accounts::AccountRecipeId::Claude,
                Vec::new(),
                None,
            )),
        );
        ws.claude
            .usage_by_account
            .advance_usage(codex_key, Err(daruda_agent::FetchError::NoToken));

        assert_eq!(
            sections(ws, cx),
            vec![daruda_store::accounts::AccountRecipeId::Claude],
            "a signed-out domain must leave no section behind"
        );

        // Signing out of both leaves nothing, which the renderer replaces with
        // a single "no provider" notice.
        ws.claude
            .usage_by_account
            .advance_usage(claude_key, Err(daruda_agent::FetchError::NoToken));
        assert!(sections(ws, cx).is_empty());
    });
}

#[gpui::test]
async fn empty_activity_is_not_attached_to_usage_snapshot(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        let key = UsageKey {
            recipe: daruda_store::accounts::AccountRecipeId::Claude,
            account: AccountSelection::SystemDefault,
        };
        ws.claude.usage_by_account.advance_usage(
            key,
            Ok(daruda_agent::ProviderUsage::new(
                daruda_store::accounts::AccountRecipeId::Claude,
                Vec::new(),
                None,
            )),
        );

        ws.claude
            .usage_by_account
            .set_activity(key, daruda_agent::ActivityStats::default());
        assert!(
            ws.prepare_right_dock_snapshot(cx).activity.is_empty(),
            "empty activity should not render chart headings"
        );

        ws.claude.usage_by_account.set_activity(
            key,
            daruda_agent::ActivityStats {
                daily: vec![daruda_agent::DayActivity {
                    date: "2026-07-29".to_string(),
                    turns: 1,
                    tokens: 2,
                }],
                recent_sessions: Vec::new(),
            },
        );
        assert_eq!(ws.prepare_right_dock_snapshot(cx).activity.len(), 1);
    });
}

/// `prepare_right_dock_snapshot` threads the focused pane's resolved
/// `AccountDomain` and the switcher's manual override straight through — the
/// Usage tab renderer (`right_dock::usage::resolve_displayed_domain`) has
/// nowhere else to read either from.
#[gpui::test]
async fn right_dock_snapshot_threads_focused_domain_and_override(cx: &mut TestAppContext) {
    // The default test `Config` only carries Claude in its agent catalog
    // (`daruda_config::default_agents`) — Codex needs to be present too, or
    // `agent_launch_for` can't resolve the seeded pane's agent id and its
    // domain reads back as `Unsupported` instead of `Exactly(Codex)`.
    let config = daruda_config::Config {
        agents: vec![
            daruda_config::AgentDefinition::claude_default(),
            daruda_config::AgentDefinition::codex_default(),
        ],
        ..Default::default()
    };
    let (window_handle, workspace) = build_workspace_with(cx, &config, None);
    cx.run_until_parked();

    // A fresh workspace boots on its default terminal tab — no agent-chat
    // pane focused, so the domain is ambiguous and the override starts unset.
    workspace.update(cx, |ws, cx| {
        let snap = ws.prepare_right_dock_snapshot(cx);
        assert_eq!(snap.focused_agent_domain, AccountDomain::Any);
        assert_eq!(snap.usage_domain_override, None);

        ws.set_usage_domain_override(AccountRecipeId::Codex, cx);
        let snap = ws.prepare_right_dock_snapshot(cx);
        assert_eq!(snap.usage_domain_override, Some(AccountRecipeId::Codex));
    });

    // Focusing a Codex agent-chat pane resolves the domain exactly.
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            seed_and_focus_agent_pane(
                ws,
                daruda_config::AgentDefinition::codex_default().id,
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.update(cx, |ws, cx| {
        let snap = ws.prepare_right_dock_snapshot(cx);
        assert_eq!(
            snap.focused_agent_domain,
            AccountDomain::Exactly(AccountRecipeId::Codex)
        );
    });
}

/// `prepare_right_dock_snapshot`'s `recent_sessions` only surfaces sessions
/// whose cwd matches the active Lane, most-recent first, capped at 10 — the
/// Usage tab table has nowhere else to get this filtering/ordering from.
#[gpui::test]
async fn recent_sessions_are_scoped_to_active_lane_sorted_and_capped(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap();
    let inactive_temp = tempfile::tempdir().unwrap();
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(temp.path());
    let (_wh, ws) = build_workspace_with(cx, &config, Some(project));
    cx.run_until_parked();

    ws.update(cx, |ws, cx| {
        let managed = AccountId::new();
        let managed_selection = AccountSelection::Managed(managed);
        ws.claude.sticky_focus_by_recipe.insert(
            AccountRecipeId::Claude,
            FocusedAccount::Managed {
                id: managed,
                recipe: AccountRecipeId::Claude,
                config_dir: temp.path().join("account"),
            },
        );

        let key = UsageKey {
            recipe: AccountRecipeId::Claude,
            account: managed_selection,
        };
        ws.claude.usage_by_account.advance_usage(
            key,
            Ok(daruda_agent::ProviderUsage::new(
                AccountRecipeId::Claude,
                Vec::new(),
                None,
            )),
        );

        let inactive_lane_id: daruda_store::project::LaneId = 999;
        ws.projects[0]
            .lanes
            .push(crate::lane::Lane::default_for_project(
                inactive_lane_id,
                inactive_temp.path().to_path_buf(),
            ));
        let open_lane_cwd = ws.projects[0].lanes[0].path.clone();
        let inactive_lane_cwd = inactive_temp.path().to_path_buf();
        let unrelated_cwd = std::path::PathBuf::from("/does/not/match/any/open/lane");

        let mut sessions = vec![
            daruda_agent::activity::SessionSummary {
                session_id: "matched-older".to_string(),
                cwd: open_lane_cwd.clone(),
                title: Some("Older session".to_string()),
                prompt_preview: None,
                git_branch: None,
                last_active: std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000),
            },
            daruda_agent::activity::SessionSummary {
                session_id: "matched-newest".to_string(),
                cwd: open_lane_cwd.clone(),
                title: Some("Newest session".to_string()),
                prompt_preview: Some("Newest prompt".to_string()),
                git_branch: Some("main".to_string()),
                last_active: std::time::UNIX_EPOCH + std::time::Duration::from_secs(2_000),
            },
            daruda_agent::activity::SessionSummary {
                session_id: "inactive-lane-newer".to_string(),
                cwd: inactive_lane_cwd,
                title: Some("Other lane".to_string()),
                prompt_preview: None,
                git_branch: None,
                last_active: std::time::UNIX_EPOCH + std::time::Duration::from_secs(3_000),
            },
            daruda_agent::activity::SessionSummary {
                session_id: "unmatched".to_string(),
                cwd: unrelated_cwd,
                title: None,
                prompt_preview: None,
                git_branch: None,
                last_active: std::time::UNIX_EPOCH + std::time::Duration::from_secs(4_000),
            },
        ];
        // Pad past the cap so truncation is actually exercised.
        for i in 0..8 {
            sessions.push(daruda_agent::activity::SessionSummary {
                session_id: format!("pad-{i}"),
                cwd: open_lane_cwd.clone(),
                title: None,
                prompt_preview: None,
                git_branch: None,
                last_active: std::time::UNIX_EPOCH,
            });
        }

        ws.claude.usage_by_account.set_activity(
            key,
            daruda_agent::ActivityStats {
                daily: Vec::new(),
                recent_sessions: sessions,
            },
        );

        let snap = ws.prepare_right_dock_snapshot(cx);
        let (_, restorable) = snap
            .recent_sessions
            .iter()
            .find(|(recipe, _)| *recipe == AccountRecipeId::Claude)
            .expect("Claude has sessions matching the open lane");

        assert_eq!(restorable.len(), 10, "capped at 10");
        assert!(
            restorable.iter().all(|s| s.cwd == open_lane_cwd),
            "a session outside the active lane must be excluded, even if another open lane matches it"
        );
        assert_eq!(
            restorable[0].session_id, "matched-newest",
            "most-recent first"
        );
        assert_eq!(
            restorable[0].agent_id,
            daruda_config::AgentDefinition::claude_default().id
        );
        assert_eq!(restorable[0].prompt_preview.as_deref(), Some("Newest prompt"));
        assert_eq!(restorable[0].git_branch.as_deref(), Some("main"));
        assert!(
            restorable.iter().all(|s| s.account == managed_selection),
            "restore payload must preserve the account whose activity cache produced it"
        );
    });
}

/// `restore_session` must focus an already-open pane for the same session
/// id rather than opening a duplicate tab.
///
/// The existing pane is seeded with `cwd: None` (mirrors `seed_agent_pane`'s
/// "no cwd keeps the pane offline" trick above) so it parks in `Error`
/// rather than `Idle` — `restore_session` ends in a real `focus_pane` call,
/// and `maybe_connect_agent_chat` only skips a live ACP connect attempt for
/// a non-`Idle` pane. No test in this suite may risk a real subprocess
/// connect; that's the whole reason `seed_agent_pane` exists.
#[gpui::test]
async fn restore_session_focuses_an_already_open_pane_instead_of_duplicating(
    cx: &mut TestAppContext,
) {
    let temp = tempfile::tempdir().unwrap();
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(temp.path());
    let (window_handle, workspace) = build_workspace_with(cx, &config, Some(project));
    cx.run_until_parked();

    let (lane_ref, existing_pane_id) = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    None,
                    Some("existing-session".to_string()),
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                (ws.active, id)
            })
        })
        .unwrap();
    cx.run_until_parked();

    // `cwd` here is never read — the dedupe match on `session_id` short-
    // circuits `restore_session` before it would otherwise build a pane
    // from this field.
    let session = crate::workspace::layout::snap::RestorableSession {
        session_id: "existing-session".to_string(),
        agent_id: daruda_config::AgentDefinition::claude_default().id,
        account: AccountSelection::SystemDefault,
        lane_ref,
        title: None,
        prompt_preview: None,
        git_branch: None,
        cwd: temp.path().to_path_buf(),
        last_active: std::time::SystemTime::now(),
    };
    let panes_before = workspace.read_with(cx, |ws, _| ws.active_runtime().panes.len());

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.restore_session(session, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.active_runtime().panes.len(),
            panes_before,
            "must focus the existing pane, not add a duplicate"
        );
        assert_eq!(ws.active_runtime().focused_pane_id, existing_pane_id);
    });
}

/// `restore_session` activates the session's Lane first when it isn't
/// already the active one — combined with the dedupe path above (an
/// offline, `cwd: None` existing pane in the target lane) so the only
/// `focus_pane` call this test exercises lands on a non-`Idle` pane, same
/// safety requirement as the sibling test above.
#[gpui::test]
async fn restore_session_activates_the_sessions_lane_before_focusing(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap();
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(temp.path());
    let (window_handle, workspace) = build_workspace_with(cx, &config, Some(project));
    cx.run_until_parked();

    let second_root = tempfile::tempdir().unwrap();
    let second_lane_id: daruda_store::project::LaneId = 999;
    let (target, initial_active) = workspace.update(cx, |ws, _| {
        let project_id = ws.projects[0].id;
        ws.projects[0]
            .lanes
            .push(crate::lane::Lane::default_for_project(
                second_lane_id,
                second_root.path().to_path_buf(),
            ));
        let target = daruda_store::project::LaneRef {
            project: project_id,
            lane: second_lane_id,
        };
        (target, ws.active)
    });
    assert_ne!(initial_active, target, "the second lane starts inactive");

    let existing_pane_id = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.activate_lane(target, window, cx);
                let pane = ws.create_agent_chat_pane(
                    None,
                    Some("existing-session".to_string()),
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                ws.activate_lane(initial_active, window, cx);
                id
            })
        })
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |ws, _| {
        assert_eq!(ws.active, initial_active, "back on the original lane");
    });

    let session = crate::workspace::layout::snap::RestorableSession {
        session_id: "existing-session".to_string(),
        agent_id: daruda_config::AgentDefinition::claude_default().id,
        account: AccountSelection::SystemDefault,
        lane_ref: target,
        title: None,
        prompt_preview: None,
        git_branch: None,
        cwd: second_root.path().to_path_buf(),
        last_active: std::time::SystemTime::now(),
    };

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.restore_session(session, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();

    workspace.read_with(cx, |ws, _| {
        assert_eq!(
            ws.active, target,
            "restore_session must activate the session's own lane"
        );
        assert_eq!(
            ws.active_runtime().panes.len(),
            1,
            "must reuse the pre-existing pane, not add a duplicate"
        );
        assert_eq!(ws.active_runtime().focused_pane_id, existing_pane_id);
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
            Ok(daruda_agent::ProviderUsage::new(
                daruda_store::accounts::AccountRecipeId::Claude,
                Vec::new(),
                None,
            )),
        );
        ws.claude
            .usage_by_account
            .advance_usage(key, Err(daruda_agent::FetchError::Http("500".into())));

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
