//! Listening sockets and owner metadata, independent of workspace policy.

use std::path::PathBuf;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// One system-wide listening TCP port, with enough owning-process
/// detail for workspace attribution to attribute
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

/// None denotes an unavailable scanner; an empty vector is a successful scan.
pub(crate) fn scan() -> Option<Vec<ListeningPort>> {
    #[cfg(target_os = "macos")]
    return macos::scan();
    #[cfg(target_os = "linux")]
    return linux::scan();
    #[cfg(windows)]
    return match windows::scan() {
        Ok(ports) => Some(ports),
        Err(error) => {
            super::report_error("ports.scan", "Windows port scan failed", &error);
            None
        }
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    None
}
