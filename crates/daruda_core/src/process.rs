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

/// Ask for a child that can be torn down as a tree. Call before `spawn`,
/// then [`Group::adopt`] on the pid it returns.
///
/// Unix does the work here: the child leads its own process group, so its pid
/// doubles as a group id and a signal aimed at this app's group stops at the
/// boundary. Windows has nothing to say before a process exists.
pub fn lead_own_group(command: &mut std::process::Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    let _ = command;
}

/// A spawned child's tear-down handle: everything it goes on to fork belongs
/// to this, and [`Group::kill_tree`] ends all of it.
///
/// Held rather than derived, because Windows cannot derive it. A job object
/// has to exist before the descendants do, so the membership lives in a
/// handle the caller keeps beside the child. Dropping it kills nothing.
#[derive(Debug)]
pub struct Group(GroupInner);

#[derive(Debug)]
#[cfg(unix)]
struct GroupInner(u32);

#[derive(Debug)]
#[cfg(windows)]
struct GroupInner(std::os::windows::io::OwnedHandle);

#[derive(Debug)]
#[cfg(not(any(unix, windows)))]
struct GroupInner;

impl Group {
    /// Take the child at `pid` into a group, right after `spawn` — a Windows
    /// descendant forked in that window escapes the job.
    ///
    /// INVARIANT (unix): `pid` led its own group ([`lead_own_group`]) and is
    /// not yet reaped — a zombie holds its pid, so the group id is still this
    /// process's to name. Reap first and the OS may have reissued it.
    pub fn adopt(pid: u32) -> Self {
        #[cfg(unix)]
        {
            Self(GroupInner(pid))
        }
        #[cfg(windows)]
        {
            Self(GroupInner(windows_job::adopt(pid)))
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = pid;
            Self(GroupInner)
        }
    }

    /// End the child and everything under it.
    ///
    /// Failure is not reported: the caller is already tearing the child down,
    /// and the reap that follows is what the caller actually waits on.
    pub fn kill_tree(&self) {
        #[cfg(unix)]
        {
            // SAFETY: `killpg` takes a group id and a signal, touches no
            // memory this process owns, and cannot fail in a way that
            // invalidates state here. Which group it reaches is the contract
            // on `adopt`, not a memory question.
            unsafe {
                libc::killpg(self.0.0 as libc::pid_t, libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        windows_job::terminate(&self.0.0);
    }
}

/// The Windows half of [`Group`]. Nothing here is reachable from a caller —
/// the job handle only ever travels inside a `Group`.
#[cfg(windows)]
mod windows_job {
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    /// Exit code a terminated job reports. `1` rather than `0` so a caller
    /// reading the child's status does not mistake a kill for a clean run.
    const KILLED: u32 = 1;

    /// Create a job holding `pid`, or an empty handle when any step refuses —
    /// a `kill_tree` that reaches nothing is what this already degrades to on
    /// a platform without job objects.
    pub(super) fn adopt(pid: u32) -> OwnedHandle {
        // SAFETY: a null name and null attributes ask for an unnamed,
        // default-secured job; the returned handle is owned here.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        let Some(job) = owned(job) else {
            return empty();
        };

        // Without this a descendant outlives the handle: the job is destroyed
        // when its last handle closes, and by default that just releases them.
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
            BasicLimitInformation: unsafe { std::mem::zeroed() },
            ..unsafe { std::mem::zeroed() }
        };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: the pointer and length describe `limits`, which outlives the
        // call, and the class matches the struct the API expects for it.
        unsafe {
            SetInformationJobObject(
                job.as_raw_handle() as HANDLE,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
        }

        // SAFETY: `pid` names the child just spawned; the handle is closed
        // below whether or not the assignment takes.
        let process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid) };
        let Some(process) = owned(process) else {
            return job;
        };
        // SAFETY: both handles are live and owned for the duration.
        unsafe {
            AssignProcessToJobObject(
                job.as_raw_handle() as HANDLE,
                process.as_raw_handle() as HANDLE,
            );
        }
        job
    }

    pub(super) fn terminate(job: &OwnedHandle) {
        if job.as_raw_handle().is_null() {
            return;
        }
        // SAFETY: the handle is owned and live; terminating a job touches no
        // memory this process owns.
        unsafe {
            TerminateJobObject(job.as_raw_handle() as HANDLE, KILLED);
        }
    }

    /// `None` for the null handle every one of these APIs returns on failure.
    fn owned(handle: HANDLE) -> Option<OwnedHandle> {
        (!handle.is_null())
            // SAFETY: the API just produced this handle and hands ownership
            // to the caller; it is wrapped once and never duplicated.
            .then(|| unsafe { OwnedHandle::from_raw_handle(handle.cast()) })
    }

    /// A handle that owns nothing, so `terminate` is a no-op on it.
    fn empty() -> OwnedHandle {
        // SAFETY: a null handle is never closed by `OwnedHandle::drop`'s
        // `CloseHandle` in a way that can fail destructively, and `terminate`
        // refuses it before use.
        unsafe { OwnedHandle::from_raw_handle(std::ptr::null_mut()) }
    }
}

/// Whether `pid` still names a live process.
///
/// Signal 0 delivers nothing; it only asks whether the pid is claimed. Same
/// caveat as [`Group::kill_tree`] in reverse — a reaped pid may have been handed to
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
    use std::time::Duration;

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

    /// The reason this module exists: a tear-down has to reach what the child
    /// forked, not only the child. The fixture leaves a grandchild behind and
    /// exits, which is exactly what `child.kill()` cannot clean up.
    ///
    /// Runs everywhere, because the Windows half — a job object — has no
    /// other way to be checked from this machine.
    #[test]
    fn kill_tree_reaches_a_grandchild_the_child_left_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pid_file = dir.path().join("grandchild.pid");

        let mut cmd = command(test_process::executable());
        cmd.arg("--orphan").arg(&pid_file);
        lead_own_group(&mut cmd);
        let mut child = cmd.spawn().expect("spawn");
        let group = Group::adopt(child.id());

        let grandchild = read_pid(&pid_file);
        assert!(is_alive(grandchild), "the fixture must leave one behind");

        group.kill_tree();

        assert!(
            gone_within(grandchild, Duration::from_secs(5)),
            "the grandchild outlived the kill — it reached only the child"
        );
        let _ = child.wait();
    }

    /// The fixture writes the pid before exiting, but the write races this
    /// read on a loaded machine.
    fn read_pid(path: &std::path::Path) -> u32 {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(text) = std::fs::read_to_string(path)
                && let Ok(pid) = text.trim().parse()
            {
                return pid;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the fixture never recorded a grandchild"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The kill is asynchronous and the pid is released on reap, so "gone" is
    /// a bounded wait rather than an instant.
    fn gone_within(pid: u32, budget: Duration) -> bool {
        let deadline = std::time::Instant::now() + budget;
        while std::time::Instant::now() < deadline {
            if !is_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
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

        Group::adopt(child.id()).kill_tree();

        let status = child.wait().expect("wait");
        assert!(!status.success(), "the child was signalled");
    }
}
