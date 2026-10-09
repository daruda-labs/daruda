use std::collections::HashMap;
use std::path::PathBuf;

use super::ListeningPort;

pub(super) fn scan() -> Option<Vec<ListeningPort>> {
    let Ok(listen_output) = daruda_core::process::command("lsof")
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

    let cwd_by_pid = daruda_core::process::command("lsof")
        .args(["-a", "-p", &pid_list, "-d", "cwd", "-Fn"])
        .output()
        .ok()
        .map(|out| parse_cwd_entries(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or_default();

    let command_by_pid = daruda_core::process::command("ps")
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
