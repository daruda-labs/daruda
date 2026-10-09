//! User-scoped Generic Credentials; the OS encrypts persisted secret blobs.

use std::io;
use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
use windows_sys::Win32::Security::Credentials::{
    CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW,
    CredDeleteW, CredFree, CredReadW, CredWriteW,
};

fn target(service: &str, account: &str) -> io::Result<Vec<u16>> {
    if service.contains('\0') || account.contains('\0') || service.is_empty() || account.is_empty()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid credential identifier",
        ));
    }
    Ok(format!("{service}/{account}")
        .encode_utf16()
        .chain(Some(0))
        .collect())
}

pub(super) fn read(service: &str, account: &str) -> io::Result<Option<String>> {
    let target = target(service, account)?;
    let mut credential = std::ptr::null_mut();
    // SAFETY: target is NUL-terminated; the OS owns the returned allocation.
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0 {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
            Ok(None)
        } else {
            Err(error)
        };
    }
    // SAFETY: CredRead succeeded, and this blob is valid until CredFree.
    let bytes = unsafe {
        let credential = &*credential;
        if credential.CredentialBlobSize == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(
                credential.CredentialBlob,
                credential.CredentialBlobSize as usize,
            )
            .to_vec()
        }
    };
    unsafe {
        CredFree(credential.cast());
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Credential encoding is invalid"))
}

pub(super) fn write(service: &str, account: &str, value: &str) -> io::Result<()> {
    let mut target = target(service, account)?;
    if value.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Credential exceeds OS size limit",
        ));
    }
    let mut username: Vec<u16> = account.encode_utf16().chain(Some(0)).collect();
    let mut bytes = value.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        CredentialBlobSize: bytes.len() as u32,
        CredentialBlob: bytes.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: username.as_mut_ptr(),
        ..Default::default()
    };
    // SAFETY: every field points to storage valid through this synchronous call.
    let written = unsafe { CredWriteW(&credential, 0) };
    bytes.fill(0);
    if written == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) fn delete(service: &str, account: &str) -> io::Result<()> {
    let target = target(service, account)?;
    // SAFETY: target is NUL-terminated and only identifies this service/account.
    if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_credential_roundtrip_isolated_from_existing_accounts() {
        let name = format!("credential-test-{}", uuid::Uuid::new_v4());
        write(&name, "test", "synthetic-secret").unwrap();
        let result = read(&name, "test");
        delete(&name, "test").unwrap();
        assert_eq!(result.unwrap().as_deref(), Some("synthetic-secret"));
        assert_eq!(read(&name, "test").unwrap(), None);
    }
}
