//! Explicit fallback for platforms without native process groups.
use std::{ffi::OsStr, io, path::PathBuf, process::Command};
pub(super) fn command(program: &OsStr) -> Command {
    Command::new(program)
}
pub(super) fn resolve_program(program: &OsStr, _path: Option<&OsStr>) -> PathBuf {
    program.into()
}
pub(super) fn lead_own_group(_command: &mut Command) {}
pub(super) fn is_alive(_pid: u32) -> bool {
    false
}
pub(super) fn has_descendants(_pid: u32) -> io::Result<bool> {
    Err(unavailable())
}
fn unavailable() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "Process groups unavailable")
}
#[derive(Debug)]
pub(super) struct GroupInner;
impl GroupInner {
    pub(super) fn try_adopt(pid: u32) -> io::Result<Self> {
        Ok(Self::adopt(pid))
    }
    pub(super) fn adopt(_pid: u32) -> Self {
        Self
    }
    pub(super) fn try_kill_tree(&self) -> io::Result<()> {
        Err(unavailable())
    }
    pub(super) fn kill_tree(&self) {}
}
