//! Windows process creation, inspection and job ownership.

use std::ffi::OsStr;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
mod job;

pub(super) fn lead_own_group(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt as _;
    // `creation_flags` replaces rather than adds, so the flag `command`
    // already set is repeated here — both live in this module.
    command.creation_flags(
        windows_sys::Win32::System::Threading::CREATE_NO_WINDOW
            | windows_sys::Win32::System::Threading::CREATE_SUSPENDED,
    );
}

pub(super) fn is_alive(pid: u32) -> bool {
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

pub(super) fn has_descendants(pid: u32) -> std::io::Result<bool> {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    // SAFETY: snapshots own no borrowed pointers and the handle closes below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the snapshot is a valid owned handle, closed once by Drop.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot.cast()) };
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    let mut found = false;
    // SAFETY: the sized entry and snapshot remain live throughout the walk.
    let mut valid = unsafe { Process32FirstW(snapshot.as_raw_handle().cast(), &mut entry) };
    if valid == 0 {
        let error = std::io::Error::last_os_error();
        return Err(error);
    }
    while valid != 0 {
        if entry.th32ParentProcessID == pid {
            found = true;
            break;
        }
        valid = unsafe { Process32NextW(snapshot.as_raw_handle().cast(), &mut entry) };
    }
    Ok(found)
}

#[derive(Debug)]
pub(super) struct GroupInner(Option<OwnedHandle>);
impl GroupInner {
    pub(super) fn try_adopt(pid: u32) -> std::io::Result<Self> {
        let handle = job::try_adopt(pid)?;
        job::try_resume(pid)?;
        Ok(Self(Some(handle)))
    }
    pub(super) fn adopt(pid: u32) -> Self {
        let handle = job::adopt(pid);
        job::resume(pid);
        Self(handle)
    }
    pub(super) fn try_kill_tree(&self) -> std::io::Result<()> {
        let handle = self
            .0
            .as_ref()
            .ok_or_else(|| std::io::Error::other("Process group was not assigned"))?;
        job::try_terminate(handle)
    }
    pub(super) fn kill_tree(&self) {
        if let Some(handle) = &self.0 {
            job::terminate(handle);
        }
    }
}

pub(super) fn command(program: &OsStr) -> std::process::Command {
    use std::os::windows::process::CommandExt as _;
    let mut command = std::process::Command::new(program);
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    command
}

pub(super) fn resolve_program(program: &OsStr, path: Option<&OsStr>) -> std::path::PathBuf {
    if std::path::Path::new(program)
        .parent()
        .is_none_or(|parent| parent.as_os_str().is_empty())
    {
        let found = match path {
            Some(path) => which::which_in(
                program,
                Some(path),
                std::env::current_dir().unwrap_or_default(),
            ),
            None => which::which(program),
        };
        if let Ok(found) = found {
            return found;
        }
    }

    std::path::PathBuf::from(program)
}
