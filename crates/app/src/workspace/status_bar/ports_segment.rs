//! Status bar's Ports segment — one chip whose count is only the
//! listening TCP ports owned by an open lane in *this* workspace
//! (`sync::ports` scans system-wide, `lane::port_attribution` narrows
//! it down). Ports the scan found but no open lane claims are secondary
//! context in the same dropdown. Workspace ports render first; external
//! ports render below them as table rows. The summary and column header
//! stay fixed while the port list body scrolls inside the window.

use super::StatusBarDensity;
use crate::ui::theme;
use crate::ui::{DropdownMenu as _, PopupMenuItem, button_status_pill, menu_builder};
use crate::workspace::sync::ports::{PortEntry, PortKind, PortScanStatus};
use gpui::{
    AnyElement, App, ClipboardItem, IntoElement, ScrollHandle, SharedString, div, prelude::*, px,
};
use gpui_component::scroll::{Scrollbar, ScrollbarShow};
use std::collections::BTreeMap;

const PORT_MENU_SCROLL_MAX_H: f32 = 360.0;
const PORT_MENU_SCROLLBAR_GUTTER: f32 = 10.0;
const PORT_TABLE_WIDTH: f32 = 340.0;
const PORT_TABLE_PORT_W: f32 = 46.0;
const PORT_TABLE_PROCESS_W: f32 = 118.0;
const PORT_TABLE_GAP: f32 = 10.0;
const PORT_SECTION_GAP: f32 = 4.0;
const PORT_ROW_RADIUS: f32 = 3.0;

/// Render the Ports trigger button. The trigger count is workspace
/// ports only; non-workspace ports remain available from the dropdown
/// but never become their own status-bar chip.
pub(super) fn render(
    ports: &[PortEntry],
    status: PortScanStatus,
    density: StatusBarDensity,
    cx: &App,
) -> impl IntoElement {
    let groups = workspace_groups(ports);
    let external = external_ports(ports);
    let workspace_count: usize = groups.iter().map(|(_, ports)| ports.len()).sum();
    let external_count = external.len();
    let label = SharedString::from(trigger_label(workspace_count, density));
    let summary = SharedString::from(summary_label(status, workspace_count, external_count));
    let scroll_handle = ScrollHandle::default();
    button_status_pill("status-ports", label, cx)
        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
        .tooltip(summary.clone())
        .dropdown_menu(menu_builder(move |menu, _window, _cx| {
            let menu = menu.label(summary.clone());

            match status {
                PortScanStatus::Pending => {
                    return menu.separator().label(SharedString::from(
                        crate::surface::strings::status_bar_ports_scanning(),
                    ));
                }
                PortScanStatus::Unavailable => {
                    return menu.separator().label(SharedString::from(
                        crate::surface::strings::status_bar_ports_scan_unavailable(),
                    ));
                }
                PortScanStatus::Available => {}
            }

            if workspace_count == 0 && external_count == 0 {
                return menu.separator().label(SharedString::from(
                    crate::surface::strings::status_bar_ports_no_workspace(),
                ));
            }

            let groups = groups.clone();
            let external = external.clone();
            let scroll_handle = scroll_handle.clone();
            menu.separator()
                .item(table_header_item())
                .item(PopupMenuItem::element(move |_window, cx| {
                    port_scroll_body(&groups, &external, &scroll_handle, cx)
                }))
        }))
}

fn summary_label(status: PortScanStatus, workspace_count: usize, external_count: usize) -> String {
    match status {
        PortScanStatus::Pending => crate::surface::strings::status_bar_ports_scanning(),
        PortScanStatus::Unavailable => crate::surface::strings::status_bar_ports_scan_unavailable(),
        PortScanStatus::Available => {
            crate::surface::strings::status_bar_ports_summary(workspace_count, external_count)
        }
    }
}

/// `"Ports: N"` at `Full`; bare `"N"` at `Compact`/`IconOnly` — the count is
/// the only information that survives every tier. No dropdown chevron: this
/// is a reading, not a control, and the pill's hover state already says it
/// is clickable.
fn trigger_label(count: usize, density: StatusBarDensity) -> String {
    if density.is_reduced() {
        count.to_string()
    } else {
        crate::surface::strings::status_bar_ports_label(count)
    }
}

/// Ports owned by an open lane, grouped by lane label (`(lane_label,
/// ports)`), sorted by lane label and then port/host/process.
fn workspace_groups(ports: &[PortEntry]) -> Vec<(String, Vec<PortEntry>)> {
    let mut groups: BTreeMap<String, Vec<PortEntry>> = BTreeMap::new();
    for entry in ports {
        let PortKind::Workspace { lane_label, .. } = &entry.kind else {
            continue;
        };
        groups
            .entry(lane_label.clone())
            .or_default()
            .push(entry.clone());
    }
    let mut groups: Vec<(String, Vec<PortEntry>)> = groups.into_iter().collect();
    for (_, ports) in &mut groups {
        ports.sort_by(compare_display_ports);
    }
    groups
}

/// Ports no open lane's cwd/command line claimed. Container-attributed
/// rows live here too, matching Orca's `kind !== "workspace"` grouping.
fn external_ports(ports: &[PortEntry]) -> Vec<PortEntry> {
    let mut external: Vec<PortEntry> = ports
        .iter()
        .filter(|p| !matches!(p.kind, PortKind::Workspace { .. }))
        .cloned()
        .collect();
    external.sort_by(compare_display_ports);
    external
}

fn compare_display_ports(a: &PortEntry, b: &PortEntry) -> std::cmp::Ordering {
    a.port
        .cmp(&b.port)
        .then_with(|| sort_host_for_address(&a.address).cmp(&sort_host_for_address(&b.address)))
        .then_with(|| a.address.cmp(&b.address))
        .then_with(|| a.process.cmp(&b.process))
}

fn sort_host_for_address(address: &str) -> String {
    address
        .rsplit_once(':')
        .map_or(address, |(host, _)| host)
        .trim_matches(['[', ']'])
        .to_string()
}

fn port_scroll_body(
    groups: &[(String, Vec<PortEntry>)],
    external: &[PortEntry],
    scroll_handle: &ScrollHandle,
    cx: &App,
) -> AnyElement {
    let mut content = div()
        .flex()
        .flex_col()
        .gap(px(PORT_SECTION_GAP))
        .w(px(PORT_TABLE_WIDTH))
        .text_size(px(theme::STATUS_BAR_FONT_SIZE));
    let mut row_ix = 0usize;

    if groups.is_empty() {
        content = content.child(status_row(
            crate::surface::strings::status_bar_ports_no_workspace(),
            cx,
        ));
    } else {
        let workspace_count: usize = groups.iter().map(|(_, ports)| ports.len()).sum();
        content = content.child(section_label(
            crate::surface::strings::status_bar_ports_workspace_count(workspace_count),
            cx,
        ));
        for (label, ports) in groups {
            content = content.child(section_label(label.clone(), cx));
            for entry in ports {
                content = content.child(port_click_row(row_ix, entry, cx));
                row_ix += 1;
            }
        }
    }

    if !external.is_empty() {
        content = content.child(section_label(
            crate::surface::strings::status_bar_ports_external_count(external.len()),
            cx,
        ));
        for entry in external {
            content = content.child(port_click_row(row_ix, entry, cx));
            row_ix += 1;
        }
    }

    let body_scroll = scroll_handle.clone();
    let bar_scroll = scroll_handle.clone();
    div()
        .relative()
        .w(px(PORT_TABLE_WIDTH + PORT_MENU_SCROLLBAR_GUTTER))
        .max_h(px(PORT_MENU_SCROLL_MAX_H))
        .child(
            div()
                .id("ports-scroll-body")
                .max_h(px(PORT_MENU_SCROLL_MAX_H))
                .pr(px(PORT_MENU_SCROLLBAR_GUTTER))
                .overflow_y_scroll()
                .track_scroll(&body_scroll)
                .child(content),
        )
        .child(
            Scrollbar::vertical(&bar_scroll)
                .id("ports-menu-scrollbar")
                .scrollbar_show(ScrollbarShow::Always),
        )
        .into_any_element()
}

fn section_label(label: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    let t = theme::current(cx);
    div()
        .pt(px(PORT_SECTION_GAP))
        .text_color(t.text_subtle)
        .child(label.into())
}

fn status_row(label: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    let t = theme::current(cx);
    div()
        .w(px(PORT_TABLE_WIDTH))
        .text_color(t.text_subtle)
        .child(label.into())
}

fn port_click_row(row_ix: usize, entry: &PortEntry, cx: &App) -> AnyElement {
    let t = theme::current(cx);
    let address = entry.address.clone();
    div()
        .id(("ports-menu-row", row_ix))
        .rounded(px(PORT_ROW_RADIUS))
        .cursor_pointer()
        .hover(|this| this.bg(t.status_bar_account_hover_bg))
        .on_click(move |_, _window, app| {
            app.write_to_clipboard(ClipboardItem::new_string(address.clone()));
        })
        .child(table_row(entry))
        .into_any_element()
}

fn table_header_item() -> PopupMenuItem {
    PopupMenuItem::element(|_window, _cx| table_header()).disabled(true)
}

fn table_header() -> AnyElement {
    port_table_row(
        crate::surface::strings::status_bar_ports_table_port(),
        crate::surface::strings::status_bar_ports_table_process(),
        crate::surface::strings::status_bar_ports_table_address(),
    )
}

fn table_row(entry: &PortEntry) -> AnyElement {
    port_table_row(
        entry.port.to_string(),
        entry.process.clone(),
        entry.address.clone(),
    )
}

fn port_table_row(
    port: impl Into<SharedString>,
    process: impl Into<SharedString>,
    address: impl Into<SharedString>,
) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(PORT_TABLE_GAP))
        .w(px(PORT_TABLE_WIDTH))
        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
        .child(port_table_cell(PORT_TABLE_PORT_W, port))
        .child(port_table_cell(PORT_TABLE_PROCESS_W, process))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(address.into()),
        )
        .into_any_element()
}

fn port_table_cell(width: f32, text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .w(px(width))
        .overflow_hidden()
        .whitespace_nowrap()
        .child(text.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port_row_label(entry: &PortEntry) -> String {
        format!("{}  {}  {}", entry.port, entry.process, entry.address)
    }

    fn workspace_entry(address: &str, lane_label: &str) -> PortEntry {
        PortEntry {
            port: port_number(address),
            address: address.to_string(),
            process: "node".to_string(),
            kind: PortKind::Workspace {
                lane_label: lane_label.to_string(),
                confidence: crate::lane::port_attribution::AttributionConfidence::Cwd,
            },
        }
    }

    fn external_entry(address: &str) -> PortEntry {
        PortEntry {
            port: port_number(address),
            address: address.to_string(),
            process: "node".to_string(),
            kind: PortKind::External,
        }
    }

    fn port_number(address: &str) -> u16 {
        address
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse().ok())
            .unwrap_or(0)
    }

    #[test]
    fn groups_attributed_ports_by_lane() {
        let ports = vec![
            workspace_entry("127.0.0.1:3000", "app/main"),
            workspace_entry("127.0.0.1:3001", "app/main"),
            workspace_entry("127.0.0.1:8080", "app/feature"),
        ];
        let groups = workspace_groups(&ports);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "app/feature");
        assert_eq!(groups[0].1.len(), 1);
        assert_eq!(groups[1].0, "app/main");
        assert_eq!(
            groups[1]
                .1
                .iter()
                .map(|entry| entry.port)
                .collect::<Vec<_>>(),
            vec![3000, 3001]
        );
        assert!(external_ports(&ports).is_empty());
    }

    #[test]
    fn external_ports_are_sorted_by_port_host_and_process() {
        let mut a = external_entry("127.0.0.1:5432");
        a.process = "postgres".to_string();
        let mut b = external_entry("*:5432");
        b.process = "docker".to_string();
        let mut c = external_entry("*:6379");
        c.process = "redis".to_string();

        let ports = external_ports(&[c, a, b]);
        assert_eq!(
            ports
                .iter()
                .map(|entry| (entry.port, entry.address.as_str(), entry.process.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (5432, "*:5432", "docker"),
                (5432, "127.0.0.1:5432", "postgres"),
                (6379, "*:6379", "redis"),
            ]
        );
    }

    #[test]
    fn unattributed_ports_are_external_only() {
        let ports = vec![
            workspace_entry("127.0.0.1:3000", "app/main"),
            external_entry("*:5432"),
        ];
        let groups = workspace_groups(&ports);
        let external = external_ports(&ports);
        assert_eq!(groups.len(), 1);
        assert_eq!(external.len(), 1);
        assert_eq!(external[0].address, "*:5432");
    }

    #[test]
    fn all_external_yields_no_workspace_groups() {
        let ports = vec![external_entry("*:5432"), external_entry("*:6379")];
        assert!(workspace_groups(&ports).is_empty());
        assert_eq!(external_ports(&ports).len(), 2);
    }

    #[test]
    fn container_ports_share_the_external_bucket() {
        let ports = vec![PortEntry {
            port: 0,
            address: "*:5000".to_string(),
            process: "com.docker.backend".to_string(),
            kind: PortKind::Container,
        }];
        assert!(workspace_groups(&ports).is_empty());
        assert_eq!(external_ports(&ports).len(), 1);
    }

    #[test]
    fn trigger_label_shows_full_word_at_full_density() {
        assert!(
            trigger_label(3, StatusBarDensity::Full)
                .starts_with(&crate::surface::strings::status_bar_ports_label(3))
        );
    }

    #[test]
    fn trigger_label_is_bare_count_when_reduced() {
        assert_eq!(trigger_label(3, StatusBarDensity::Compact), "3");
        assert_eq!(trigger_label(3, StatusBarDensity::IconOnly), "3");
    }

    /// The chevron is reserved for pills you pick a value from; a port count
    /// is a reading, and two chevrons in a row read as noise once the usage
    /// chip sits beside it.
    #[test]
    fn trigger_label_carries_no_dropdown_chevron() {
        let chevron = crate::surface::strings::TASK_PILL_CHEVRON.trim();
        for density in [
            StatusBarDensity::Full,
            StatusBarDensity::Compact,
            StatusBarDensity::IconOnly,
        ] {
            assert!(!trigger_label(3, density).contains(chevron));
        }
    }

    #[test]
    fn summary_label_distinguishes_scan_states() {
        assert_eq!(
            summary_label(PortScanStatus::Pending, 0, 0),
            crate::surface::strings::status_bar_ports_scanning()
        );
        assert_eq!(
            summary_label(PortScanStatus::Unavailable, 0, 0),
            crate::surface::strings::status_bar_ports_scan_unavailable()
        );
        assert_eq!(
            summary_label(PortScanStatus::Available, 1, 2),
            crate::surface::strings::status_bar_ports_summary(1, 2)
        );
    }

    #[test]
    fn port_row_label_keeps_copyable_address_visible() {
        let entry = PortEntry {
            port: 5432,
            address: "127.0.0.1:5432".to_string(),
            process: "postgres".to_string(),
            kind: PortKind::External,
        };
        assert_eq!(port_row_label(&entry), "5432  postgres  127.0.0.1:5432");
    }
}
