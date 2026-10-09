//! Native PTY lifetime and foreground-job behavior.

#[cfg(not(windows))]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
pub(super) use unix::{runs_foreground_job, wrap_master};
#[cfg(windows)]
pub(super) use windows::{runs_foreground_job, wrap_master};
