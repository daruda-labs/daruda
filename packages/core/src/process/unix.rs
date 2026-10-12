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

    #[test]
    fn eperm_means_gone_only_where_darwin_reports_zombies_with_it() {
        assert!(group_is_gone(libc::EPERM, true));
        assert!(!group_is_gone(libc::EPERM, false));
        assert!(!group_is_gone(libc::EINVAL, true));
    }
}
