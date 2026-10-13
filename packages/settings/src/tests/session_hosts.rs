use super::*;

// ---- Session Hosts section ----

/// Test-only — click "Add Host".
fn add_session_host(
    wh: &WindowHandle<daruda_ui::Root>,
    win: &Entity<SettingsView>,
    cx: &mut TestAppContext,
) {
    let win = win.clone();
    wh.update(cx, |_root, window, cx| {
        win.update(cx, |w, cx| w.add_session_host_row(window, cx));
    })
    .expect("settings window should still be open during the test");
}

/// Test-only — pick `kind` in one session-host row's kind dropdown.
fn select_session_host_kind(
    wh: &WindowHandle<daruda_ui::Root>,
    win: &Entity<SettingsView>,
    cx: &mut TestAppContext,
    index: usize,
    kind: &str,
) {
    let state = win.read_with(cx, |w, _| {
        w.session_host_row(index).unwrap().kind_select.clone()
    });
    let value = SharedString::from(kind.to_owned());
    wh.update(cx, |_root, window, cx| {
        state.update(cx, |s, cx| s.set_selected_value(&value, window, cx));
    })
    .expect("settings window should still be open during the test");
}

/// Test-only — write `value` into one session-host row's input.
fn set_session_host_row_input(
    wh: &WindowHandle<daruda_ui::Root>,
    win: &Entity<SettingsView>,
    cx: &mut TestAppContext,
    index: usize,
    field: fn(&SessionHostRow) -> Entity<InputState>,
    value: &str,
) {
    let state = win.read_with(cx, |w, _| field(w.session_host_row(index).unwrap()));
    wh.update(cx, |_root, window, cx| {
        state.update(cx, |i, cx_state| {
            i.set_value(value.to_owned(), window, cx_state)
        });
    })
    .expect("settings window should still be open during the test");
}

#[gpui::test]
fn validate_session_host_catalog_cases(cx: &mut TestAppContext) {
    let (_wh, win) = build_window(cx);
    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("an empty registry is a valid state");
        assert!(cfg.session_hosts.is_empty());
    });

    let (wh, win) = build_window(cx);
    add_session_host(&wh, &win, cx);
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.label_input.clone(), "Build box");
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.target_input.clone(), "vm-work");
    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("a filled-in ssh row must validate");
        assert_eq!(cfg.session_hosts.len(), 1);
        assert_eq!(cfg.session_hosts[0].label, "Build box");
        assert_eq!(
            cfg.session_hosts[0].kind,
            daruda_config::SessionHostKind::Ssh {
                target: "vm-work".to_string()
            }
        );
    });

    let (wh, win) = build_window(cx);
    add_session_host(&wh, &win, cx);
    select_session_host_kind(&wh, &win, cx, 0, "docker");
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.label_input.clone(), "Dev container");
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.container_input.clone(), "dev-1");
    win.read_with(cx, |w, cx| {
        let cfg = w
            .validate(cx)
            .expect("a filled-in docker row must validate");
        assert_eq!(cfg.session_hosts.len(), 1);
        assert_eq!(
            cfg.session_hosts[0].kind,
            daruda_config::SessionHostKind::Docker {
                container: "dev-1".to_string()
            }
        );
    });

    let (wh, win) = build_window(cx);
    add_session_host(&wh, &win, cx);
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.target_input.clone(), "vm-work");
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains('1'));
    });

    let (wh, win) = build_window(cx);
    add_session_host(&wh, &win, cx);
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.label_input.clone(), "Build Box");
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.target_input.clone(), "vm-work");
    add_session_host(&wh, &win, cx);
    set_session_host_row_input(&wh, &win, cx, 1, |r| r.label_input.clone(), "  build box  ");
    set_session_host_row_input(&wh, &win, cx, 1, |r| r.target_input.clone(), "vm-other");
    win.read_with(cx, |w, cx| {
        let err = w.validate(cx).unwrap_err();
        assert!(err.contains("build box") || err.contains("Build Box"));
    });

    // `target`/`container` are validated through the exact same
    // `lane::session_host::checked_bare_word` `SessionHostModal` uses.
    let (wh, win) = build_window(cx);
    add_session_host(&wh, &win, cx);
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.label_input.clone(), "Box");
    set_session_host_row_input(
        &wh,
        &win,
        cx,
        0,
        |r| r.target_input.clone(),
        "box; rm -rf /",
    );
    win.read_with(cx, |w, cx| {
        assert!(w.validate(cx).is_err());
    });

    let (wh, win) = build_window(cx);
    add_session_host(&wh, &win, cx);
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.label_input.clone(), "Box");
    win.read_with(cx, |w, cx| {
        assert!(w.validate(cx).is_err());
    });
}

/// Deleting a row that was loaded from config and saving must record a
/// tombstone for it.
#[gpui::test]
fn deleting_a_loaded_row_and_saving_records_a_tombstone(cx: &mut TestAppContext) {
    let id = daruda_store::project::SessionHostId::new();
    let config = daruda_config::Config {
        session_hosts: vec![daruda_config::SessionHostEntry {
            id,
            label: "Build box".to_string(),
            kind: daruda_config::SessionHostKind::Ssh {
                target: "vm-work".to_string(),
            },
        }],
        ..daruda_config::Config::default()
    };
    let (_wh, win) = build_window_with_config(cx, config);
    win.update(cx, |w, cx| w.remove_session_host_row(0, cx));
    win.read_with(cx, |w, cx| {
        let cfg = w
            .validate(cx)
            .expect("removing the only row is a valid save");
        assert!(cfg.session_hosts.is_empty());
        assert_eq!(cfg.session_host_tombstones.len(), 1);
        assert_eq!(cfg.session_host_tombstones[0].old_id, id);
        assert_eq!(cfg.session_host_tombstones[0].value, "vm-work");
        assert_eq!(cfg.session_host_tombstones[0].redirected_to, None);
    });
}

/// Re-adding a host with the same `(kind, value)` as a just-deleted one sets
/// `redirected_to` on the most recent matching tombstone, so a lane still
/// referencing the deleted id resolves through the redirect chain.
#[gpui::test]
fn readding_the_same_target_redirects_the_deleted_tombstone(cx: &mut TestAppContext) {
    let old_id = daruda_store::project::SessionHostId::new();
    let config = daruda_config::Config {
        session_hosts: vec![daruda_config::SessionHostEntry {
            id: old_id,
            label: "Build box".to_string(),
            kind: daruda_config::SessionHostKind::Ssh {
                target: "vm-work".to_string(),
            },
        }],
        ..daruda_config::Config::default()
    };
    let (wh, win) = build_window_with_config(cx, config);
    win.update(cx, |w, cx| w.remove_session_host_row(0, cx));
    add_session_host(&wh, &win, cx);
    set_session_host_row_input(
        &wh,
        &win,
        cx,
        0,
        |r| r.label_input.clone(),
        "Build box again",
    );
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.target_input.clone(), "vm-work");

    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("the recreated row must validate");
        assert_eq!(cfg.session_hosts.len(), 1);
        let new_id = cfg.session_hosts[0].id;
        assert_eq!(cfg.session_host_tombstones.len(), 1);
        assert_eq!(cfg.session_host_tombstones[0].old_id, old_id);
        assert_eq!(cfg.session_host_tombstones[0].redirected_to, Some(new_id));
    });
}

/// A one-row SSH catalog plus the lane host linked to it — the starting
/// state for the retype tests below.
fn ssh_catalog_config(
    id: daruda_store::project::SessionHostId,
) -> (
    daruda_config::Config,
    daruda_store::project::LaneSessionHost,
) {
    let config = daruda_config::Config {
        session_hosts: vec![daruda_config::SessionHostEntry {
            id,
            label: "Build box".to_string(),
            kind: daruda_config::SessionHostKind::Ssh {
                target: "vm-work".to_string(),
            },
        }],
        ..daruda_config::Config::default()
    };
    let lane_host = daruda_store::project::LaneSessionHost::Ssh {
        target: "vm-work".to_string(),
        session_path: "/srv/app".to_string(),
        registry_id: Some(id),
    };
    (config, lane_host)
}

/// Changing a loaded row's Type retires its id. A lane's `registry_id`
/// resolves on id *and* kind, so keeping the id would leave every linked lane
/// unresolvable with nothing recorded anywhere; retiring it records the
/// removal and surfaces the lane as Orphaned (banner + "keep current" path)
/// instead of silently stale.
#[gpui::test]
fn retyping_a_loaded_row_retires_its_id_and_orphans_the_linked_lane(cx: &mut TestAppContext) {
    let old_id = daruda_store::project::SessionHostId::new();
    let (config, lane_host) = ssh_catalog_config(old_id);
    let (wh, win) = build_window_with_config(cx, config);
    select_session_host_kind(&wh, &win, cx, 0, "docker");
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.container_input.clone(), "dev-1");

    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("a retyped row must still validate");
        assert_eq!(cfg.session_hosts.len(), 1);
        assert_ne!(
            cfg.session_hosts[0].id, old_id,
            "a row that switched Type must not keep the id SSH lanes point at"
        );
        assert_eq!(cfg.session_host_tombstones.len(), 1);
        assert_eq!(cfg.session_host_tombstones[0].old_id, old_id);
        assert_eq!(
            cfg.session_host_tombstones[0].kind,
            daruda_config::SessionHostKind::Ssh {
                target: "vm-work".to_string()
            }
        );
        assert_eq!(
            cfg.session_host_tombstones[0].redirected_to, None,
            "an SSH lane must never be redirected onto a Docker entry"
        );

        use daruda_project::lane::session_host;
        assert_eq!(
            session_host::registry_link_status(&lane_host, &cfg.session_hosts),
            session_host::LinkStatus::Orphaned
        );
        assert_eq!(
            session_host::effective_session_host(
                Some(&lane_host),
                None,
                &daruda_config::AgentLaunch::Raw("adapter".to_string()),
                &cfg.session_hosts,
                &cfg.session_host_tombstones,
            ),
            Ok(lane_host),
            "an orphaned lane keeps its cached target, never the Docker one"
        );
    });
}

/// What the retype's tombstone buys: registering the SSH host again later
/// redirects the retired id, so the linked lane resolves to the new entry
/// instead of sitting on a stale cache forever.
#[gpui::test]
fn re_registering_a_retyped_host_heals_the_linked_lane(cx: &mut TestAppContext) {
    let old_id = daruda_store::project::SessionHostId::new();
    let (config, lane_host) = ssh_catalog_config(old_id);

    let (wh, win) = build_window_with_config(cx, config);
    select_session_host_kind(&wh, &win, cx, 0, "docker");
    set_session_host_row_input(&wh, &win, cx, 0, |r| r.container_input.clone(), "dev-1");
    let after_retype = win.read_with(cx, |w, cx| w.validate(cx).expect("retype must validate"));

    // Second Save, over the config the first one produced: the user
    // re-registers the SSH host they moved off of.
    let (wh, win) = build_window_with_config(cx, after_retype);
    add_session_host(&wh, &win, cx);
    set_session_host_row_input(
        &wh,
        &win,
        cx,
        1,
        |r| r.label_input.clone(),
        "Build box (ssh)",
    );
    set_session_host_row_input(&wh, &win, cx, 1, |r| r.target_input.clone(), "vm-work");

    win.read_with(cx, |w, cx| {
        let cfg = w.validate(cx).expect("re-registering must validate");
        let new_ssh_id = cfg
            .session_hosts
            .iter()
            .find(|e| matches!(e.kind, daruda_config::SessionHostKind::Ssh { .. }))
            .expect("the re-registered ssh entry")
            .id;

        use daruda_project::lane::session_host;
        assert_eq!(
            session_host::resolved_registry_id(
                &lane_host,
                &cfg.session_hosts,
                &cfg.session_host_tombstones,
            ),
            Some(new_ssh_id),
            "the retired id must redirect onto the re-registered ssh entry"
        );
    });
}

#[gpui::test]
fn removing_a_session_host_row_updates_the_row_list_immediately(cx: &mut TestAppContext) {
    let (wh, win) = build_window(cx);
    add_session_host(&wh, &win, cx);
    add_session_host(&wh, &win, cx);
    win.read_with(cx, |w, _| assert_eq!(w.session_host_rows().count(), 2));
    win.update(cx, |w, cx| w.remove_session_host_row(0, cx));
    win.read_with(cx, |w, _| assert_eq!(w.session_host_rows().count(), 1));
}

// ---- Session Hosts tab cycle ----

/// Test-only — open the Session Hosts page, landing focus where a sidebar
/// click would.
fn open_session_hosts_section(
    wh: &WindowHandle<daruda_ui::Root>,
    win: &Entity<SettingsView>,
    cx: &mut TestAppContext,
) {
    let win = win.clone();
    wh.update(cx, |_root, window, cx| {
        win.update(cx, |w, cx| {
            w.focus_section(BuiltinSection::SessionHosts, window, cx)
        });
    })
    .expect("settings window should still be open during the test");
}

/// Test-only — one Tab (`forward`) / Shift-Tab press.
fn press_tab(
    wh: &WindowHandle<daruda_ui::Root>,
    win: &Entity<SettingsView>,
    cx: &mut TestAppContext,
    forward: bool,
) {
    let win = win.clone();
    wh.update(cx, |_root, window, cx| {
        win.update(cx, |w, cx| w.focus_next_input(forward, window, cx));
    })
    .expect("settings window should still be open during the test");
}

/// Test-only — whether one session-host row's input currently holds focus.
fn session_host_input_is_focused(
    wh: &WindowHandle<daruda_ui::Root>,
    win: &Entity<SettingsView>,
    cx: &mut TestAppContext,
    index: usize,
    field: fn(&SessionHostRow) -> Entity<InputState>,
) -> bool {
    let state = win.read_with(cx, |w, _| field(w.session_host_row(index).unwrap()));
    wh.update(cx, |_root, window, cx| {
        state.read(cx).focus_handle(cx).is_focused(window)
    })
    .expect("settings window should still be open during the test")
}

fn focus_session_host_input(
    wh: &WindowHandle<daruda_ui::Root>,
    win: &Entity<SettingsView>,
    cx: &mut TestAppContext,
    index: usize,
    field: fn(&SessionHostRow) -> Entity<InputState>,
) {
    let state = win.read_with(cx, |w, _| field(w.session_host_row(index).unwrap()));
    wh.update(cx, |_root, window, cx| {
        state.read(cx).focus_handle(cx).focus(window, cx);
    })
    .expect("settings window should still be open during the test");
}

fn session_host_kind_is_focused(
    wh: &WindowHandle<daruda_ui::Root>,
    win: &Entity<SettingsView>,
    cx: &mut TestAppContext,
    index: usize,
) -> bool {
    let state = win.read_with(cx, |w, _| {
        w.session_host_row(index).unwrap().kind_select.clone()
    });
    wh.update(cx, |_root, window, cx| {
        state.read(cx).focus_handle(cx).is_focused(window)
    })
    .expect("settings window should still be open during the test")
}

/// Tab has to walk a row's fields, not sit on its label: the section's
/// precomputed focus list holds only the first field, so the cycle is built
/// from the live rows instead.
#[gpui::test]
fn tab_cycle_covers_session_host_rows(cx: &mut TestAppContext) {
    let (wh, win) = build_window_with_config(cx, one_ssh_row_config());
    open_session_hosts_section(&wh, &win, cx);
    assert!(
        session_host_input_is_focused(&wh, &win, cx, 0, |r| r.label_input.clone()),
        "opening the page lands on the first row's label"
    );

    press_tab(&wh, &win, cx, true);
    assert!(
        session_host_kind_is_focused(&wh, &win, cx, 0),
        "Tab from the label must reach the row's kind select"
    );
    press_tab(&wh, &win, cx, true);
    assert!(
        session_host_input_is_focused(&wh, &win, cx, 0, |r| r.target_input.clone()),
        "Tab from the kind select must reach the row's target field"
    );

    press_tab(&wh, &win, cx, false);
    assert!(session_host_kind_is_focused(&wh, &win, cx, 0));
    press_tab(&wh, &win, cx, false);
    assert!(
        session_host_input_is_focused(&wh, &win, cx, 0, |r| r.label_input.clone()),
        "Shift-Tab must walk back to the label"
    );

    add_session_host(&wh, &win, cx);
    select_session_host_kind(&wh, &win, cx, 1, "docker");

    press_tab(&wh, &win, cx, true);
    assert!(session_host_kind_is_focused(&wh, &win, cx, 0));
    press_tab(&wh, &win, cx, true);
    assert!(
        session_host_input_is_focused(&wh, &win, cx, 0, |r| r.target_input.clone()),
        "the first row's value field comes after its label"
    );
    press_tab(&wh, &win, cx, true);
    // The second row's Remove button is a real tab stop and precedes its
    // fields in render order.
    press_tab(&wh, &win, cx, true);
    assert!(
        session_host_input_is_focused(&wh, &win, cx, 1, |r| r.label_input.clone()),
        "the cycle carries on into the second row"
    );
    press_tab(&wh, &win, cx, true);
    assert!(session_host_kind_is_focused(&wh, &win, cx, 1));
    press_tab(&wh, &win, cx, true);
    assert!(
        session_host_input_is_focused(&wh, &win, cx, 1, |r| r.container_input.clone()),
        "a Docker row's container field is the one on screen"
    );
    assert!(
        !session_host_input_is_focused(&wh, &win, cx, 1, |r| r.target_input.clone()),
        "the hidden target field must never take focus"
    );
    let (wh, win) = build_window(cx);
    open_session_hosts_section(&wh, &win, cx);
    add_session_host(&wh, &win, cx);

    focus_session_host_input(&wh, &win, cx, 0, |r| r.label_input.clone());
    assert!(
        session_host_input_is_focused(&wh, &win, cx, 0, |r| r.label_input.clone()),
        "the first field of a newly added row can enter the native tab order"
    );
    press_tab(&wh, &win, cx, true);
    assert!(session_host_kind_is_focused(&wh, &win, cx, 0));
    press_tab(&wh, &win, cx, true);
    assert!(
        session_host_input_is_focused(&wh, &win, cx, 0, |r| r.target_input.clone()),
        "and carries on to its value field"
    );
}
