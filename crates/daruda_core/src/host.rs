//! This machine's network name — what tells a report about this machine
//! apart from one about another (a shell reporting its cwd over SSH).

/// The host name, or `None` when the OS will not say.
pub fn name() -> Option<String> {
    imp::name().filter(|name| !name.is_empty())
}

#[cfg(unix)]
mod imp {
    /// `HOST_NAME_MAX` is 255 on Linux and macOS; one more for the NUL.
    const CAPACITY: usize = 256;

    pub(super) fn name() -> Option<String> {
        let mut buf = [0u8; CAPACITY];
        // SAFETY: `buf` is writable for `buf.len()` bytes, which is all
        // `gethostname` may write.
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        if rc != 0 {
            return None;
        }
        // POSIX leaves a truncated name unterminated; refuse it over guessing.
        let end = buf.iter().position(|&b| b == 0)?;
        String::from_utf8(buf[..end].to_vec()).ok()
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::SystemInformation::{
        ComputerNameDnsHostname, GetComputerNameExW,
    };

    pub(super) fn name() -> Option<String> {
        let mut len: u32 = 0;
        // SAFETY: a null buffer of length 0 only asks for the size needed,
        // which the call writes to `len` (NUL included) as it fails.
        unsafe { GetComputerNameExW(ComputerNameDnsHostname, std::ptr::null_mut(), &mut len) };
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u16; len as usize];
        // SAFETY: `buf` holds `len` UTF-16 units; on success `len` becomes
        // the count written, NUL excluded.
        let ok = unsafe { GetComputerNameExW(ComputerNameDnsHostname, buf.as_mut_ptr(), &mut len) };
        if ok == 0 {
            return None;
        }
        String::from_utf16(buf.get(..len as usize)?).ok()
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    pub(super) fn name() -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_machine_has_a_name() {
        let name = name().expect("the OS names this machine");
        assert!(!name.contains('\0'));
    }
}
