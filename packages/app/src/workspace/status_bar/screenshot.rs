//! Screenshot-only samples projected through the real status-bar widgets.
use super::StatusBarData;
use crate::lane::port_attribution::AttributionConfidence;
use crate::ui::Popover;
use crate::workspace::flow_request::FlowSource;
use crate::workspace::flow_rows::FlowRunRow;
use crate::workspace::sync::ports::{PortEntry, PortKind, PortScanStatus};
use daruda_config::StatusBarItem;
use daruda_store::project::LaneRef;
use gpui::{App, Global};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusBarShot {
    Ports,
    Flows,
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
