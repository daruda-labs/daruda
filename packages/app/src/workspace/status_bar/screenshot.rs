//! Screenshot-only samples projected through the real status-bar widgets.
use super::StatusBarData;
use crate::keep_awake::KeepAwakeStatus;
use crate::lane::port_attribution::AttributionConfidence;
use crate::ui::{Popover, PopupMenu};
use crate::workspace::flow_request::FlowSource;
use crate::workspace::flow_rows::FlowRunRow;
use crate::workspace::main_area::pane::AccountDomain;
use crate::workspace::sync::ports::{PortEntry, PortKind, PortScanStatus};
use daruda_agent::{ProviderUsage, UsageOutcome, UsageWindow, WindowScope};
use daruda_config::KeepAwakeMode;
use daruda_config::StatusBarItem;
use daruda_store::accounts::{
    AccountId, AccountRecipeId, AccountSelection, AccountsState, ManagedAccount,
};
use daruda_store::project::LaneRef;
use gpui::{AnyElement, App, Context, Global, IntoElement as _, SharedString, Window};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusBarShot {
    Ports,
    Flows,
    /// The keep-awake menu open over an agent-mode hold.
    KeepAwake,
    /// The keep-awake menu open over a lease the OS refused.
    KeepAwakeFailed,
    /// One provider's usage menu open over seeded windows.
    Usage,
    /// The account menu open over seeded accounts in two domains.
    Account,
}

impl Global for StatusBarShot {}

/// The popup each shot opens, by the id its chip passes to `menu`.
fn shot_popup(shot: StatusBarShot) -> &'static str {
    match shot {
        StatusBarShot::Ports => "status-ports-popover",
        StatusBarShot::Flows => "status-flow-popover",
        StatusBarShot::KeepAwake | StatusBarShot::KeepAwakeFailed => "status-keep-awake",
        StatusBarShot::Usage => "status-usage",
        StatusBarShot::Account => "status-account",
    }
}

pub(super) fn is_open(id: &str, cx: &App) -> bool {
    cx.try_global::<StatusBarShot>()
        .is_some_and(|shot| shot_popup(*shot) == id)
}

pub(super) fn open_for_shot(popover: Popover, id: &str, cx: &App) -> Popover {
    if is_open(id, cx) {
        popover.open(true)
    } else {
        popover
    }
}

/// The chip's menu, open. A dropdown menu cannot be opened from outside, so
/// the capture hosts the same builder in an open popover — what a click on
/// the trigger would have built.
pub(super) fn open_menu(
    id: &'static str,
    trigger: crate::ui::Button,
    builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> AnyElement {
    let builder = std::rc::Rc::new(builder);
    Popover::new(SharedString::from(format!("{id}-shot")))
        .appearance(false)
        .anchor(super::menu::ANCHOR)
        .open(true)
        .trigger(trigger)
        .content(move |_, window, cx| {
            let builder = builder.clone();
            window
                .use_keyed_state(
                    SharedString::from(format!("{id}-shot-menu")),
                    cx,
                    move |window, cx| {
                        PopupMenu::build(window, cx, move |menu, window, cx| {
                            builder(menu, window, cx)
                        })
                    },
                )
                .read(cx)
                .clone()
        })
        .into_any_element()
}

pub(super) fn seeded(mut data: StatusBarData, cx: &App) -> StatusBarData {
    let Some(shot) = cx.try_global::<StatusBarShot>() else {
        return data;
    };
    match shot {
        StatusBarShot::Ports => {
            if !data.visible.is_visible(StatusBarItem::Ports) {
                data.visible.toggle(StatusBarItem::Ports);
            }
            data.ports_status = PortScanStatus::Available;
            data.ports = (0..40)
                .map(|i| PortEntry {
                    port: 4000 + i,
                    address: format!("127.0.0.1:{}", 4000 + i),
                    process: if i % 2 == 0 {
                        "node".into()
                    } else {
                        "postgres".into()
                    },
                    kind: if i < 24 {
                        PortKind::Workspace {
                            lane_label: "daruda / ui-controls".into(),
                            confidence: AttributionConfidence::Cwd,
                        }
                    } else {
                        PortKind::External
                    },
                })
                .collect();
        }
        StatusBarShot::KeepAwake | StatusBarShot::KeepAwakeFailed => {
            if !data.visible.is_visible(StatusBarItem::KeepAwake) {
                data.visible.toggle(StatusBarItem::KeepAwake);
            }
            data.keep_awake.mode = KeepAwakeMode::Agent;
            data.keep_awake.status = if *shot == StatusBarShot::KeepAwake {
                KeepAwakeStatus::Active {
                    working: 2,
                    waiting: 1,
                }
            } else {
                KeepAwakeStatus::Failed
            };
        }
        StatusBarShot::Usage => {
            if !data.visible.is_visible(StatusBarItem::ClaudeUsage) {
                data.visible.toggle(StatusBarItem::ClaudeUsage);
            }
            data.usage = vec![(AccountRecipeId::Claude, seeded_usage())];
        }
        StatusBarShot::Account => {
            if !data.visible.is_visible(StatusBarItem::AccountSlot) {
                data.visible.toggle(StatusBarItem::AccountSlot);
            }
            data.account = Some(seeded_account(&data));
        }
        StatusBarShot::Flows => {
            data.flows = (0..24)
                .map(|i| FlowRunRow {
                    lane: LaneRef {
                        project: 1,
                        lane: i,
                    },
                    run_dir: std::env::temp_dir().join("daruda-shot-flow"),
                    source: FlowSource::Resumed {
                        run_dir: std::env::temp_dir().join("daruda-shot-flow"),
                    },
                    lane_label: format!("daruda / ui-controls-{i:02}").into(),
                    doing: "Checking shared controls".into(),
                    asking: Vec::new(),
                })
                .collect();
        }
    }
    data
}

fn seeded_usage() -> UsageOutcome {
    let now = SystemTime::now();
    let window = |hours: u64, utilization: f32, resets_in: u64, scope| UsageWindow {
        window: Duration::from_secs(hours * 3600),
        utilization,
        resets_at: Some(now + Duration::from_secs(resets_in)),
        scope,
    };
    UsageOutcome::Ok(ProviderUsage::new(
        AccountRecipeId::Claude,
        vec![
            window(5, 42.0, 2 * 3600 + 13 * 60, WindowScope::Overall),
            window(7 * 24, 81.0, 3 * 24 * 3600, WindowScope::Overall),
            window(7 * 24, 12.0, 3 * 24 * 3600, WindowScope::Opus),
        ],
        None,
    ))
}

/// Two Claude accounts and one Codex account on a terminal pane, the second
/// Claude account selected — every row shape the menu has.
fn seeded_account(data: &StatusBarData) -> super::AccountSlot {
    let account = |email: &str, organization: Option<&str>, recipe| ManagedAccount {
        id: AccountId::new(),
        recipe,
        email: Some(email.to_string()),
        organization: organization.map(str::to_string),
        config_dir: std::env::temp_dir().join("daruda-shot-account"),
        created_at: 0,
        last_authenticated_at: 0,
    };
    let accounts = AccountsState {
        accounts: vec![
            account("alice@example.com", None, AccountRecipeId::Claude),
            account(
                "bob@example.com",
                Some("Example Team"),
                AccountRecipeId::Claude,
            ),
            account("carol@example.com", None, AccountRecipeId::Codex),
        ],
        ..AccountsState::default()
    };
    let selected = AccountSelection::Managed(accounts.accounts[1].id);
    let pane_id = data.account.as_ref().map_or(0, |slot| slot.pane_id);
    super::AccountSlot::resolve(
        pane_id,
        selected,
        AccountDomain::Any,
        &accounts,
        data.workspace.clone(),
        false,
    )
}
