//! Read listening IPv4/IPv6 sockets and owner PIDs through IP Helper.

use std::{
    io,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
    path::PathBuf,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER},
    NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID,
        TCP_TABLE_OWNER_PID_LISTENER,
    },
    Networking::WinSock::{AF_INET, AF_INET6},
    System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    },
};

pub(crate) struct Listener {
    pub address: SocketAddr,
    pub pid: u32,
    pub process_name: Option<String>,
}

pub(crate) fn scan() -> io::Result<Vec<Listener>> {
    let mut listeners = Vec::new();
    for row in rows::<MIB_TCPROW_OWNER_PID>(u32::from(AF_INET))? {
        listeners.push(Listener {
            address: SocketAddr::from((
                Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes()),
                port(row.dwLocalPort),
            )),
            pid: row.dwOwningPid,
            process_name: process_name(row.dwOwningPid),
        });
    }
    for row in rows::<MIB_TCP6ROW_OWNER_PID>(u32::from(AF_INET6))? {
        listeners.push(Listener {
            address: SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(row.ucLocalAddr),
                port(row.dwLocalPort),
                0,
                row.dwLocalScopeId,
            )),
            pid: row.dwOwningPid,
            process_name: process_name(row.dwOwningPid),
        });
    }
    Ok(listeners)
}

fn port(value: u32) -> u16 {
    u16::from_be(value as u16)
}

fn rows<T: Copy>(family: u32) -> io::Result<Vec<T>> {
    let mut bytes = 0;
    // SAFETY: the null first call asks for buffer size; no table is read.
    let result = unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut bytes,
            0,
            family,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if result != 0 && result != ERROR_INSUFFICIENT_BUFFER {
        return Err(io::Error::from_raw_os_error(result as i32));
    }
    for _ in 0..4 {
        if !(4..=16 * 1024 * 1024).contains(&bytes) {
            return Err(io::Error::other("Invalid TCP table size"));
        }
        let mut table = vec![0u32; (bytes as usize).div_ceil(4)];
        // SAFETY: u32 storage has the table's alignment and allocated size.
        let result = unsafe {
            GetExtendedTcpTable(
                table.as_mut_ptr().cast(),
                &mut bytes,
                0,
                family,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if result == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if result != 0 {
            return Err(io::Error::from_raw_os_error(result as i32));
        }
        let count = table[0] as usize;
        let used = count
            .checked_mul(size_of::<T>())
            .and_then(|size| size.checked_add(4))
            .ok_or_else(|| io::Error::other("Invalid TCP row count"))?;
        if used > bytes as usize || used > table.len() * 4 {
            return Err(io::Error::other("Truncated TCP table"));
        }
        // SAFETY: count and byte length were validated above; copy rows rather
        // than borrowing from the differently typed backing allocation.
        return Ok((0..count)
            .map(|index| unsafe {
                table
                    .as_ptr()
                    .cast::<u8>()
                    .add(4 + index * size_of::<T>())
                    .cast::<T>()
                    .read_unaligned()
            })
            .collect());
    }
    Err(io::Error::other("TCP table changed repeatedly during scan"))
}

fn process_name(pid: u32) -> Option<String> {
    // SAFETY: query a read-only process handle and close it on every path.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        let success = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length);
        CloseHandle(process);
        if success == 0 {
            return None;
        }
        let path = PathBuf::from(String::from_utf16_lossy(&path[..length as usize]));
        path.file_stem()
            .map(|name| name.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_scan_finds_own_ipv4_and_ipv6_listeners_with_owner_pid() {
        let v4 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let v6 = std::net::TcpListener::bind("[::1]:0").ok();
        let listeners = scan().unwrap();
        for address in std::iter::once(v4.local_addr().unwrap())
            .chain(v6.as_ref().map(|v| v.local_addr().unwrap()))
        {
            assert!(listeners.iter().any(|row| row.pid == std::process::id()
                && row.address == address
                && row.process_name.is_some()));
        }
    }
}
