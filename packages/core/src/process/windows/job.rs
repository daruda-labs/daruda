use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

/// Exit code a terminated job reports. `1` rather than `0` so a caller
/// reading the child's status does not mistake a kill for a clean run.
const KILLED: u32 = 1;

/// A refused job has no handle; it must never become an invalid OwnedHandle.
pub(in crate::process) fn adopt(pid: u32) -> Option<OwnedHandle> {
    try_adopt(pid).ok()
}

pub(in crate::process) fn try_adopt(pid: u32) -> std::io::Result<OwnedHandle> {
    // SAFETY: a null name and null attributes ask for an unnamed,
    // default-secured job; the returned handle is owned here.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    let Some(job) = owned(job) else {
        return Err(std::io::Error::last_os_error());
    };

    // Without this a descendant outlives the process that spawned it: the
    // job ends when its last handle closes, and by default that only
    // releases the members. See the invariant on `Group`.
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
        BasicLimitInformation: unsafe { std::mem::zeroed() },
        ..unsafe { std::mem::zeroed() }
    };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: the pointer and length describe `limits`, which outlives the
    // call, and the class matches the struct the API expects for it.
    let configured = unsafe {
        SetInformationJobObject(
            job.as_raw_handle() as HANDLE,
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&limits).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if configured == 0 {
        return Err(std::io::Error::last_os_error());
    }

    // SAFETY: `pid` names the child just spawned; the handle is closed
    // below whether or not the assignment takes.
    let process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid) };
    let Some(process) = owned(process) else {
        return Err(std::io::Error::last_os_error());
    };
    // SAFETY: both handles are live and owned for the duration.
    let assigned = unsafe {
        AssignProcessToJobObject(
            job.as_raw_handle() as HANDLE,
            process.as_raw_handle() as HANDLE,
        )
    };
    if assigned == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(job)
}

pub(in crate::process) fn terminate(job: &OwnedHandle) {
    let _ = request_termination(job);
}

fn request_termination(job: &OwnedHandle) -> std::io::Result<()> {
    if job.as_raw_handle().is_null() {
        return Err(std::io::Error::other("Process group was not assigned"));
    }
    // SAFETY: the handle is owned and live; terminating a job touches no
    // memory this process owns.
    if unsafe { TerminateJobObject(job.as_raw_handle() as HANDLE, KILLED) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(in crate::process) fn try_terminate(job: &OwnedHandle) -> std::io::Result<()> {
    request_termination(job)?;
    use windows_sys::Win32::System::JobObjects::{
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
        QueryInformationJobObject,
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: the owned job and sized accounting buffer stay live.
        if unsafe {
            QueryInformationJobObject(
                job.as_raw_handle() as HANDLE,
                JobObjectBasicAccountingInformation,
                std::ptr::from_mut(&mut accounting).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        if accounting.ActiveProcesses == 0 {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Process job did not finish terminating",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Both null and the Toolhelp invalid sentinel represent failed handles.
fn owned(handle: HANDLE) -> Option<OwnedHandle> {
    (!handle.is_null() && handle != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE)
        // SAFETY: the API just produced this handle and hands ownership
        // to the caller; it is wrapped once and never duplicated.
        .then(|| unsafe { OwnedHandle::from_raw_handle(handle.cast()) })
}

/// Let a child started by [`super::lead_own_group`] run.
///
/// A process created suspended has exactly one thread, so resuming every
/// thread it owns is resuming that one. Failure is not reported: the
/// caller is about to wait on a child that will never answer, and the
/// wait is where that surfaces.
pub(in crate::process) fn resume(pid: u32) {
    let _ = try_resume(pid);
}

pub(in crate::process) fn try_resume(pid: u32) -> std::io::Result<()> {
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    // SAFETY: a thread snapshot takes no pointer arguments; the handle is
    // owned here and closed by `OwnedHandle`.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    let Some(snapshot) = owned(snapshot) else {
        return Err(std::io::Error::last_os_error());
    };
    let mut entry = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    // SAFETY: `entry` is sized as the API requires and outlives the walk.
    let mut walking = unsafe { Thread32First(snapshot.as_raw_handle() as HANDLE, &mut entry) };
    let mut found = false;
    while walking != 0 {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: the id came from the snapshot; the handle is closed
            // on the next line whether or not the resume takes.
            unsafe {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                let thread = owned(thread).ok_or_else(std::io::Error::last_os_error)?;
                if ResumeThread(thread.as_raw_handle() as HANDLE) == u32::MAX {
                    return Err(std::io::Error::last_os_error());
                }
                found = true;
            }
        }
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        // SAFETY: same contract as `Thread32First`.
        walking = unsafe { Thread32Next(snapshot.as_raw_handle() as HANDLE, &mut entry) };
    }
    if found {
        Ok(())
    } else {
        Err(std::io::Error::other(
            "Suspended process thread is unavailable",
        ))
    }
}
