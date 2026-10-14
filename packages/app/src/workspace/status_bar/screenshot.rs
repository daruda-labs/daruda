//! Screenshot-only samples projected through the real status-bar widgets.
use super::StatusBarData;
use crate::keep_awake::KeepAwakeStatus;
use crate::lane::port_attribution::AttributionConfidence;
use crate::ui::{Popover, PopupMenu};
use crate::workspace::flow_request::FlowSource;
use crate::workspace::flow_rows::FlowRunRow;
use crate::workspace::sync::ports::{PortEntry, PortKind, PortScanStatus};
use daruda_config::KeepAwakeMode;
use daruda_config::StatusBarItem;
use daruda_store::project::LaneRef;
use gpui::{AnyElement, App, Context, Global, IntoElement as _, Window};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusBarShot {
    Ports,
    Flows,
    /// The keep-awake menu open over an agent-mode hold.
    KeepAwake,
    /// The keep-awake menu open over a lease the OS refused.
    KeepAwakeFailed,
}

impl Global for StatusBarShot {}

pub(super) fn open_for_shot(popover: Popover, id: &str, cx: &App) -> Popover {
    let open = matches!(
        (cx.try_global::<StatusBarShot>(), id),
        (Some(StatusBarShot::Ports), "status-ports-popover")
            | (Some(StatusBarShot::Flows), "status-flow-popover")
    );
    if open { popover.open(true) } else { popover }
}

pub(super) fn keep_awake_menu_open(cx: &App) -> bool {
    matches!(
        cx.try_global::<StatusBarShot>(),
        Some(StatusBarShot::KeepAwake | StatusBarShot::KeepAwakeFailed)
    )
}

/// The segment's menu, open. A dropdown menu cannot be opened from outside,
/// so the capture hosts the same builder in an open popover — what a click
/// on the trigger would have built.
pub(super) fn open_menu(
    trigger: crate::ui::Button,
    builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> AnyElement {
    let builder = std::rc::Rc::new(builder);
    Popover::new("status-keep-awake-shot")
        .appearance(false)
        .anchor(super::keep_awake_segment::MENU_ANCHOR)
        .open(true)
        .trigger(trigger)
        .content(move |_, window, cx| {
            let builder = builder.clone();
            window
                .use_keyed_state("status-keep-awake-shot-menu", cx, move |window, cx| {
                    PopupMenu::build(window, cx, move |menu, window, cx| {
                        builder(menu, window, cx)
                    })
                })
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
                    asking: None,
                    also_waiting: 0,
                })
                .collect();
        }
    }
    data
}
