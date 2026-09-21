//! Spawning a child, and reaching the tree it forks.
//!
//! Four crates each spelled this out, in two spellings of one POSIX call —
//! so a second platform would have meant four Job Object implementations.
//! Deciding *whether* to kill a tree stays with the caller; the call that
//! does it lives here.

use std::ffi::OsStr;

/// The OS-provided archive extractor. Windows' bsdtar understands Node's ZIP
/// archives; a Git installation's GNU tar may shadow it on PATH.
pub fn archive_command() -> std::process::Command {
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").expect("Windows system directory");
        command(std::path::PathBuf::from(root).join("System32/tar.exe"))
    }
    #[cfg(not(windows))]
    {
        command("tar")
    }
}

/// Build a command through the one gate every spawn goes through.
///
/// Windows console subprocesses stay hidden when launched by the GUI.
/// The caller still supplies an executable, not a shell command line.
pub fn command(program: impl AsRef<OsStr>) -> std::process::Command {
    let command = std::process::Command::new(program);
    #[cfg(windows)]
    let command = {
        use std::os::windows::process::CommandExt as _;
        let mut command = command;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        command
    };
    command
}

/// Make the child lead its own process group, so a later [`kill_tree`]
/// reaches everything it forks and nothing else. Call before `spawn`.
///
/// Its pid then doubles as the group id, and a signal aimed at this app's
/// group stops at the boundary. A no-op where the platform has no
/// equivalent.
pub fn lead_own_group(command: &mut std::process::Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    let _ = command;
}

/// Kill the whole tree led by `pid`.
///
/// INVARIANT: `pid` led its own group (see [`lead_own_group`]) and has not
/// been reaped. An unreaped child keeps its pid — a zombie still holds one —
/// so the group id is still this process's to name. Reap first and the OS
/// may have handed that id to someone else, which is whose tree this would
/// reach. The one caller that cannot tell is `read_auth_status`, which
/// therefore kills only its direct child.
///
/// Failure is not reported: the caller is already tearing the child down.
pub fn kill_tree(pid: u32) {
    #[cfg(unix)]
    {
        // SAFETY: `killpg` takes a group id and a signal, touches no memory
        // this process owns, and cannot fail in a way that invalidates state
        // here. Which group it reaches is the contract above, not a memory
        // question.
        unsafe {
            libc::killpg(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// Whether `pid` still names a live process.
///
/// Signal 0 delivers nothing; it only asks whether the pid is claimed. Same
/// caveat as [`kill_tree`] in reverse — a reaped pid may have been handed to
/// someone else, so this answers "is something there", not "is *that* still
/// there".
pub fn is_alive(pid: u32) -> bool {
    // Zero names a Unix process group or the Windows idle process, never
    // a child that can hold a daruda run lock.
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: any value is a valid pid argument, and signal 0 has no
        // effect beyond the existence check.
        unsafe {
            libc::kill(pid as libc::pid_t, 0) == 0
                || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
        };
        // SAFETY: the handle is used only for a zero-time wait and closed
        // exactly once. Failure to inspect a process must not reclaim its lock.
        unsafe {
            let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if process.is_null() {
                return std::io::Error::last_os_error().raw_os_error()
                    != Some(ERROR_INVALID_PARAMETER as i32);
            }
            let alive = WaitForSingleObject(process, 0) != WAIT_OBJECT_0;
            CloseHandle(process);
            alive
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_alive() {
        assert!(is_alive(std::process::id()));
        assert!(!is_alive(0));
    }

    #[test]
    fn another_live_process_is_not_mistaken_for_a_stale_lock() {
        let mut child = command(test_process::executable())
            .args(["--sleep-ms", "30000"])
            .spawn()
            .unwrap();
        let alive = is_alive(child.id());
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(alive);
    }

    #[test]
    fn command_keeps_the_program_it_was_given() {
        let built = command("some-program");

        assert_eq!(built.get_program(), "some-program");
    }

    /// The reason this module exists: a signal has to reach what the child
    /// forked, not only the child. `sh` exits immediately and leaves the
    /// grandchild holding the pipe, so a read on that pipe returns only once
    /// the grandchild is gone too.
    #[cfg(unix)]
    #[test]
    fn kill_tree_reaches_a_grandchild_the_child_left_behind() {
        use std::io::Read as _;
        use std::process::Stdio;
        use std::sync::mpsc;
        use std::time::Duration;

        let mut cmd = command("sh");
        cmd.arg("-c")
            .arg("sleep 30 & exec 1>&-; wait")
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        lead_own_group(&mut cmd);
        let mut child = cmd.spawn().expect("spawn");
        let pid = child.id();
        let mut pipe = child.stdout.take().expect("piped stdout");

        kill_tree(pid);

        // The pipe's write end outlives `sh` — the grandchild holds it — so
        // this read returns only once the grandchild is gone too. Bounded,
        // because the assertion is that it happens *now*: `sleep 30` would
        // end it on its own eventually, and a test that waited would pass
        // just as well with `kill_tree` doing nothing at all.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut sink = Vec::new();
            let _ = pipe.read_to_end(&mut sink);
            let _ = tx.send(());
        });

        assert!(
            rx.recv_timeout(Duration::from_secs(5)).is_ok(),
            "the grandchild still holds the pipe — the kill reached only `sh`"
        );

        let _ = child.wait();
    }

    /// The contract's other half: killing the group must not take this
    /// process with it. A child that never called `lead_own_group` shares
    /// our group, and killing *that* would be suicide — so the call is only
    /// ever made with a pid that led its own.
    #[cfg(unix)]
    #[test]
    fn a_group_leader_is_the_only_thing_killed() {
        let mut cmd = command("sh");
        cmd.arg("-c").arg("sleep 30");
        lead_own_group(&mut cmd);
        let mut child = cmd.spawn().expect("spawn");

        kill_tree(child.id());

        let status = child.wait().expect("wait");
        assert!(!status.success(), "the child was signalled");
    }
}
