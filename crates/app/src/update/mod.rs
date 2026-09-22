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
    // A portable install keeps the files it replaced until something can
    // delete them, which is any run after the one that held them open.
    if let Ok(exe) = cx.app_path()
        && let Some(root) = exe.parent()
    {
        daruda_update::sweep_aside(root);
    }
}

/// How often to ask whether the process being replaced has gone.
const EXIT_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// How long to wait before giving up on it. Generous: a quit can be slow, and
/// starting a second instance while the first still holds the control socket
/// is worse than starting late.
const EXIT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(120);

/// Wait for `pid` to exit, then start this executable with no arguments.
///
/// The body of `daruda --await-exit`. Returns the exit code for `main`.
pub fn await_exit_and_start(pid: u32) -> i32 {
    let deadline = std::time::Instant::now() + EXIT_DEADLINE;
    while daruda_core::process::is_alive(pid) {
        if std::time::Instant::now() >= deadline {
            // Starting anyway would put two instances on one control socket.
            return 1;
        }
        std::thread::sleep(EXIT_POLL);
    }
    let Ok(exe) = std::env::current_exe() else {
        return 1;
    };
    match daruda_core::process::command(exe).spawn() {
        Ok(_) => 0,
        Err(_) => 1,
    }
}
