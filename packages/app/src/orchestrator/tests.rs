use super::*;
use gpui::{BorrowAppContext as _, TestAppContext};

#[gpui::test]
fn orchestrator_opens_a_host_only_when_no_workspace_exists(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    with_config(cx, enabled_naming(None));
    cx.update(|cx| {
        assert!(WindowRegistry::all_handles(cx).is_empty());
        let (handle, weak) = host_workspace(cx).expect("host opens");
        assert!(weak.upgrade().is_some());
        assert_eq!(WindowRegistry::all_handles(cx), vec![handle]);
        let again = host_workspace_with(cx, |_| panic!("existing host must be reused")).unwrap();
        assert_eq!(again.0, handle);
    });
}

#[gpui::test]
fn orchestrator_host_open_failure_is_returned_without_a_registry_slot(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let result = host_workspace_with(cx, |_| Err(EnsureError::OpenFailed("no display".into())));
        assert!(matches!(result, Err(EnsureError::OpenFailed(reason)) if reason == "no display"));
        assert!(WindowRegistry::orchestrator(cx).is_none());
        assert!(WindowRegistry::all_handles(cx).is_empty());
    });
}

#[test]
fn a_phone_created_host_does_not_take_keyboard_focus() {
    assert!(!orchestrator_host_window_options(&daruda_config::Config::default()).focus);
}

/// Install a `SettingsStore` holding `config`. `ensure` reads it before
/// any `Workspace` exists to install one.
fn with_config(cx: &mut TestAppContext, config: daruda_config::Config) {
    cx.update(|cx| {
        crate::settings_store::SettingsStore::init(cx);
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store.set_user_for_testing(config);
        });
    });
}

fn enabled_naming(agent: Option<&str>) -> daruda_config::Config {
    daruda_config::Config {
        orchestrator: daruda_config::OrchestratorConfig {
            enabled: true,
            agent_id: agent.map(str::to_owned),
            account_id: None,
        },
        ..daruda_config::Config::default()
    }
}

/// The threat model in one assertion: a lane agent must never be handed
/// daruda's tools. `daruda_chat_new` takes an `agent` argument, so if any
/// pane could pick up the server, the self-target guard's claim to be the
/// only remaining loop would stop being true.
#[gpui::test]
fn only_the_orchestrators_pane_is_offered_the_control_server(cx: &mut TestAppContext) {
    let user = crate::test_support::workspace_with_agent_chat(cx);
    let orchestrator = crate::test_support::register_test_orchestrator(cx);

    // Driven through `update`, not `read_with`: production asks this from
    // inside `Workspace::update`, and `read_with` does not lease — a test
    // that used it would pass against a double-lease panic.
    assert!(
        offered(&user.workspace, user.pane(), cx).is_empty(),
        "no control surface up yet, so nobody gets a server — a session \
         with a server it cannot reach is worse than one with none"
    );
    cx.update(|cx| assert!(mcp_server(cx).is_none()));

    cx.update(seed_control_surface_for_test);

    // The user's pane still gets nothing.
    assert!(
        offered(&user.workspace, user.pane(), cx).is_empty(),
        "a lane agent must not be handed daruda's tools"
    );

    // The orchestrator's pane gets exactly one, and it is the real thing:
    // the shim subcommand plus the session token. Without asserting those,
    // deleting the whole authentication mechanism would still pass.
    let (_, weak) = cx
        .update(|cx| WindowRegistry::orchestrator(cx))
        .expect("registered");
    let orchestrator_ws = weak.upgrade().expect("live");
    let servers = offered(&orchestrator_ws, orchestrator.pane, cx);
    assert_eq!(servers.len(), 1, "the orchestrator gets the control server");
    let described = serde_json::to_value(&servers[0]).expect("serialize");
    assert!(
        described["name"]
            .as_str()
            .is_some_and(|n| n.starts_with(crate::surface::constants::AGENT_FACING_NAME)),
        "D20 names it per run: {described}"
    );
    assert_eq!(
        described["args"],
        serde_json::json!([crate::control::mcp::shim::SUBCOMMAND])
    );
    let env = described["env"].as_array().expect("env");
    let offered_token = env
        .iter()
        .find(|e| e["name"] == daruda_core::process_env::CONTROL_TOKEN.name())
        .and_then(|e| e["value"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("the shim authenticates with a token: {described}"));
    assert!(!offered_token.is_empty());

    // Each session gets its own token and retires the last one —
    // otherwise a shim from a discarded session keeps app-driving tools.
    let again = offered(&orchestrator_ws, orchestrator.pane, cx);
    let reissued = serde_json::to_value(&again[0]).expect("serialize");
    assert_ne!(
        reissued["env"][0]["value"].as_str(),
        Some(offered_token.as_str()),
        "a new session must not reuse the previous session's token"
    );
    // That the retired one stops working is `socket`'s half of the rule —
    // see `a_retired_token_no_longer_authenticates`.
}

/// Ask through `Workspace::update`, the way production does.
fn offered(
    workspace: &gpui::Entity<crate::workspace::Workspace>,
    pane: u64,
    cx: &mut TestAppContext,
) -> Vec<daruda_acp::McpServer> {
    workspace.update(cx, |ws, cx| ws.mcp_servers_for_pane_for_test(pane, cx))
}

/// A pane id that matches the orchestrator's *number* in a different
/// window must not inherit its tools — the same window-scoping mistake
/// the lane handles had.
#[gpui::test]
fn a_same_numbered_pane_in_another_window_gets_nothing(cx: &mut TestAppContext) {
    let user = crate::test_support::workspace_with_agent_chat(cx);
    let orchestrator = crate::test_support::register_test_orchestrator(cx);
    cx.update(seed_control_surface_for_test);
    // Ask the *user's* workspace about the orchestrator's pane number.
    assert!(
        offered(&user.workspace, orchestrator.pane, cx).is_empty(),
        "which window this is, is half the identity"
    );
}

#[test]
fn the_server_name_carries_a_runtime_suffix() {
    let name = mcp_server_name("a3f9c2d18b7e4051");
    assert_eq!(name, "daruda-a3f9c2d1");
    assert!(
        !name.contains(char::is_whitespace),
        "codex sanitizes whitespace away, so a name must not need it"
    );
}

#[test]
fn two_runtimes_get_different_names() {
    assert_ne!(
        mcp_server_name("aaaaaaaa1111"),
        mcp_server_name("bbbbbbbb2222")
    );
}

/// A short id must not panic on the slice — ids come from elsewhere, and
/// a name is not worth a crash.
#[test]
fn a_short_runtime_id_is_used_whole() {
    assert_eq!(mcp_server_name("abc"), "daruda-abc");
    assert_eq!(mcp_server_name(""), "daruda-");
}

#[gpui::test]
fn nothing_is_up_until_something_brings_it_up(cx: &mut TestAppContext) {
    with_config(cx, daruda_config::Config::default());
    cx.update(|cx| assert!(pane(cx).is_none()));
}

#[gpui::test]
fn switching_the_feature_off_refuses_even_with_one_already_up(cx: &mut TestAppContext) {
    let pane_ref = crate::test_support::register_test_orchestrator(cx);
    // `ensure` wants a surface before it will reuse a live pane, and a
    // test cannot bind the profile's.
    cx.update(seed_control_surface_for_test);
    with_config(cx, enabled_naming(None));
    cx.update(|cx| assert_eq!(ensure(cx), Ok(pane_ref), "reused while enabled"));
    with_config(cx, daruda_config::Config::default());
    cx.update(|cx| {
        assert_eq!(ensure(cx), Err(EnsureError::Disabled));
        assert!(
            pane(cx).is_some(),
            "refusing does not tear the window down; closing it is the user's call"
        );
    });
}

#[gpui::test]
fn a_disabled_orchestrator_refuses_without_opening_a_window(cx: &mut TestAppContext) {
    with_config(cx, daruda_config::Config::default());
    cx.update(|cx| {
        assert_eq!(ensure(cx), Err(EnsureError::Disabled));
        assert!(WindowRegistry::orchestrator(cx).is_none());
    });
}

/// Switched on but naming an agent the catalog does not hold: a different
/// refusal from "off", because the fix is a different one.
#[gpui::test]
fn an_unresolvable_agent_is_its_own_refusal(cx: &mut TestAppContext) {
    with_config(cx, enabled_naming(Some("no-such-agent")));
    cx.update(|cx| {
        assert_eq!(ensure(cx), Err(EnsureError::Unresolvable));
        assert!(WindowRegistry::orchestrator(cx).is_none());
    });
}
