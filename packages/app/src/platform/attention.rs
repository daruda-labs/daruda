//! Native attention, foreground and idle capabilities.
//!
//! Linux uses X11/EWMH. Without an X server, attention is a no-op,
//! foreground is false and idle time is unavailable. A missing idle sensor
//! is represented by None rather than zero seconds.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as backend;
#[cfg(target_os = "macos")]
use macos as backend;
#[cfg(windows)]
use windows as backend;

#[cfg(not(test))]
pub use backend::system_idle_seconds;
pub use backend::{apply, is_app_active, set_badge_count};
