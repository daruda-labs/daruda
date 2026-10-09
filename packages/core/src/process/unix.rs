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
            if error.raw_os_error() == Some(libc::ESRCH) {
                Ok(())
            } else {
                Err(error)
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

pub(super) fn command(program: &std::ffi::OsStr) -> std::process::Command {
    std::process::Command::new(program)
}

pub(super) fn resolve_program(
    program: &std::ffi::OsStr,
    _path: Option<&std::ffi::OsStr>,
) -> std::path::PathBuf {
    program.into()
}
