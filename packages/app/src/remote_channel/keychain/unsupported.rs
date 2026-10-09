//! Explicit fallback for hosts without a credential backend.

use std::io;
pub(super) fn read(_service: &str, _account: &str) -> io::Result<Option<String>> {
    Ok(None)
}
pub(super) fn write(_service: &str, _account: &str, _value: &str) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "OS credential storage is unavailable",
    ))
}
pub(super) fn delete(service: &str, account: &str) -> io::Result<()> {
    write(service, account, "")
}
