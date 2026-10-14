//! Keep-awake follows agent sessions across windows: the lease is taken when
//! an agent works, released when it settles, and every change is one deferred
//! decision. A fake lease stands in for the OS.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::*;
use crate::keep_awake::{self, KeepAwakeStatus};
use crate::platform::power::{PowerError, SleepLease};
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_config::KeepAwakeMode;
use gpui::BorrowAppContext;

/// Records every lease change and every decision that read the lease.
#[derive(Clone, Default)]
struct FakeLease {
    changes: Rc<RefCell<Vec<bool>>>,
    reads: Rc<Cell<usize>>,
    held: Rc<Cell<bool>>,
    fails: Rc<Cell<bool>>,
}

impl SleepLease for FakeLease {
    fn set_active(&mut self, active: bool) -> Result<(), PowerError> {
        self.changes.borrow_mut().push(active);
        if self.fails.get() {
            return Err(PowerError::Os(std::io::ErrorKind::PermissionDenied.into()));
        }
        self.held.set(active);
        Ok(())
    }

    fn is_held(&mut self) -> bool {
        self.reads.set(self.reads.get() + 1);
        self.held.get()
    }
}

impl FakeLease {
    fn changes(&self) -> Vec<bool> {
        self.changes.borrow().clone()
    }
}

fn set_mode(mode: KeepAwakeMode, cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            let mut config = (*store.user()).clone();
            config.desktop.keep_awake = mode;
            store.set_user_for_testing(config);
        });
    });
    cx.run_until_parked();
}

/// A registered window holding one agent-chat pane, keep-awake installed over
/// `lease` in `mode`.
fn agent_window(
    mode: KeepAwakeMode,
    lease: &FakeLease,
    cx: &mut TestAppContext,
) -> (gpui::Entity<Workspace>, PaneId) {
    let (wh, ws) = build_workspace(cx);
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.open_agent_chat_pane(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(wh.into(), ws.downgrade(), cx);
    });
    set_mode(mode, cx);
    let fake = lease.clone();
    cx.update(|cx| keep_awake::install_with(Box::new(fake), cx));
    cx.run_until_parked();
    let pane = ws.read_with(cx, |ws, _| ws.active_runtime().focused_pane_id);
    (ws, pane)
}

/// Put the pane's turn in flight or settle it, then report the change the way
/// every agent-status site does.
fn set_working(ws: &gpui::Entity<Workspace>, pane: PaneId, working: bool, cx: &mut TestAppContext) {
    ws.update(cx, |ws, cx| {
        super::agent_chat::agent_view(ws, pane).update(cx, |v, _| {
            v.set_status_for_test(daruda_chat::view::AgentSessionStatus::Connected);
            if working {
                v.set_turn_in_flight();
            } else {
                v.set_turn_idle();
            }
        });
        ws.notify_status_docks(cx);
    });
    cx.run_until_parked();
}

#[gpui::test]
async fn an_agent_turn_holds_the_lease_and_settling_releases_it(cx: &mut TestAppContext) {
    let lease = FakeLease::default();
    let (ws, pane) = agent_window(KeepAwakeMode::Agent, &lease, cx);
    assert!(lease.changes().is_empty(), "no agent works yet");

    set_working(&ws, pane, true, cx);
    assert_eq!(lease.changes(), vec![true]);
    assert_eq!(
        cx.update(|cx| keep_awake::status(cx)),
        KeepAwakeStatus::Active {
            working: 1,
            waiting: 0
        }
    );

    set_working(&ws, pane, false, cx);
    assert_eq!(lease.changes(), vec![true, false]);
    assert_eq!(
        cx.update(|cx| keep_awake::status(cx)),
        KeepAwakeStatus::Inactive
    );
}

#[gpui::test]
async fn refreshes_within_one_cycle_make_one_decision(cx: &mut TestAppContext) {
    let lease = FakeLease::default();
    let _window = agent_window(KeepAwakeMode::Agent, &lease, cx);
    lease.reads.set(0);
    cx.update(|cx| {
        keep_awake::refresh(cx);
        keep_awake::refresh(cx);
        keep_awake::refresh(cx);
    });
    cx.run_until_parked();
    assert_eq!(lease.reads.get(), 1);
}

#[gpui::test]
async fn a_refused_lease_reports_failed_and_waits_before_retrying(cx: &mut TestAppContext) {
    let lease = FakeLease::default();
    lease.fails.set(true);
    let (ws, pane) = agent_window(KeepAwakeMode::Agent, &lease, cx);
    set_working(&ws, pane, true, cx);
    assert_eq!(lease.changes(), vec![true]);
    assert_eq!(
        cx.update(|cx| keep_awake::status(cx)),
        KeepAwakeStatus::Failed
    );

    cx.update(keep_awake::refresh);
    cx.run_until_parked();
    assert_eq!(lease.changes(), vec![true], "no retry inside the pause");
    assert_eq!(
        cx.update(|cx| keep_awake::status(cx)),
        KeepAwakeStatus::Failed
    );
}

/// A lease that keeps failing is logged once; only a successful change
/// re-arms the report for the next failure.
#[gpui::test]
async fn a_lease_that_keeps_failing_is_reported_once(cx: &mut TestAppContext) {
    let lease = FakeLease::default();
    let (ws, pane) = agent_window(KeepAwakeMode::Agent, &lease, cx);
    set_working(&ws, pane, true, cx);

    // Releases are not paused, so each pass retries the refused release.
    lease.fails.set(true);
    set_working(&ws, pane, false, cx);
    cx.update(keep_awake::refresh);
    cx.run_until_parked();
    assert_eq!(lease.changes(), vec![true, false, false]);
    assert_eq!(cx.update(|cx| keep_awake::failure_reports(cx)), 1);

    lease.fails.set(false);
    cx.update(keep_awake::refresh);
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| keep_awake::status(cx)),
        KeepAwakeStatus::Inactive
    );

    lease.fails.set(true);
    set_working(&ws, pane, true, cx);
    assert_eq!(cx.update(|cx| keep_awake::failure_reports(cx)), 2);
}

/// Going inactive ends the failure pause, so work that starts again right away
/// is not stranded behind it.
#[gpui::test]
async fn going_inactive_after_a_failure_clears_the_retry_pause(cx: &mut TestAppContext) {
    let lease = FakeLease::default();
    lease.fails.set(true);
    let (ws, pane) = agent_window(KeepAwakeMode::Agent, &lease, cx);
    set_working(&ws, pane, true, cx);
    assert_eq!(lease.changes(), vec![true]);

    set_working(&ws, pane, false, cx);
    assert_eq!(
        cx.update(|cx| keep_awake::status(cx)),
        KeepAwakeStatus::Inactive
    );

    lease.fails.set(false);
    set_working(&ws, pane, true, cx);
    assert_eq!(lease.changes(), vec![true, true], "the hold is retried");
    assert_eq!(
        cx.update(|cx| keep_awake::status(cx)),
        KeepAwakeStatus::Active {
            working: 1,
            waiting: 0
        }
    );
}

/// A lease the OS dropped (the helper exited) is taken again on the next pass.
#[gpui::test]
async fn a_lease_that_died_is_acquired_again(cx: &mut TestAppContext) {
    let lease = FakeLease::default();
    let (ws, pane) = agent_window(KeepAwakeMode::Agent, &lease, cx);
    set_working(&ws, pane, true, cx);
    assert_eq!(lease.changes(), vec![true]);

    lease.held.set(false);
    cx.update(keep_awake::refresh);
    cx.run_until_parked();
    assert_eq!(lease.changes(), vec![true, true]);
}

/// A lease that died and then refuses its re-hold (a helper that keeps
/// exiting) reports `Failed` instead of being retaken on every pass.
#[gpui::test]
async fn a_dead_lease_that_refuses_its_rehold_reports_failed(cx: &mut TestAppContext) {
    let lease = FakeLease::default();
    let (ws, pane) = agent_window(KeepAwakeMode::Agent, &lease, cx);
    set_working(&ws, pane, true, cx);

    lease.held.set(false);
    lease.fails.set(true);
    cx.update(keep_awake::refresh);
    cx.run_until_parked();
    assert_eq!(lease.changes(), vec![true, true]);
    assert_eq!(
        cx.update(|cx| keep_awake::status(cx)),
        KeepAwakeStatus::Failed
    );

    cx.update(keep_awake::refresh);
    cx.run_until_parked();
    assert_eq!(
        lease.changes(),
        vec![true, true],
        "no retry inside the pause"
    );
}

#[gpui::test]
async fn switching_to_on_holds_the_lease_with_no_agent_working(cx: &mut TestAppContext) {
    let lease = FakeLease::default();
    let _window = agent_window(KeepAwakeMode::Off, &lease, cx);
    assert!(lease.changes().is_empty());

    set_mode(KeepAwakeMode::On, cx);
    assert_eq!(lease.changes(), vec![true]);
    assert_eq!(
        cx.update(|cx| keep_awake::status(cx)),
        KeepAwakeStatus::Active {
            working: 0,
            waiting: 0
        }
    );

    set_mode(KeepAwakeMode::Off, cx);
    assert_eq!(lease.changes(), vec![true, false]);
}

#[gpui::test]
async fn off_ignores_a_working_agent(cx: &mut TestAppContext) {
    let lease = FakeLease::default();
    let (ws, pane) = agent_window(KeepAwakeMode::Off, &lease, cx);
    set_working(&ws, pane, true, cx);
    assert!(lease.changes().is_empty());
}

/// The status-bar menu writes the mode through the settings store, and the
/// policy follows it without another nudge.
#[gpui::test]
async fn the_status_bar_menu_saves_the_mode_and_the_policy_follows(cx: &mut TestAppContext) {
    let lease = FakeLease::default();
    let (ws, _pane) = agent_window(KeepAwakeMode::Off, &lease, cx);

    ws.update(cx, |ws, cx| ws.set_keep_awake_mode(KeepAwakeMode::On, cx));
    cx.run_until_parked();

    let saved = cx.update(|cx| {
        crate::settings_store::SettingsStore::global(cx)
            .user()
            .desktop
            .keep_awake
    });
    assert_eq!(saved, KeepAwakeMode::On);
    assert_eq!(lease.changes(), vec![true]);
}

/// The snapshot carries the mode and the policy's status whether or not the
/// segment shows; hiding it flips only the visibility the status bar gates on.
#[gpui::test]
async fn the_status_bar_snapshot_follows_the_policy_and_the_toggle_gates_it(
    cx: &mut TestAppContext,
) {
    let lease = FakeLease::default();
    let (ws, pane) = agent_window(KeepAwakeMode::Agent, &lease, cx);
    set_working(&ws, pane, true, cx);

    let expected = crate::workspace::status_bar::KeepAwakeView {
        mode: KeepAwakeMode::Agent,
        status: KeepAwakeStatus::Active {
            working: 1,
            waiting: 0,
        },
    };
    let shown = ws.read_with(cx, |ws, cx| {
        (
            ws.keep_awake_view(cx),
            ws.mirrors
                .status_bar
                .is_visible(daruda_config::StatusBarItem::KeepAwake),
        )
    });
    assert_eq!(shown, (expected, true));

    ws.update(cx, |ws, _| {
        ws.mirrors
            .status_bar
            .toggle(daruda_config::StatusBarItem::KeepAwake)
    });
    let hidden = ws.read_with(cx, |ws, cx| {
        (
            ws.keep_awake_view(cx),
            ws.mirrors
                .status_bar
                .is_visible(daruda_config::StatusBarItem::KeepAwake),
        )
    });
    assert_eq!(hidden, (expected, false));
}
