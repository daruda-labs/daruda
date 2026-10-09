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
        let command = fs::read_to_string(entry.path().join("cmdline"))
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
