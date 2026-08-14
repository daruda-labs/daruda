//! Claude Code hook integration — push channel.
//!
//! Wires `daruda_agent::hooks::*` (GPUI-free FSM) into the GPUI app:
//!
//! - [`handler`] — implementation of the `daruda --hook <eventType>`
//!   subcommand that Claude Code spawns for each hook event. Reads
//!   stdin, runs the FSM, writes `~/.daruda/status/<session_id>.json`.
//!
pub mod flow_watcher;
pub mod handler;
pub mod installer;
pub mod jsonl_watcher;
pub mod mcp_watcher;
pub mod pty_tracker;
pub mod skills_watcher;
pub mod watcher;

#[cfg(test)]
pub(crate) mod tests_common {
    //! Cross-test serialization for the FSEvent watchers.
    //!
    //! macOS aggregates filesystem events under shared `/var/folders`
    //! parents, so parallel `notify::Watcher` tests overlap each other's
    //! event bursts and miss their deadlines. A process-global mutex
    //! serializes the watcher tests; pure-data tests keep running in
    //! parallel.
    use std::sync::{Mutex, MutexGuard};

    static FSEVENT_LOCK: Mutex<()> = Mutex::new(());

    /// Acquire the FSEvent serialization mutex. Use as
    /// `let _g = fsevent_serial();` at the top of any test that
    /// spawns a `notify::Watcher`. The guard releases on scope exit
    /// and the next waiting test proceeds.
    pub(crate) fn fsevent_serial() -> MutexGuard<'static, ()> {
        // Poison only signals that a previous test panicked while
        // holding the guard — irrelevant here since the lock protects
        // ordering, not shared state. Recover and keep going so one
        // failing test doesn't cascade into a wave of poisoned ones.
        FSEVENT_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
