use super::*;
use crate::control::approval::{self, ApprovalOutcome};
use crate::platform::presence::AwaySignal;
use crate::remote_channel::global::Delivery;
use crate::settings_store::SettingsStore;
use std::time::{Duration, Instant};

fn away(cx: &mut App) {
    crate::app_presence::seed_for_test(
        AwaySignal::HERE.observe(
            false,
            Some(Duration::from_secs(300)),
            Instant::now() - Duration::from_secs(120),
        ),
        false,
        Some(Duration::from_secs(300)),
        cx,
    );
}

#[gpui::test]
fn the_presence_pump_relays_waiting_approvals_without_a_workspace(cx: &mut gpui::TestAppContext) {
    let (mut remote, mut telegram, id) = cx.update(|cx| {
        let remote = crate::remote_channel::global::tests::setup(cx);
        let telegram = crate::telegram::global::install_for_test(true, Some(42), cx);
        let mut config = SettingsStore::global(cx).user().clone();
        config.telegram.enabled = true;
        config.telegram.authorized_chat_id = Some(42);
        config.telegram.only_when_away = true;
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(config);
        let (id, _) = approval::request(
            "Create worktree".into(),
            Delivery::Presence { away: false },
            cx,
        );
        away(cx);
        spawn_presence_and_telegram_pump(cx);
        (remote, telegram, id)
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(15));
    cx.run_until_parked();
    let mut channels = Vec::new();
    while let Ok(pending) = remote.try_recv() {
        channels.push(pending.id);
    }
    channels.sort();
    assert_eq!(channels, ["personal", "work"]);
    assert!(matches!(
        telegram.try_recv(),
        Ok(crate::telegram::bridge::Outbound::Approval(_))
    ));
    cx.executor().advance_clock(Duration::from_secs(15));
    cx.run_until_parked();
    assert!(remote.try_recv().is_err());
    assert!(telegram.try_recv().is_err());
    cx.update(|cx| assert!(approval::withdraw(id, cx)));
}

#[gpui::test]
fn the_presence_pump_does_not_relay_a_cancelled_approval(cx: &mut gpui::TestAppContext) {
    let (mut remote, mut telegram, answer) = cx.update(|cx| {
        let mut remote = crate::remote_channel::global::tests::setup(cx);
        let telegram = crate::telegram::global::install_for_test(true, Some(42), cx);
        let mut config = SettingsStore::global(cx).user().clone();
        config.telegram.enabled = true;
        config.telegram.authorized_chat_id = Some(42);
        cx.global_mut::<SettingsStore>()
            .set_user_for_testing(config);
        let (id, answer) = approval::request(
            "Create worktree".into(),
            Delivery::Presence { away: false },
            cx,
        );
        assert_eq!(remote.try_recv().unwrap().id, "personal");
        assert!(approval::withdraw(id, cx));
        away(cx);
        spawn_presence_and_telegram_pump(cx);
        (remote, telegram, answer)
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(15));
    cx.run_until_parked();
    assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::Withdrawn));
    assert!(remote.try_recv().is_err());
    assert!(telegram.try_recv().is_err());
}

#[gpui::test]
fn an_away_relay_near_expiry_leaves_two_minutes_to_answer(cx: &mut gpui::TestAppContext) {
    let (mut remote, mut telegram, id, answer) = cx.update(|cx| {
        let mut remote = crate::remote_channel::global::tests::setup(cx);
        let telegram = crate::telegram::global::install_for_test(true, Some(42), cx);
        let (id, answer) = approval::request(
            "Create worktree".into(),
            Delivery::Presence { away: false },
            cx,
        );
        assert_eq!(remote.try_recv().unwrap().id, "personal");
        (remote, telegram, id, answer)
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(284));
    cx.update(|cx| {
        away(cx);
        spawn_presence_and_telegram_pump(cx);
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(15));
    cx.run_until_parked();
    assert_eq!(remote.try_recv().unwrap().id, "work");
    assert!(telegram.try_recv().is_ok());
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert!(answer.is_empty());
    cx.executor().advance_clock(Duration::from_secs(117));
    cx.run_until_parked();
    cx.update(|cx| assert!(approval::is_waiting(id, cx)));
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(answer.try_recv(), Ok(ApprovalOutcome::TimedOut));
}
