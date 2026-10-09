//! ConPTY must drain output while ClosePseudoConsole waits for its pipe.

use portable_pty::MasterPty;
use std::sync::{Arc, Weak};

mod draining_master;

pub(in crate::pty) struct OutputLifecycle(Weak<()>);

impl OutputLifecycle {
    pub(in crate::pty) fn closing(&self) -> bool {
        self.0.strong_count() == 0
    }
    pub(in crate::pty) fn stop_reading(&self, _delivered: bool) -> bool {
        false
    }
}

pub(in crate::pty) fn wrap_master(
    master: Box<dyn MasterPty + Send>,
) -> (Arc<dyn MasterPty + Send>, OutputLifecycle) {
    let alive = Arc::new(());
    let output = OutputLifecycle(Arc::downgrade(&alive));
    let master = Arc::new(draining_master::DrainingMaster {
        _alive: alive,
        inner: std::sync::Mutex::new(master),
    });
    (master, output)
}

pub(in crate::pty) fn runs_foreground_job(
    _master: &(dyn MasterPty + Send),
    shell_pid: Option<u32>,
) -> bool {
    shell_pid.is_some_and(|pid| daruda_core::process::has_descendants(pid).unwrap_or(true))
}
