//! Background scan of system-wide listening TCP ports, feeding the
//! status bar's Ports segment. Mirrors `sync/limits.rs`'s per-Workspace
//! poll-loop shape, but the "fetch" here is local process introspection
//! (`lsof` on macOS, `/proc` on Linux) rather than a network call, so it
//! always runs on `background_executor` and never touches the network.
//!
//! Skipped entirely (loop idles at [`IDLE_RECHECK`]) whenever the
//! Ports segment is hidden (`StatusBarConfig::visible_items`), so a
//! user who never opens the segment pays no subprocess-spawn cost.

use std::time::Duration;

use daruda_config::StatusBarItem;
use gpui::{Context, Task, WeakEntity};

use crate::lane::port_attribution::{AttributionConfidence, LaneCandidate, ScannedPort, attribute};
use crate::workspace::Workspace;

/// Re-check cadence while the Ports segment is hidden. Reuses
/// `PortsConfig::MIN_POLL_SECS` scale: toggling the segment back on
/// takes effect quickly without spinning on `read_with` while idle.
const IDLE_RECHECK: Duration = Duration::from_secs(daruda_config::PortsConfig::MIN_POLL_SECS);

pub use crate::platform::ports::ListeningPort;

/// Classification for a scanned listening port. Mirrors Orca's
/// workspace/container/external split: workspace-owned ports lead the
/// status bar, while container and external ports are only secondary
/// context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortKind {
    Workspace {
        lane_label: String,
        confidence: AttributionConfidence,
    },
    Container,
    External,
}

/// One row of the Ports segment's popover: a scanned port's display
/// address, owning-process label, and explicit classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortEntry {
    pub port: u16,
    pub address: String,
    /// The owning process's short name, or `"PID <n>"` when a name
    /// couldn't be resolved — always non-empty, mirroring Orca's
    /// `processName ?? "PID ${pid}"` row label.
    pub process: String,
    pub kind: PortKind,
}

/// Status of the latest listening-port scan. Kept separate from
/// `PortEntry` rows so the status bar can distinguish the initial
/// pending state and an unavailable scanner from a successful scan that
/// found zero listeners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortScanStatus {
    Pending,
    Available,
    Unavailable,
}

/// The status bar's Ports data, written only by `set_port_scan_result`.
pub(in crate::workspace) struct PortsState {
    /// `Pending` until the first scan lands, then whether the scanner
    /// produced rows or was unavailable on this platform/runtime.
    pub(in crate::workspace) status: PortScanStatus,
    /// Latest listening ports, attributed to a lane where possible. Empty
    /// unless `status` is `Available`.
    pub(in crate::workspace) entries: Vec<PortEntry>,
}

impl Default for PortsState {
    fn default() -> Self {
        Self {
            status: PortScanStatus::Pending,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PortScanResult {
    status: PortScanStatus,
    ports: Vec<ListeningPort>,
}

impl PortScanResult {
    fn available(ports: Vec<ListeningPort>) -> Self {
        Self {
            status: PortScanStatus::Available,
            ports,
        }
    }

    fn unavailable() -> Self {
        Self {
            status: PortScanStatus::Unavailable,
            ports: Vec::new(),
        }
    }
}

/// Spawn the Ports scan pump. Returns the `Task<()>` handle so the
/// caller (Workspace constructor) can keep it alive in a field —
/// dropping the task cancels the loop.
pub(in crate::workspace) fn spawn(cx: &mut Context<Workspace>) -> Task<()> {
    cx.spawn(async move |this: WeakEntity<Workspace>, cx| {
        loop {
            let state = match this.read_with(cx, |ws, _| {
                (
                    ws.mirrors.status_bar.is_visible(StatusBarItem::Ports),
                    ws.mirrors.ports_poll_interval,
                )
            }) {
                Ok(state) => state,
                Err(_) => break,
            };
            let (visible, interval) = state;

            if !visible {
                cx.background_executor().timer(IDLE_RECHECK).await;
                continue;
            }

            let scan = cx.background_executor().spawn(async { scan() }).await;

            if this
                .update(cx, |ws, cx| ws.set_port_scan_result(scan, cx))
                .is_err()
            {
                break;
            }

            cx.background_executor().timer(interval).await;
        }
    })
}

impl Workspace {
    /// Attribute a fresh port scan against every open project's lanes
    /// and store the result for the status bar's Ports segment.
    ///
    /// Scans every open project's lanes, not just the active one — a
    /// background dev server can be running in a project the user
    /// isn't currently focused on, and the segment should still
    /// attribute it correctly when the user switches over.
    #[cfg(test)]
    pub(in crate::workspace) fn set_scanned_ports(
        &mut self,
        ports: Vec<ListeningPort>,
        cx: &mut Context<Self>,
    ) {
        self.set_port_scan_result(PortScanResult::available(ports), cx);
    }

    fn set_port_scan_result(&mut self, scan: PortScanResult, cx: &mut Context<Self>) {
        if scan.status == PortScanStatus::Unavailable {
            if self.ports.status != scan.status || !self.ports.entries.is_empty() {
                self.ports.status = scan.status;
                self.ports.entries.clear();
                cx.notify();
            }
            return;
        }

        let ports = scan.ports;
        let lanes: Vec<LaneCandidate> = self
            .projects
            .lanes()
            .map(|(_, project, lane)| LaneCandidate {
                path: lane.path.clone(),
                label: crate::workspace::lane_ops::lane_label(&project.name, lane),
            })
            .collect();
        let scanned: Vec<ScannedPort> = ports
            .iter()
            .map(|p| ScannedPort {
                port: p.port,
                cwd: p.cwd.clone(),
                command: p.command.clone(),
            })
            .collect();
        let attributed = attribute(&scanned, &lanes);
        let mut entries: Vec<PortEntry> = ports
            .into_iter()
            .zip(attributed)
            .map(|(port, attributed)| PortEntry {
                port: port.port,
                address: port.address.clone(),
                process: port
                    .process_name
                    .clone()
                    .unwrap_or_else(|| format!("PID {}", port.pid)),
                kind: match attributed.owner {
                    Some(owner) => PortKind::Workspace {
                        lane_label: owner.lane_label,
                        confidence: owner.confidence,
                    },
                    None if is_container_process(
                        port.process_name.as_deref(),
                        port.command.as_deref(),
                    ) =>
                    {
                        PortKind::Container
                    }
                    None => PortKind::External,
                },
            })
            .collect();
        entries.sort_by(compare_port_entries);
        if self.ports.status != scan.status || self.ports.entries != entries {
            self.ports.status = scan.status;
            self.ports.entries = entries;
            cx.notify();
        }
    }
}

fn compare_port_entries(a: &PortEntry, b: &PortEntry) -> std::cmp::Ordering {
    port_kind_rank(&a.kind)
        .cmp(&port_kind_rank(&b.kind))
        .then_with(|| a.port.cmp(&b.port))
        .then_with(|| sort_host_for_address(&a.address).cmp(&sort_host_for_address(&b.address)))
        .then_with(|| a.address.cmp(&b.address))
        .then_with(|| a.process.cmp(&b.process))
        .then_with(|| port_kind_label(&a.kind).cmp(port_kind_label(&b.kind)))
}

fn port_kind_rank(kind: &PortKind) -> u8 {
    match kind {
        PortKind::Workspace { .. } => 0,
        PortKind::Container => 1,
        PortKind::External => 2,
    }
}

fn port_kind_label(kind: &PortKind) -> &str {
    match kind {
        PortKind::Workspace { lane_label, .. } => lane_label,
        PortKind::Container | PortKind::External => "",
    }
}

fn is_container_process(process_name: Option<&str>, command: Option<&str>) -> bool {
    let haystack = format!(
        "{} {}",
        process_name.unwrap_or_default(),
        command.unwrap_or_default()
    )
    .to_ascii_lowercase();
    haystack
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-'))
        .any(|word| {
            word.starts_with("container")
                || word.starts_with("com.container")
                || (word.starts_with("com.") && word.ends_with(".backend"))
        })
}

fn dedupe_listening_ports(ports: Vec<ListeningPort>) -> Vec<ListeningPort> {
    let mut seen = std::collections::HashSet::new();
    let mut deduped = Vec::new();
    for port in ports {
        let key = (connect_host_for_address(&port.address), port.port, port.pid);
        if seen.insert(key) {
            deduped.push(port);
        }
    }
    deduped
}

fn connect_host_for_address(address: &str) -> String {
    let host = sort_host_for_address(address);
    if matches!(host.as_str(), "*" | "0.0.0.0" | "::") {
        "localhost".to_string()
    } else {
        host
    }
}

fn sort_host_for_address(address: &str) -> String {
    address
        .rsplit_once(':')
        .map_or(address, |(host, _)| host)
        .trim_matches(['[', ']'])
        .to_string()
}

/// Scan failures produce `Unavailable`; an empty successful scan is available.
fn scan() -> PortScanResult {
    crate::platform::ports::scan()
        .map(dedupe_listening_ports)
        .map(PortScanResult::available)
        .unwrap_or_else(PortScanResult::unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(address: &str, pid: u32) -> ListeningPort {
        ListeningPort {
            port: address
                .rsplit_once(':')
                .and_then(|(_, port)| port.parse().ok())
                .unwrap_or(0),
            address: address.to_string(),
            pid,
            process_name: Some("node".to_string()),
            cwd: None,
            command: None,
        }
    }

    #[test]
    fn dedupes_same_listener_reported_on_equivalent_connect_hosts() {
        let ports = dedupe_listening_ports(vec![
            port("*:3000", 123),
            port("0.0.0.0:3000", 123),
            port("127.0.0.1:3000", 123),
        ]);
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].address, "*:3000");
        assert_eq!(ports[1].address, "127.0.0.1:3000");
    }

    #[test]
    fn port_entries_sort_by_kind_port_host_and_process() {
        let mut entries = [
            entry(3000, "127.0.0.1:3000", "z", PortKind::External),
            entry(3000, "*:3000", "docker", PortKind::Container),
            entry(
                5000,
                "*:5000",
                "node",
                PortKind::Workspace {
                    lane_label: "app/main".to_string(),
                    confidence: AttributionConfidence::Cwd,
                },
            ),
            entry(
                3000,
                "127.0.0.1:3000",
                "node",
                PortKind::Workspace {
                    lane_label: "app/main".to_string(),
                    confidence: AttributionConfidence::Cwd,
                },
            ),
            entry(3000, "*:3000", "a", PortKind::External),
        ];

        entries.sort_by(compare_port_entries);

        assert_eq!(
            entries
                .iter()
                .map(|entry| (
                    port_kind_rank(&entry.kind),
                    entry.port,
                    entry.address.as_str(),
                    entry.process.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![
                (0, 3000, "127.0.0.1:3000", "node"),
                (0, 5000, "*:5000", "node"),
                (1, 3000, "*:3000", "docker"),
                (2, 3000, "*:3000", "a"),
                (2, 3000, "127.0.0.1:3000", "z"),
            ]
        );
    }

    fn entry(port: u16, address: &str, process: &str, kind: PortKind) -> PortEntry {
        PortEntry {
            port,
            address: address.to_string(),
            process: process.to_string(),
            kind,
        }
    }
}
