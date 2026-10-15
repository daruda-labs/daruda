//! `[usage] stop_on_overage` end to end through the event fold: a billed
//! usage report stops the turn and says so beyond the pane's own banner.

use gpui::{AppContext as _, BorrowAppContext as _, TestAppContext};

use super::super::build_workspace_with;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::TabEntry;
use crate::workspace::main_area::pane_tree::PaneId;
use crate::workspace::main_area::pane_tree::PaneLayout;
use daruda_chat::view::{AgentChatView, AgentSessionStatus};
use daruda_store::project::LaneRef;

const DEDUP: &str = "agent_chat.overage_stopped";

fn billed_usage() -> daruda_acp::AcpEvent {
    daruda_acp::AcpEvent::UsageChanged(daruda_acp::UsageView {
        used: 1,
        size: 2,
        cost: None,
        rate_limit: Some(daruda_acp::RateLimitView {
            window: Some(daruda_acp::RateLimitWindow::FiveHour),
            resets_at: Some(1_791_340_200),
            is_using_overage: true,
        }),
    })
}

struct Fixture {
    ws: gpui::Entity<Workspace>,
    pane: PaneId,
    home: LaneRef,
    _dir: tempfile::TempDir,
}

/// A busy chat pane with one queued prompt, parked in a lane the user has
/// moved away from, with the setting at `enabled`.
fn busy_pane_off_screen(cx: &mut TestAppContext, enabled: bool) -> Fixture {
    busy_pane_off_screen_with(cx, enabled, daruda_config::Config::default())
}

fn busy_pane_off_screen_with(
    cx: &mut TestAppContext,
    enabled: bool,
    config: daruda_config::Config,
) -> Fixture {
    // Built rather than opened, so no adapter is spawned.
    let dir = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(dir.path());
    let (wh, ws) = build_workspace_with(cx, &config, Some(project));
    cx.run_until_parked();
    let pane = cx
        .update_window(wh.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| {
                let pane = ws.create_agent_chat_pane(
                    None,
                    None,
                    daruda_config::AgentDefinition::claude_default().id,
                    None,
                    window,
                    cx,
                );
                let pane_id = pane.id;
                ws.active_runtime_mut().panes.push(pane);
                let tab_id = ws.alloc_id();
                ws.active_runtime_mut().tabs.push(TabEntry {
                    id: tab_id,
                    layout: PaneLayout::Pane(pane_id),
                    last_focused_pane: pane_id,
                    user_label: None,
                });
                pane_id
            })
        })
        .unwrap();
    cx.update(|cx| {
        cx.update_global::<crate::settings_store::SettingsStore, _>(|store, _| {
            let mut cfg = (*store.user()).clone();
            cfg.usage.stop_on_overage = enabled;
            cfg.telegram = config.telegram.clone();
            store.set_user_for_testing(cfg);
        });
        crate::overage_policy::OveragePolicy::sync(cx);
    });
    let (home, other) = ws.update(cx, |ws, cx| {
        view(ws, pane).update(cx, |v, cx| {
            v.set_status_for_test(AgentSessionStatus::Connected);
            v.set_turn_in_flight();
            v.fill_queue_for_test(1);
            // Open the activity span, as the pump's tick would.
            assert_eq!(v.tick_activity(std::time::Instant::now(), cx), None);
        });
        let home = ws.active;
        let lane_id = ws.alloc_id();
        let mut lane = crate::lane::Lane::default_for_project(lane_id, dir.path().join("other"));
        lane.tab_order = 1;
        ws.project_for_mut(home.project).unwrap().lanes.push(lane);
        let other = LaneRef {
            project: home.project,
            lane: lane_id,
        };
        (home, other)
    });
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_lane(other, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    Fixture {
        ws,
        pane,
        home,
        _dir: dir,
    }
}

fn view(ws: &Workspace, pane: PaneId) -> gpui::Entity<AgentChatView> {
    ws.agent_chat_view(pane).expect("chat pane").clone()
}

fn fold_billed(f: &Fixture, cx: &mut TestAppContext) {
    f.ws.update(cx, |ws, cx| {
        ws.fold_agent_chat_event(f.pane, billed_usage(), cx);
    });
    cx.run_until_parked();
}

fn overage_toasts(ws: &Workspace) -> usize {
    ws.error_history()
        .iter()
        .filter(|r| r.dedup_key.as_deref() == Some(DEDUP))
        .count()
}

fn parked_count(v: &AgentChatView) -> usize {
    v.queue()
        .in_strip_order()
        .filter(|(_, parked)| *parked)
        .count()
}

#[gpui::test]
async fn a_billed_reply_stops_the_turn_and_says_so_off_screen(cx: &mut TestAppContext) {
    let f = busy_pane_off_screen(cx, true);
    fold_billed(&f, cx);

    f.ws.update(cx, |ws, cx| {
        let view = view(ws, f.pane);
        view.update(cx, |v, cx| {
            assert!(v.turn_is_idle(), "the turn is stopped");
            assert!(v.overage_notice().is_some());
            assert!(!v.queue().has_pending(), "nothing left to auto-send");
            assert_eq!(parked_count(v), 1, "the queued prompt is parked");
            assert!(
                !v.owes_completion_for_test(),
                "the Stopped completion fired in the same fold"
            );
            assert_eq!(
                v.tick_activity(std::time::Instant::now(), cx),
                None,
                "and only once"
            );
        });
        assert_eq!(overage_toasts(ws), 1);
        assert!(ws.lane_for(f.home).unwrap().is_unread);
    });

    // The turn is already stopped: a later billed report signals nothing.
    fold_billed(&f, cx);
    f.ws.read_with(cx, |ws, _| assert_eq!(overage_toasts(ws), 1));
}

#[gpui::test]
async fn with_the_setting_off_a_billed_reply_changes_nothing(cx: &mut TestAppContext) {
    let f = busy_pane_off_screen(cx, false);
    fold_billed(&f, cx);

    f.ws.read_with(cx, |ws, cx| {
        let v = view(ws, f.pane);
        let v = v.read(cx);
        assert!(!v.turn_is_idle(), "the turn keeps running");
        assert!(v.overage_notice().is_none());
        assert!(v.queue().has_pending());
        assert_eq!(parked_count(v), 0);
        assert!(v.is_busy());
        assert_eq!(overage_toasts(ws), 0);
        assert!(!ws.lane_for(f.home).unwrap().is_unread);
    });
}

/// Telegram paired, with an outbound queue a test can drain.
fn paired_with_phone(cx: &mut TestAppContext) -> (daruda_config::Config, Outbox) {
    let outbound = cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let mut config = daruda_config::Config::default();
    config.telegram.enabled = true;
    config.telegram.authorized_chat_id = Some(42);
    (config, outbound)
}

type Outbox = crate::telegram::global::TestOutbox;

fn drained_plain_texts(outbound: &mut Outbox) -> Vec<String> {
    use futures::{FutureExt as _, StreamExt as _};
    let mut texts = Vec::new();
    while let Some(Some(out)) = outbound.next().now_or_never() {
        if let crate::telegram::bridge::Outbound::Ping(ping) = out
            && let crate::telegram::bridge::TelegramTail::Plain(text) = ping.tail
        {
            texts.push(text);
        }
    }
    texts
}

fn expected_overage_text() -> String {
    let notice = daruda_chat::view::OverageNotice {
        window: Some(daruda_acp::RateLimitWindow::FiveHour),
        resets_at: Some(1_791_340_200),
    };
    let at = chrono::DateTime::from_timestamp(1_791_340_200, 0).expect("valid");
    crate::surface::strings::agent_chat::overage_stopped_resets(
        notice.window_label(),
        crate::surface::strings::flow::overage_reset_at(at),
    )
}

#[gpui::test]
async fn a_phone_started_turn_hears_why_it_stopped(cx: &mut TestAppContext) {
    let (config, mut outbound) = paired_with_phone(cx);
    let f = busy_pane_off_screen_with(cx, true, config);
    f.ws.update(cx, |ws, cx| {
        view(ws, f.pane).update(cx, |v, _| {
            v.start_phone_turn_for_test(std::time::Instant::now());
        });
    });
    drained_plain_texts(&mut outbound);
    fold_billed(&f, cx);

    assert!(
        drained_plain_texts(&mut outbound).contains(&expected_overage_text()),
        "the phone that started the turn is told"
    );
}

#[gpui::test]
async fn a_desk_turn_relays_nothing_to_the_phone(cx: &mut TestAppContext) {
    let (config, mut outbound) = paired_with_phone(cx);
    let f = busy_pane_off_screen_with(cx, true, config);
    drained_plain_texts(&mut outbound);
    fold_billed(&f, cx);

    f.ws.read_with(cx, |ws, _| assert_eq!(overage_toasts(ws), 1));
    assert!(
        !drained_plain_texts(&mut outbound).contains(&expected_overage_text()),
        "no phone turn, no relay"
    );
}
