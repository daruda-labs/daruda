//! Background scan of system-wide listening TCP ports, feeding the
//! status bar's Ports segment. Mirrors `sync/limits.rs`'s per-Workspace
//! poll-loop shape, but the "fetch" here is local process introspection
//! (`lsof` on macOS, `/proc` on Linux) rather than a network call, so it
//! always runs on `background_executor` and never touches the network.
//!
//! Skipped entirely (loop idles at [`IDLE_RECHECK`]) whenever the
//! Ports segment is hidden (`StatusBarConfig::visible_items`), so a
//! user who never opens the segment pays no subprocess-spawn cost.

use std::path::PathBuf;
use std::time::Duration;

use daruda_config::StatusBarItem;
use gpui::{Context, Task, WeakEntity};

use crate::lane::port_attribution::{AttributionConfidence, LaneCandidate, ScannedPort, attribute};
use crate::workspace::Workspace;

/// Re-check cadence while the Ports segment is hidden. Reuses
/// `PortsConfig::MIN_POLL_SECS` scale: toggling the segment back on
/// takes effect quickly without spinning on `read_with` while idle.
const IDLE_RECHECK: Duration = Duration::from_secs(daruda_config::PortsConfig::MIN_POLL_SECS);

/// One system-wide listening TCP port, with enough owning-process
/// detail for [`crate::lane::port_attribution::attribute`] to attribute
/// it to a lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListeningPort {
    pub port: u16,
    /// `host:port` as reported by the scan (e.g. `*:3000`,
    /// `127.0.0.1:5432`) — display-only, not used for attribution.
    pub address: String,
    pub pid: u32,
    /// Owning process's short name (e.g. `node`, `python3`), when
    /// resolvable — display-only, distinct from `command`'s full
    /// argv-joined line.
    pub process_name: Option<String>,
    /// Owning process's current working directory, when resolvable.
    pub cwd: Option<PathBuf>,
    /// Owning process's full command line, when resolvable.
    pub command: Option<String>,
}

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
            if self.port_scan_status != scan.status || !self.attributed_ports.is_empty() {
                self.port_scan_status = scan.status;
                self.attributed_ports.clear();
                cx.notify();
            }
            return;
        }

        let ports = scan.ports;
        let lanes: Vec<LaneCandidate> = self
            .projects
            .iter()
            .flat_map(|project| {
                project.lanes.iter().map(|lane| LaneCandidate {
                    path: lane.path.clone(),
                    label: format!("{}/{}", project.name, lane.display_name()),
                })
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
        if self.port_scan_status != scan.status || self.attributed_ports != entries {
            self.port_scan_status = scan.status;
            self.attributed_ports = entries;
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

/// Scan the current OS for listening TCP ports. Command/procfs failures
/// produce `Unavailable`; a successful scan with no listeners produces
/// `Available` with an empty row list.
#[cfg(target_os = "macos")]
fn scan() -> PortScanResult {
    macos::scan()
        .map(dedupe_listening_ports)
        .map(PortScanResult::available)
        .unwrap_or_else(PortScanResult::unavailable)
}

#[cfg(target_os = "linux")]
fn scan() -> PortScanResult {
    linux::scan()
        .map(dedupe_listening_ports)
        .map(PortScanResult::available)
        .unwrap_or_else(PortScanResult::unavailable)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn scan() -> PortScanResult {
    PortScanResult::unavailable()
}

#[cfg(target_os = "macos")]
mod macos {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::process::Command;

    use super::ListeningPort;

    pub(super) fn scan() -> Option<Vec<ListeningPort>> {
        let Ok(listen_output) = Command::new("lsof")
            .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-F", "pcn"])
            .output()
        else {
            return None;
        };
        let entries = parse_listen_entries(&String::from_utf8_lossy(&listen_output.stdout));
        if entries.is_empty() {
            return Some(Vec::new());
        }

        let mut pids: Vec<u32> = entries.iter().map(|e| e.pid).collect();
        pids.sort_unstable();
        pids.dedup();
        let pid_list = pids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");

        let cwd_by_pid = Command::new("lsof")
            .args(["-a", "-p", &pid_list, "-d", "cwd", "-Fn"])
            .output()
            .ok()
            .map(|out| parse_cwd_entries(&String::from_utf8_lossy(&out.stdout)))
            .unwrap_or_default();

        let command_by_pid = Command::new("ps")
            .args(["-p", &pid_list, "-o", "pid=", "-o", "command="])
            .output()
            .ok()
            .map(|out| parse_ps_commands(&String::from_utf8_lossy(&out.stdout)))
            .unwrap_or_default();

        Some(
            entries
                .into_iter()
                .map(|entry| ListeningPort {
                    port: entry.port,
                    address: entry.address,
                    pid: entry.pid,
                    process_name: entry.process_name,
                    cwd: cwd_by_pid.get(&entry.pid).cloned(),
                    command: command_by_pid.get(&entry.pid).cloned(),
                })
                .collect(),
        )
    }

    struct ListenEntry {
        pid: u32,
        port: u16,
        address: String,
        process_name: Option<String>,
    }

    /// Parse `lsof -F pcn` output: each process block starts with a
    /// `p<pid>` field line, followed by a `c<command>` line (the
    /// process's short name) and one or more `n<address>:<port>`
    /// lines (one per listening socket owned by that process).
    fn parse_listen_entries(output: &str) -> Vec<ListenEntry> {
        let mut entries = Vec::new();
        let mut current_pid: Option<u32> = None;
        let mut current_process_name: Option<String> = None;
        for line in output.lines() {
            let mut chars = line.chars();
            let tag = chars.next();
            let rest = chars.as_str();
            match tag {
                Some('p') => {
                    current_pid = rest.parse().ok();
                    current_process_name = None;
                }
                Some('c') => current_process_name = Some(rest.to_string()),
                Some('n') => {
                    let Some(pid) = current_pid else { continue };
                    let Some(port) = rest.rsplit(':').next().and_then(|p| p.parse().ok()) else {
                        continue;
                    };
                    entries.push(ListenEntry {
                        pid,
                        port,
                        address: rest.to_string(),
                        process_name: current_process_name.clone(),
                    });
                }
                _ => {}
            }
        }
        entries
    }

    /// Parse `lsof -a -p <pids> -d cwd -Fn` output: `p<pid>` lines
    /// followed by the process's cwd as `n<path>`.
    fn parse_cwd_entries(output: &str) -> HashMap<u32, PathBuf> {
        let mut map = HashMap::new();
        let mut current_pid: Option<u32> = None;
        for line in output.lines() {
            let mut chars = line.chars();
            let tag = chars.next();
            let rest = chars.as_str();
            match tag {
                Some('p') => current_pid = rest.parse().ok(),
                Some('n') => {
                    if let Some(pid) = current_pid {
                        map.insert(pid, PathBuf::from(rest));
                    }
                }
                _ => {}
            }
        }
        map
    }

    /// Parse `ps -p <pids> -o pid= -o command=` output: one line per
    /// pid, no header (`=` suffix suppresses it), pid then the full
    /// command line.
    fn parse_ps_commands(output: &str) -> HashMap<u32, String> {
        output
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim_start();
                let mut parts = trimmed.splitn(2, char::is_whitespace);
                let pid = parts.next()?.parse::<u32>().ok()?;
                let command = parts.next()?.trim_start().to_string();
                Some((pid, command))
            })
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_multiple_listen_entries_across_processes() {
            let output = "p1234\ncnode\nn*:3000\np5678\ncpython3\nn127.0.0.1:8000\n";
            let entries = parse_listen_entries(output);
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].pid, 1234);
            assert_eq!(entries[0].port, 3000);
            assert_eq!(entries[0].process_name.as_deref(), Some("node"));
            assert_eq!(entries[1].pid, 5678);
            assert_eq!(entries[1].port, 8000);
            assert_eq!(entries[1].process_name.as_deref(), Some("python3"));
        }

        #[test]
        fn parses_multiple_ports_from_same_process() {
            let output = "p1234\ncnode\nn*:3000\nn*:3001\n";
            let entries = parse_listen_entries(output);
            assert_eq!(entries.len(), 2);
            assert!(entries.iter().all(|e| e.pid == 1234));
            assert!(
                entries
                    .iter()
                    .all(|e| e.process_name.as_deref() == Some("node"))
            );
        }

        #[test]
        fn parses_cwd_entries() {
            let output = "p1234\nn/repo/app\n";
            let map = parse_cwd_entries(output);
            assert_eq!(map.get(&1234), Some(&PathBuf::from("/repo/app")));
        }

        #[test]
        fn parses_ps_commands_with_spaces_in_command() {
            let output = "  1234 node server.js --port 3000\n  5678 python3 -m http.server 8000\n";
            let map = parse_ps_commands(output);
            assert_eq!(
                map.get(&1234).map(String::as_str),
                Some("node server.js --port 3000")
            );
            assert_eq!(
                map.get(&5678).map(String::as_str),
                Some("python3 -m http.server 8000")
            );
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashMap;
    use std::fs;

    use super::ListeningPort;

    pub(super) fn scan() -> Option<Vec<ListeningPort>> {
        let mut by_inode: HashMap<u64, (u16, String)> = HashMap::new();
        let mut read_any = false;
        for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
            if let Ok(content) = fs::read_to_string(path) {
                read_any = true;
                by_inode.extend(parse_proc_net_tcp(&content));
            }
        }
        if !read_any {
            return None;
        }
        if by_inode.is_empty() {
            return Some(Vec::new());
        }

        let Ok(proc_entries) = fs::read_dir("/proc") else {
            return None;
        };
        let mut results = Vec::new();
        for entry in proc_entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(fds) = fs::read_dir(entry.path().join("fd")) else {
                continue;
            };
            let mut matched: Vec<(u16, String)> = fds
                .flatten()
                .filter_map(|fd| fs::read_link(fd.path()).ok())
                .filter_map(|link| socket_inode(&link))
                .filter_map(|inode| by_inode.get(&inode).cloned())
                .collect();
            if matched.is_empty() {
                continue;
            }
            matched.sort_unstable();
            matched.dedup();

            let cwd = fs::read_link(entry.path().join("cwd")).ok();
            let command =
                fs::read_to_string(entry.path().join("cmdline"))
                    .ok()
                    .map(|raw: String| {
                        raw.split('\0')
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                            .join(" ")
                    });
            let process_name = fs::read_to_string(entry.path().join("comm"))
                .ok()
                .map(|raw| raw.trim().to_string())
                .filter(|s| !s.is_empty());

            for (port, address) in matched {
                results.push(ListeningPort {
                    port,
                    address: address.clone(),
                    pid,
                    process_name: process_name.clone(),
                    cwd: cwd.clone(),
                    command: command.clone(),
                });
            }
        }
        Some(results)
    }

    /// Parse one `/proc/net/tcp[6]` file. Each data line's whitespace
    /// fields are, 0-indexed: `sl local_address rem_address st ...
    /// inode` — field 1 is `hex_addr:hex_port`, field 3 is connection
    /// state (`0A` = `TCP_LISTEN`), field 9 is the socket inode. The
    /// header line and any malformed line are skipped.
    fn parse_proc_net_tcp(content: &str) -> HashMap<u64, (u16, String)> {
        content
            .lines()
            .skip(1)
            .filter_map(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                if fields.get(3)? != &"0A" {
                    return None;
                }
                let hex_addr_port = *fields.get(1)?;
                let (port, address) = decode_local_address(hex_addr_port)?;
                let inode = fields.get(9)?.parse::<u64>().ok()?;
                Some((inode, (port, address)))
            })
            .collect()
    }

    /// Decode a `/proc/net/tcp[6]` `local_address` field
    /// (`hex_ip:hex_port`) into `(port, "host:port")`. IPv4 addresses
    /// are fully decoded (4 hex bytes, stored little-endian so the byte
    /// order is reversed); IPv6 falls back to a `*` host — daruda's
    /// Linux GUI runtime isn't yet verified (see project CLAUDE.md), so
    /// the extra V6 word-order decoding isn't worth it until that's
    /// real. The port is always decoded correctly either way.
    fn decode_local_address(hex_addr_port: &str) -> Option<(u16, String)> {
        let (hex_ip, hex_port) = hex_addr_port.split_once(':')?;
        let port = u16::from_str_radix(hex_port, 16).ok()?;
        if hex_ip.len() == 8
            && let Ok(bytes) = (0..4)
                .map(|i| u8::from_str_radix(&hex_ip[i * 2..i * 2 + 2], 16))
                .collect::<Result<Vec<_>, _>>()
        {
            return Some((
                port,
                format!(
                    "{}.{}.{}.{}:{}",
                    bytes[3], bytes[2], bytes[1], bytes[0], port
                ),
            ));
        }
        Some((port, format!("*:{port}")))
    }

    /// Extract the inode from an fd symlink target of the form
    /// `socket:[12345]`; `None` for any other fd kind (regular file,
    /// pipe, tty, …).
    fn socket_inode(link: &std::path::Path) -> Option<u64> {
        let s = link.to_str()?;
        s.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_listening_entry_from_proc_net_tcp() {
            let sample = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 12345 1 0000000000000000 100 0 0 10 0\n";
            let result = parse_proc_net_tcp(sample);
            assert_eq!(
                result.get(&12345),
                Some(&(8080, "127.0.0.1:8080".to_string()))
            );
        }

        #[test]
        fn skips_non_listen_states() {
            let sample = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 00000000:1F90 0100007F:0050 01 00000000:00000000 00:00000000 00000000     0        0 12345 1 0000000000000000 100 0 0 10 0\n";
            assert!(parse_proc_net_tcp(sample).is_empty());
        }

        #[test]
        fn decodes_ipv4_address_in_reversed_byte_order() {
            assert_eq!(
                decode_local_address("0100007F:1F90"),
                Some((8080, "127.0.0.1:8080".to_string()))
            );
        }

        #[test]
        fn falls_back_to_wildcard_host_for_ipv6() {
            let (port, address) =
                decode_local_address("00000000000000000000000000000000:1F90").unwrap();
            assert_eq!(port, 8080);
            assert_eq!(address, "*:8080");
        }

        #[test]
        fn extracts_socket_inode_from_fd_symlink() {
            assert_eq!(
                socket_inode(std::path::Path::new("socket:[98765]")),
                Some(98765)
            );
            assert_eq!(socket_inode(std::path::Path::new("/dev/null")), None);
        }
    }
}
