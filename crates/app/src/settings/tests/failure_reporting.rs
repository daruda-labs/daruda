//! A user-initiated Settings action that cannot land must say so on screen.
//!
//! Every case here used to write only to the NDJSON log: the button did
//! nothing, the row kept its old value, and the window stayed silent. The
//! assertions are all the same shape — drive the action against a store or a
//! credential path that refuses the write, then require `error` to be set and
//! the underlying state to be unchanged.

use super::*;

/// Corrupt the temp `config.toml` the test store writes through, so the next
/// patch fails in `daruda_config` rather than in the UI layer under test.
fn break_settings_persistence(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let path = crate::settings_store::SettingsStore::global(cx)
            .writer_path_for_testing()
            .to_path_buf();
        std::fs::write(&path, "not = = toml").expect("corrupt the test config");
    });
}

/// Fill the token field and let the resulting `InputEvent::Change` land before
/// returning. Typing and clicking Save are two separate turns for a real user,
/// and `subscribe_draft_input` clears the banner on every change — folding both
/// into one update block would wipe the very banner under test.
fn type_token(
    wh: &WindowHandle<gpui_component::Root>,
    win: &Entity<SettingsView>,
    token: &str,
    cx: &mut TestAppContext,
) {
    let token = token.to_owned();
    wh.update(cx, |_root, window, cx| {
        win.update(cx, |w, cx| {
            w.telegram_token_input
                .update(cx, |input, cx| input.set_value(token, window, cx));
        });
    })
    .unwrap();
    cx.run_until_parked();
}

fn paired_config(chat_id: i64) -> daruda_config::Config {
    let mut config = daruda_config::Config::default();
    config.telegram.authorized_chat_id = Some(chat_id);
    config
}

#[gpui::test]
fn unpair_clears_the_pairing(cx: &mut TestAppContext) {
    let (_wh, win) = build_window_with_config(cx, paired_config(42));

    win.update(cx, |w, cx| w.unpair_telegram(cx));

    win.read_with(cx, |w, cx| {
        assert!(w.error.is_none(), "a successful unpair explains nothing");
        assert_eq!(
            crate::settings_store::SettingsStore::global(cx)
                .user()
                .telegram
                .authorized_chat_id,
            None,
        );
    });
}

#[gpui::test]
fn unpair_that_cannot_be_saved_is_reported(cx: &mut TestAppContext) {
    let (_wh, win) = build_window_with_config(cx, paired_config(42));
    break_settings_persistence(cx);

    win.update(cx, |w, cx| w.unpair_telegram(cx));

    win.read_with(cx, |w, cx| {
        assert!(
            w.error.is_some(),
            "an unpair that did not land must be visible, not only logged"
        );
        assert_eq!(
            crate::settings_store::SettingsStore::global(cx)
                .user()
                .telegram
                .authorized_chat_id,
            Some(42),
            "the pairing still stands, so the window must keep showing it"
        );
    });
}

/// The credential-store call is a parameter on these paths, so a test can
/// drive both outcomes without touching the developer's real Keychain — the
/// same hazard `keychain::read_token`'s own `cfg!(test)` guard exists for.
#[gpui::test]
fn saving_a_token_that_the_credential_store_rejects_is_reported(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    type_token(&wh, &win, "123:abc", cx);

    wh.update(cx, |_root, window, cx| {
        win.update(cx, |w, cx| {
            w.save_telegram_token_with(
                |_| Err(std::io::Error::other("keychain said no")),
                window,
                cx,
            );
        });
    })
    .unwrap();

    win.read_with(cx, |w, cx| {
        assert!(
            w.error.is_some(),
            "a rejected token write must be explained"
        );
        assert!(
            !w.telegram_token_configured,
            "nothing was stored, so the row must not claim a token is configured"
        );
        assert_eq!(
            w.telegram_token_input.read(cx).value(),
            "123:abc",
            "the typed token stays in the field so the user can retry"
        );
    });
}

#[gpui::test]
fn saving_a_token_that_lands_clears_the_field(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    type_token(&wh, &win, "123:abc", cx);

    wh.update(cx, |_root, window, cx| {
        win.update(cx, |w, cx| {
            w.save_telegram_token_with(|_| Ok(()), window, cx)
        });
    })
    .unwrap();

    win.read_with(cx, |w, cx| {
        assert!(w.error.is_none());
        assert!(w.telegram_token_configured);
        assert_eq!(w.telegram_token_input.read(cx).value(), "");
    });
}

#[gpui::test]
fn clearing_a_token_that_the_credential_store_rejects_is_reported(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.update(cx, |w, cx| {
        w.telegram_token_configured = true;
        w.clear_telegram_token_with(|| Err(std::io::Error::other("keychain said no")), cx);
    });

    win.read_with(cx, |w, _| {
        assert!(
            w.error.is_some(),
            "a rejected token delete must be explained"
        );
        assert!(
            w.telegram_token_configured,
            "the token is still there, so the row must keep saying so"
        );
    });
}

/// Accounts live under the profile's real `accounts.json`, so the persist step
/// is a parameter on these paths. `failing_persist` models what
/// `mutate_accounts_in` actually does on a bad save — it runs the mutation and
/// *then* fails to write — which is the ordering these tests are about.
mod accounts {
    use super::*;
    use daruda_store::accounts::{
        AccountId, AccountRecipeId, AccountsState, ManagedAccount, mutate_accounts_in,
        save_accounts_in,
    };

    fn managed(id: AccountId, config_dir: &std::path::Path) -> ManagedAccount {
        ManagedAccount {
            id,
            recipe: AccountRecipeId::Claude,
            email: Some("someone@example.com".to_owned()),
            organization: None,
            config_dir: config_dir.to_path_buf(),
            created_at: 1,
            last_authenticated_at: 1,
        }
    }

    /// One seeded account whose config dir exists on disk.
    fn seeded(config_dir: &std::path::Path) -> (AccountId, AccountsState) {
        std::fs::create_dir_all(config_dir).expect("seed the account config dir");
        std::fs::write(config_dir.join("marker"), b"x").expect("seed a file inside it");
        let id = AccountId::new();
        let state = AccountsState {
            accounts: vec![managed(id, config_dir)],
            ..AccountsState::default()
        };
        (id, state)
    }

    #[gpui::test]
    fn removing_an_account_deletes_its_config_dir_once_the_save_lands(cx: &mut TestAppContext) {
        let data = tempfile::tempdir().expect("tempdir");
        let config_dir = data.path().join("account-home");
        let (id, state) = seeded(&config_dir);
        save_accounts_in(data.path(), &state).expect("seed accounts.json");
        let (_wh, win) = build_window(cx);
        win.update(cx, |w, _| w.accounts = state);

        win.update(cx, |w, cx| {
            w.remove_account_with(
                id,
                |mutate| mutate_accounts_in(data.path(), |s| mutate(s)).map(|(s, ())| s),
                cx,
            );
        });

        assert!(!config_dir.exists(), "the removed account's home is gone");
        win.read_with(cx, |w, _| {
            assert!(w.error.is_none());
            assert!(w.accounts.find(id).is_none(), "and so is its row");
        });
    }

    #[gpui::test]
    fn a_removal_that_cannot_be_saved_keeps_the_account_whole(cx: &mut TestAppContext) {
        let data = tempfile::tempdir().expect("tempdir");
        let config_dir = data.path().join("account-home");
        let (id, state) = seeded(&config_dir);
        let (_wh, win) = build_window(cx);
        win.update(cx, |w, _| w.accounts = state.clone());

        win.update(cx, |w, cx| {
            w.remove_account_with(
                id,
                |mutate| {
                    let mut edited = state.clone();
                    mutate(&mut edited);
                    Err(std::io::Error::other("accounts.json could not be written"))
                },
                cx,
            );
        });

        assert!(
            config_dir.exists(),
            "the account is still listed, so its home must not have been deleted"
        );
        win.read_with(cx, |w, _| {
            assert!(w.error.is_some(), "a failed removal must be explained");
            assert!(w.accounts.find(id).is_some(), "and the row must stay");
        });
    }

    #[gpui::test]
    fn a_default_choice_that_cannot_be_saved_is_reported(cx: &mut TestAppContext) {
        let data = tempfile::tempdir().expect("tempdir");
        let (id, state) = seeded(&data.path().join("account-home"));
        let (_wh, win) = build_window(cx);
        win.update(cx, |w, _| w.accounts = state);

        win.update(cx, |w, cx| {
            w.set_default_account_with(
                AccountRecipeId::Claude,
                Some(id),
                |mutate| {
                    let mut edited = AccountsState::default();
                    mutate(&mut edited);
                    Err(std::io::Error::other("accounts.json could not be written"))
                },
                cx,
            );
        });

        win.read_with(cx, |w, _| {
            assert!(
                w.error.is_some(),
                "a failed default choice must be explained"
            );
            assert!(
                w.accounts.default_by_recipe.is_empty(),
                "nothing was written, so the mirror must not claim a default"
            );
        });
    }
}

/// The bridge's poll loop pairs a phone while Settings is open. The window has
/// to catch up on its own: `authorized_chat_id` is deliberately absent from
/// `settings_ui_patches` (pairing is not a field this window edits), so the
/// external-change diff finds nothing and the section would keep rendering the
/// pairing state it was opened with.
#[gpui::test]
fn a_background_pairing_reaches_the_open_window(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.read_with(cx, |w, _| assert_eq!(w.telegram_authorized_chat_id, None));

    cx.update(|cx| {
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            let mut cfg = (*store.user_arc()).clone();
            cfg.telegram.authorized_chat_id = Some(42);
            store.set_user_for_testing(cfg);
        });
    });
    cx.run_until_parked();

    win.read_with(cx, |w, _| {
        assert_eq!(
            w.telegram_authorized_chat_id,
            Some(42),
            "a pairing completed elsewhere must reach the open window unprompted"
        );
    });
}

/// The other half: unpairing from this window leaves the same mirror empty,
/// so the status line does not keep claiming a pairing the config no longer has.
#[gpui::test]
fn unpairing_empties_the_mirrored_pairing(cx: &mut TestAppContext) {
    let (_wh, win) = build_window_with_config(cx, paired_config(42));
    win.read_with(cx, |w, _| {
        assert_eq!(w.telegram_authorized_chat_id, Some(42))
    });

    win.update(cx, |w, cx| w.unpair_telegram(cx));
    cx.run_until_parked();

    win.read_with(cx, |w, _| assert_eq!(w.telegram_authorized_chat_id, None));
}

/// A plugin op reported into its own second banner field, which meant it never
/// reached the NDJSON log and rendered as bare coloured text rather than the
/// alert every other Settings failure uses. One channel now.
#[gpui::test]
fn a_failed_plugin_op_reports_through_the_one_banner(cx: &mut TestAppContext) {
    use crate::agent::skills::plugin_ops::{PluginAction, PluginOpError};

    let (_wh, win) = build_window(cx);
    win.update(cx, |w, cx| {
        w.plugin_ops_in_flight.insert("acme@market".to_owned());
        w.finish_plugin_op(
            "acme@market",
            PluginAction::Install,
            Err(PluginOpError::NotFound),
            cx,
        );
    });

    win.read_with(cx, |w, _| {
        let err = w
            .error
            .as_ref()
            .expect("a failed install must be explained");
        assert!(err.contains("acme@market"), "names the plugin: {err}");
        assert!(
            w.plugin_ops_in_flight.is_empty(),
            "the in-flight marker must drop so the button is clickable again"
        );
    });
}

#[gpui::test]
fn a_plugin_op_that_lands_leaves_no_banner(cx: &mut TestAppContext) {
    use crate::agent::skills::plugin_ops::PluginAction;

    let (_wh, win) = build_window(cx);
    win.update(cx, |w, cx| {
        w.error = Some("stale".into());
        w.plugin_ops_in_flight.insert("acme@market".to_owned());
        w.finish_plugin_op("acme@market", PluginAction::Install, Ok(String::new()), cx);
    });

    win.read_with(cx, |w, _| {
        assert!(w.error.is_none());
        assert!(w.plugin_ops_in_flight.is_empty());
    });
}

/// An external edit and a failed action are different questions — "someone
/// else changed this" versus "what you just clicked did not happen" — and both
/// can be live at once. The banner area used to render the conflict *instead
/// of* the error, which put the failure back where this module started: in the
/// log only.
#[gpui::test]
fn a_failure_and_a_pending_conflict_are_both_live(cx: &mut TestAppContext) {
    let (wh, win) = build_window_with_config(cx, paired_config(42));

    // A local draft on a field, then the same field edited underneath it.
    set_input(
        &wh,
        &win,
        cx,
        |window| window.terminal_font_size_input.clone(),
        "16",
    );
    cx.update(|cx| {
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            store
                .apply_patch(daruda_config::SettingsPatch::TerminalFontSize(20.0))
                .expect("external edit");
        });
    });
    let input = win.read_with(cx, |window, _| window.terminal_font_size_input.clone());
    win.update(cx, |window, cx| {
        window.persist_text_setting(&input, TextSetting::TerminalFontSize, cx);
    });
    win.read_with(cx, |w, _| {
        assert!(w.conflict.is_some(), "conflict is pending")
    });

    // Now an unrelated action fails while that choice is still outstanding.
    break_settings_persistence(cx);
    win.update(cx, |w, cx| w.unpair_telegram(cx));

    win.read_with(cx, |w, _| {
        assert!(w.conflict.is_some(), "the choice is still outstanding");
        assert!(
            w.error.is_some(),
            "and the failed unpair must be reported alongside it"
        );
    });
}

/// "Open Config File" created the directory first and opened the URL either
/// way, so a `create_dir_all` that failed handed the OS a path to a file that
/// could not be there: the editor opened nothing and the window said nothing.
#[gpui::test]
fn a_config_file_that_cannot_be_reached_is_reported(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);

    let opened = win.update(cx, |w, cx| {
        w.open_config_file_with(|_| Err(std::io::Error::other("read-only file system")), cx)
    });

    assert!(
        !opened,
        "there is nothing to open, so nothing must be opened"
    );
    win.read_with(cx, |w, _| {
        assert!(w.error.is_some(), "and the user must be told why");
    });
}
