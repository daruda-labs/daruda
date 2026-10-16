//! POSIX process groups and liveness checks.

pub(super) fn lead_own_group(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt as _;
    command.process_group(0);
}

pub(super) fn is_alive(pid: u32) -> bool {
    // SAFETY: any value is a valid pid argument, and signal 0 has no
    // effect beyond the existence check.
    unsafe {
        libc::kill(pid as libc::pid_t, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

/// Microseconds since the epoch at which `pid` started.
#[cfg(target_vendor = "apple")]
pub(super) fn start_time(pid: u32) -> Option<u64> {
    let pid = libc::pid_t::try_from(pid).ok()?;
    // SAFETY: `proc_bsdinfo` is plain C data, for which all zeroes is valid.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = libc::c_int::try_from(std::mem::size_of::<libc::proc_bsdinfo>()).ok()?;
    // SAFETY: the buffer is `info`, owned by this frame, and `size` is its
    // length; the call writes at most that many bytes.
    let written =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    (written == size).then(|| {
        info.pbi_start_tvsec
            .saturating_mul(1_000_000)
            .saturating_add(info.pbi_start_tvusec)
    })
}

/// Clock ticks after boot at which `pid` started.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub(super) fn start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    proc_stat_start_time(&stat)
}

#[cfg(not(any(target_vendor = "apple", target_os = "linux", target_os = "android")))]
pub(super) fn start_time(_pid: u32) -> Option<u64> {
    None
}

/// Field 22 of `/proc/<pid>/stat`. Counted after the last `)`, because the
/// command name before it is parenthesised and may hold spaces or `)`.
#[cfg(any(target_os = "linux", target_os = "android", test))]
fn proc_stat_start_time(stat: &str) -> Option<u64> {
    let (_, after_name) = stat.rsplit_once(')')?;
    // Field 3 (state) is the first after the name, so field 22 is the 20th.
    after_name.split_whitespace().nth(19)?.parse().ok()
}

pub(super) fn kill_group(leader: u32) -> std::io::Result<()> {
    GroupInner::adopt(leader).try_kill_tree()
}

pub(super) fn has_descendants(_pid: u32) -> std::io::Result<bool> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Use native foreground groups",
    ))
}

#[derive(Debug)]
pub(super) struct GroupInner(u32);
impl GroupInner {
    pub(super) fn try_adopt(pid: u32) -> std::io::Result<Self> {
        Ok(Self::adopt(pid))
    }
    pub(super) fn adopt(pid: u32) -> Self {
        Self(pid)
    }
    pub(super) fn try_kill_tree(&self) -> std::io::Result<()> {
        // SAFETY: the caller retains the unreaped group identity.
        if unsafe { libc::killpg(self.0 as libc::pid_t, libc::SIGKILL) } == 0 {
            Ok(())
        } else {
            let error = std::io::Error::last_os_error();
            match error.raw_os_error() {
                Some(errno) if group_is_gone(errno, cfg!(target_vendor = "apple")) => Ok(()),
                _ => Err(error),
            }
        }
    }
    pub(super) fn kill_tree(&self) {
        // SAFETY: the PID names the unreaped group adopted by the caller.
        unsafe {
            libc::killpg(self.0 as libc::pid_t, libc::SIGKILL);
        }
    }
}

/// Whether a failed `killpg` means nothing in the group is left to stop.
///
/// Darwin answers `EPERM` for a group of zombies only — what an unreaped
/// leader that exited on its own leaves — where Linux signals them and says
/// `0`. Only a foreign-uid member could also cause it, which a tree we
/// spawned does not hold.
fn group_is_gone(errno: i32, darwin: bool) -> bool {
    errno == libc::ESRCH || (darwin && errno == libc::EPERM)
}

pub(super) fn command(program: &std::ffi::OsStr) -> std::process::Command {
    std::process::Command::new(program)
}

pub(super) fn resolve_program(
    program: &std::ffi::OsStr,
    _path: Option<&std::ffi::OsStr>,
) -> std::path::PathBuf {
    program.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vanished_group_is_gone_on_every_unix() {
        assert!(group_is_gone(libc::ESRCH, false));
        assert!(group_is_gone(libc::ESRCH, true));
    }

    /// A command name may hold spaces and parentheses; only the last `)`
    /// ends it.
    #[test]
    fn the_start_time_is_read_past_a_command_name_with_parentheses() {
        let stat = "4242 (a) b (c) S 1 4242 4242 0 -1 4194304 90 0 0 0 0 0 0 0 20 0 1 0 \
                    987654 1000 200";
        assert_eq!(proc_stat_start_time(stat), Some(987_654));
        assert_eq!(proc_stat_start_time("4242 (cut) S 1"), None);
        assert_eq!(proc_stat_start_time("no name at all"), None);
    }

    #[test]
    fn eperm_means_gone_only_where_darwin_reports_zombies_with_it() {
        assert!(group_is_gone(libc::EPERM, true));
        assert!(!group_is_gone(libc::EPERM, false));
        assert!(!group_is_gone(libc::EINVAL, true));
    }
}
