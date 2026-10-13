use super::*;

#[gpui::test]
fn basic_toggles_and_section_focus(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    win.read_with(cx, |w, _| {
        assert_eq!(w.active_section(), BuiltinSection::General);
    });

    let (initial_blur, initial_close) =
        win.read_with(cx, |w, _| (w.window_blur, w.close_pane_on_exit));
    win.update(cx, |w, cx| {
        w.window_blur = !w.window_blur;
        w.close_pane_on_exit = !w.close_pane_on_exit;
        cx.notify();
    });
    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("toggled settings must validate");
        assert_eq!(cfg.window.blur, !initial_blur);
        assert_eq!(cfg.shell.close_pane_on_exit, !initial_close);
    });

    let win_clone = win.clone();
    cx.update_window(wh.into(), |_, window, cx| {
        win_clone.update(cx, |w, cx| {
            w.focus_section(BuiltinSection::Font, window, cx);
        });
    })
    .unwrap();
    win.read_with(cx, |w, _| {
        assert_eq!(w.active_section(), BuiltinSection::Font);
    });

    let settings_for_root = std::cell::RefCell::new(None);
    let _wh = cx.add_window(|window, cx| {
        let settings =
            cx.new(|cx| new_test_view(BuiltinSection::Keymap, FakeServices::default(), window, cx));
        *settings_for_root.borrow_mut() = Some(settings.clone());
        daruda_ui::Root::new(settings, window, cx)
    });
    let win = settings_for_root.borrow().clone().unwrap();
    win.read_with(cx, |w, _| {
        assert_eq!(w.active_section(), BuiltinSection::Keymap);
    });
}

/// Regression: `validate()` must never let clicking Save revert a
/// Telegram pairing that completed asynchronously (the bridge's poll
/// loop, via `InboundAction::Paired`) while the Settings window was
/// still open. `base_config` is a snapshot taken at window-open time,
/// so the fix re-reads `authorized_chat_id` live from `SettingsStore`
/// instead of trusting that stale snapshot.
#[gpui::test]
fn validate_does_not_revert_background_pairing(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);

    // Simulate the poll loop's `InboundAction::Paired` persist step —
    // it mutates `SettingsStore`'s live config directly, bypassing this
    // window entirely, exactly as `telegram::global::dispatch_action`
    // does. Uses `set_user_for_testing` (in-memory only), NOT the real
    // `patch_user` — `patch_user` calls `daruda_config::patch_config_file`,
    // which writes to the actual `config_path()` on disk regardless of
    // test context (there is no test-mode redirect), so calling it here
    // would silently overwrite the developer's real `~/Library/Application
    // Support/daruda/config.toml` on every test run. This bug shipped and
    // was caught by a live user report of a repeatedly-clobbered config file.
    cx.update(|cx| {
        use gpui::BorrowAppContext as _;
        cx.update_global::<crate::store::SettingsStore, _>(|store, _| {
            let mut cfg = (*store.user_arc()).clone();
            cfg.telegram.authorized_chat_id = Some(42);
            store.set_user_for_testing(cfg);
        });
    });

    win.read_with(cx, |w, cx| {
        let cfg = w
            .validate(cx)
            .expect("defaults plus a live pairing must still validate");
        assert_eq!(
            cfg.telegram.authorized_chat_id,
            Some(42),
            "Save must not revert a pairing that completed while Settings was open"
        );
    });
}

#[gpui::test]
fn telegram_enabled_toggle_round_trips_through_validate(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.read_with(cx, |w, cx| {
        assert!(!w.validate(cx).unwrap().telegram.enabled);
    });

    win.update(cx, |w, cx| {
        w.telegram_enabled = true;
        cx.notify();
    });

    win.read_with(cx, |w, cx| {
        assert!(w.validate(cx).unwrap().telegram.enabled);
    });
}

#[gpui::test]
async fn the_orchestrator_pickers_follow_a_change_made_while_settings_is_open(
    cx: &mut TestAppContext,
) {
    let (wh, win) = build_window(cx);
    let account = daruda_store::accounts::ManagedAccount {
        id: daruda_store::accounts::AccountId::new(),
        recipe: daruda_store::accounts::AccountRecipeId::Claude,
        email: Some("someone@example.com".to_string()),
        organization: None,
        config_dir: std::path::PathBuf::from("/tmp/daruda-test-account"),
        created_at: 0,
        last_authenticated_at: 0,
    };
    let account_value: SharedString = account.id.0.to_string().into();

    let can_pick = |value: &SharedString,
                    select: fn(&SettingsView) -> &Entity<SelectState>,
                    cx: &mut TestAppContext|
     -> bool {
        let value = value.clone();
        cx.update_window(wh.into(), |_, window, cx| {
            let entity = select(win.read(cx)).clone();
            entity.update(cx, |s, cx| {
                s.set_selected_value(&value, window, cx);
                s.selected_value().is_some()
            })
        })
        .expect("window is live")
    };

    assert!(
        !can_pick(&account_value, |w| &w.orchestrator_account_select, cx),
        "not offered before the account exists"
    );

    let mut state = daruda_store::accounts::AccountsState::default();
    state.accounts.push(account.clone());
    cx.update_window(wh.into(), |_, window, cx| {
        win.update(cx, |w, cx| w.set_accounts(state, false, window, cx));
    })
    .expect("window is live");
    cx.run_until_parked();
    assert!(
        can_pick(&account_value, |w| &w.orchestrator_account_select, cx),
        "a signed-in account has to be selectable without reopening Settings"
    );

    let added: SharedString = "codex-acp".into();
    assert!(!can_pick(&added, |w| &w.orchestrator_agent_select, cx));
    let with_codex = daruda_config::Config {
        agents: vec![daruda_config::AgentEntry::preset_with(
            "codex-acp".to_string(),
            Default::default(),
        )],
        ..daruda_config::Config::default()
    };
    cx.update(|cx| {
        cx.update_global::<crate::store::SettingsStore, _>(|store, _| {
            store.set_user_for_testing(with_codex.clone());
        });
    });
    cx.update_window(wh.into(), |_, window, cx| {
        win.update(cx, |w, cx| {
            w.load_settings_patch(
                &daruda_config::SettingsPatch::AgentCatalog(with_codex.agents.clone()),
                &with_codex,
                window,
                cx,
            );
        });
    })
    .expect("window is live");
    assert!(
        can_pick(&added, |w| &w.orchestrator_agent_select, cx),
        "an added agent has to become selectable"
    );
}

#[gpui::test]
async fn copy_botfather_commands_writes_the_registration_block(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);

    cx.update(|cx| {
        win.update(cx, |w, cx| w.copy_botfather_commands_for_test(cx));
    });

    let text = cx
        .read_from_clipboard()
        .expect("clipboard populated")
        .text()
        .expect("clipboard item is text");
    // That the block names every command is
    // `control::spec::tests::the_botfather_registration_lists_every_command_name`'s
    // invariant. This test owns the copy itself — asserted exactly, so a
    // truncated or doubled paste fails rather than passing a shape check.
    assert_eq!(text, s::control::botfather_commands());
    for line in text.lines() {
        assert!(line.contains(" - "), "not a BotFather row: {line}");
    }
    win.read_with(cx, |w, _cx| assert!(w.telegram_botfather_copied()));
}

#[gpui::test]
async fn copy_telegram_pair_command_writes_clipboard_and_reverts_label(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);

    // Write something else first so we can verify the copy overwrites it.
    cx.update(|cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("sentinel".to_string()));
    });

    cx.update(|cx| {
        win.update(cx, |w, cx| {
            w.copy_telegram_pair_command_for_test("AB12CD", cx)
        });
    });

    let actual = cx.read_from_clipboard().expect("clipboard populated");
    let text = actual.text().expect("clipboard item is text");
    assert_eq!(text, "/pair AB12CD");

    win.read_with(cx, |w, _cx| {
        assert!(
            w.telegram_pair_command_copied(),
            "Copied confirmation should be active"
        );
    });

    // Past the 1 s revert window — virtual clock so the test is
    // deterministic.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(1500));
    cx.run_until_parked();

    win.read_with(cx, |w, _cx| {
        assert!(
            !w.telegram_pair_command_copied(),
            "Copied label should have reverted to Copy"
        );
    });
}

#[gpui::test]
fn focus_section_resets_scroll(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    let win_a = win.clone();
    cx.update_window(wh.into(), |_, window, cx| {
        win_a.update(cx, |w, cx| {
            // First, set a non-zero scroll while on the General page
            // (so the test exercises the reset path inside focus_section).
            w.scroll_handle
                .set_offset(gpui::point(gpui::px(0.), gpui::px(-100.)));
            w.focus_section(BuiltinSection::Appearance, window, cx);
        });
    })
    .unwrap();
    win.read_with(cx, |w, _| {
        assert_eq!(w.scroll_handle.offset().y, gpui::px(0.));
    });
}
