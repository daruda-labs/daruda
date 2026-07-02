//! Self-update integration for the GPUI app.
//!
//! Wraps the GPUI-free `daruda_update` crate in an [`Updater`] entity behind
//! a process-wide global, so views can resolve the live handle and drive the
//! check → download → install → restart flow. See [`updater`] for the state
//! machine and the mandatory off-main-thread execution of the blocking
//! `daruda_update` calls.

mod updater;

pub use updater::*;

use gpui::App;

/// Register the [`Updater`] global. Thin wrapper over [`Updater::init`] so
/// `globals::init_all` can call `crate::update::init(cx)` alongside the other
/// global initialisers. Idempotent.
pub fn init(cx: &mut App) {
    Updater::init(cx);
}
