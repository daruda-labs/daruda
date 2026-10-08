//! Profile identity and owned Start menu shortcut lifecycle.

use anyhow::Context as _;
use sha2::{Digest as _, Sha256};
use windows_api::Win32::Foundation::PROPERTYKEY;
use windows_api::Win32::System::Com::STGM_READ;
use windows_api::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows_api::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile};
use windows_api::Win32::System::Variant::VT_LPWSTR;
use windows_api::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows_api::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows_api::Win32::UI::Shell::SLGP_RAWPATH;
use windows_api::Win32::UI::Shell::{
    IShellLinkW, SetCurrentProcessExplicitAppUserModelID, ShellLink,
};
use windows_api::core::{GUID, HSTRING, Interface as _, PCWSTR, PWSTR};

fn initialize_apartment() -> anyhow::Result<()> {
    // SAFETY: initialize this GUI thread's WinRT apartment. GPUI may already
    // own an STA; RPC_E_CHANGED_MODE means it is initialized and usable.
    if let Err(error) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
        && error.code().0 as u32 != 0x80010106
    {
        return Err(error.into());
    }
    Ok(())
}

fn programs_directory() -> anyhow::Result<std::path::PathBuf> {
    Ok(dirs::data_dir()
        .context("Windows roaming data directory unavailable")?
        .join("Microsoft/Windows/Start Menu/Programs"))
}

pub(super) fn unregister() -> anyhow::Result<()> {
    initialize_apartment()?;
    unregister_in(&programs_directory()?, &std::env::current_exe()?)
}

fn unregister_in(programs: &std::path::Path, executable: &std::path::Path) -> anyhow::Result<()> {
    if !programs.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(programs)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_prefix("daruda-"))
            .and_then(|name| name.strip_suffix(".lnk"))
        else {
            continue;
        };
        if id.len() != 16
            || !id.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !entry.file_type()?.is_file()
        {
            continue;
        }
        let target = shortcut_target(&entry.path());
        match target {
            Ok(target) if daruda_core::path::same_path(&target, executable) => {
                std::fs::remove_file(entry.path())?
            }
            Ok(_) => {}
            Err(error) => crate::platform::report_error(
                "desktop.notification.shortcuts",
                "Could not inspect notification shortcut",
                error.as_ref(),
            ),
        }
    }
    Ok(())
}

fn shortcut_target(path: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt as _;
    let path = HSTRING::from(path.as_os_str());
    let mut buffer = vec![0; 32768];
    // SAFETY: Load/GetPath use live UTF-16 buffers synchronously; no shortcut
    // resolution or target execution is requested.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let file: IPersistFile = link.cast()?;
        file.Load(PCWSTR(path.as_ptr()), STGM_READ)?;
        link.GetPath(&mut buffer, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)?;
    }
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .context("Unterminated shortcut target")?;
    Ok(std::ffi::OsString::from_wide(&buffer[..length]).into())
}

pub(super) fn register_identity() -> anyhow::Result<()> {
    initialize_apartment()?;
    let directory = daruda_store::persistence::default_data_dir();
    let digest = format!(
        "{:x}",
        Sha256::digest(directory.as_os_str().as_encoded_bytes())
    );
    let id = format!("Daruda.Desktop.{}", &digest[..16]);
    let executable = std::env::current_exe()?;
    let programs = programs_directory()?;
    std::fs::create_dir_all(&programs)?;
    let shortcut = programs.join(format!("daruda-{}.lnk", &digest[..16]));
    let path = HSTRING::from(executable.as_os_str());
    let id_wide = HSTRING::from(&id);
    let shortcut_wide = HSTRING::from(shortcut.as_os_str());
    // SAFETY: all pointers refer to live UTF-16 buffers for these synchronous
    // COM calls. SetValue copies the borrowed string; it owns no allocation.
    unsafe {
        SetCurrentProcessExplicitAppUserModelID(PCWSTR(id_wide.as_ptr()))?;
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(PCWSTR(path.as_ptr()))?;
        let store: IPropertyStore = link.cast()?;
        let mut value = PROPVARIANT::default();
        (*value.Anonymous.Anonymous).vt = VT_LPWSTR;
        (*value.Anonymous.Anonymous).Anonymous.pwszVal = PWSTR(id_wide.as_ptr().cast_mut());
        let key = PROPERTYKEY {
            fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
            pid: 5,
        };
        store.SetValue(&key, &value)?;
        store.Commit()?;
        let file: IPersistFile = link.cast()?;
        file.Save(PCWSTR(shortcut_wide.as_ptr()), true)?;
    }
    super::IDENTITY
        .set(id)
        .map_err(|_| anyhow::anyhow!("notification identity already registered"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn uninstall_removes_only_shortcuts_targeting_this_executable() {
        use super::*;
        initialize_apartment().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let owned = directory.path().join("daruda-0000000000000001.lnk");
        let other = directory.path().join("daruda-0000000000000002.lnk");
        let executable = directory.path().join("owned.exe");
        let other_executable = directory.path().join("other.exe");
        for (shortcut, target) in [(&owned, &executable), (&other, &other_executable)] {
            let target = HSTRING::from(target.as_os_str());
            let shortcut = HSTRING::from(shortcut.as_os_str());
            // SAFETY: temporary COM objects borrow live buffers synchronously.
            unsafe {
                let link: IShellLinkW =
                    CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
                link.SetPath(PCWSTR(target.as_ptr())).unwrap();
                let file: IPersistFile = link.cast().unwrap();
                file.Save(PCWSTR(shortcut.as_ptr()), true).unwrap();
            }
        }
        unregister_in(directory.path(), &executable).unwrap();
        assert!(!owned.exists());
        assert!(other.exists());
    }
}
