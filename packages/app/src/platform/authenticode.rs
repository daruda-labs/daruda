//! Verify a Windows update against the publisher of the running application.

use std::path::Path;

pub(crate) fn verify_update(current: &Path, candidate: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let installed = windows::publisher(current)?;
        let replacement = windows::publisher(candidate)?;
        check_publisher(installed.as_deref(), replacement.as_deref())
    }
    #[cfg(not(windows))]
    {
        let _ = (current, candidate);
        Ok(())
    }
}

#[cfg(any(windows, test))]
fn check_publisher(installed: Option<&[u8]>, replacement: Option<&[u8]>) -> std::io::Result<()> {
    if installed.is_some() && installed != replacement {
        return Err(std::io::Error::other(
            "Update publisher does not match the installed application",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signed_install_cannot_change_publishers_or_become_unsigned() {
        assert!(check_publisher(Some(b"publisher"), Some(b"publisher")).is_ok());
        assert!(check_publisher(Some(b"publisher"), Some(b"other")).is_err());
        assert!(check_publisher(Some(b"publisher"), None).is_err());
        assert!(check_publisher(None, Some(b"publisher")).is_ok());
        assert!(check_publisher(None, None).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn native_verification_accepts_an_embedded_signer_and_rejects_unsigned_replacement() {
        // PowerShell's system executable is catalog-signed; Git carries an embedded signature.
        let signed = daruda_core::process::resolve_program(std::ffi::OsStr::new("git"), None);
        assert!(
            signed.is_file(),
            "Git for Windows is required by this fixture"
        );
        assert!(windows::publisher(&signed).unwrap().is_some());
        assert!(verify_update(&signed, &signed).is_ok());
        assert!(verify_update(&signed, &std::env::current_exe().unwrap()).is_err());
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Foundation::TRUST_E_NOSIGNATURE;
    use windows_sys::Win32::Security::WinTrust::*;

    pub(super) fn publisher(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
        // Opening separately distinguishes missing files from unsigned PE files.
        let _file = std::fs::File::open(path)?;
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut file = WINTRUST_FILE_INFO {
            cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: path.as_ptr(),
            ..Default::default()
        };
        let mut data = WINTRUST_DATA {
            cbStruct: size_of::<WINTRUST_DATA>() as u32,
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_WHOLECHAIN,
            dwUnionChoice: WTD_CHOICE_FILE,
            Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
            dwStateAction: WTD_STATEACTION_VERIFY,
            dwProvFlags: WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT | WTD_DISABLE_MD2_MD4,
            ..Default::default()
        };
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        // SAFETY: paths and structures stay live until the provider state closes.
        let status = unsafe {
            WinVerifyTrust(
                std::ptr::null_mut(),
                &mut action,
                std::ptr::from_mut(&mut data).cast(),
            )
        };
        let result = if status == 0 {
            subject(&data).map(Some)
        } else if status == TRUST_E_NOSIGNATURE {
            // Existing unsigned releases still rely on the verified release checksum.
            Ok(None)
        } else {
            Err(std::io::Error::other(format!(
                "Authenticode verification failed: 0x{:08X}",
                status as u32
            )))
        };
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        // SAFETY: closes only the state produced by the matching verification above.
        unsafe {
            WinVerifyTrust(
                std::ptr::null_mut(),
                &mut action,
                std::ptr::from_mut(&mut data).cast(),
            )
        };
        result
    }

    fn subject(data: &WINTRUST_DATA) -> std::io::Result<Vec<u8>> {
        // SAFETY: this state is still owned by WinVerifyTrust. Copy the leaf
        // subject before closing it; never use the timestamp countersigner.
        unsafe {
            let provider = WTHelperProvDataFromStateData(data.hWVTStateData);
            if !provider.is_null() {
                let signer = WTHelperGetProvSignerFromChain(provider, 0, 0, 0);
                if !signer.is_null()
                    && (*signer).csCertChain > 0
                    && !(*signer).pasCertChain.is_null()
                {
                    let certificate = (*(*signer).pasCertChain).pCert;
                    if !certificate.is_null() && !(*certificate).pCertInfo.is_null() {
                        let subject = (*(*certificate).pCertInfo).Subject;
                        if subject.cbData > 0 && !subject.pbData.is_null() {
                            return Ok(std::slice::from_raw_parts(
                                subject.pbData,
                                subject.cbData as usize,
                            )
                            .to_vec());
                        }
                    }
                }
            }
        }
        Err(std::io::Error::other(
            "Verified update has no publisher certificate",
        ))
    }
}
