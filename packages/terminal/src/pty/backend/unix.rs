//! Unix output delivery ends when its receiver disconnects.

use portable_pty::MasterPty;
use std::sync::Arc;

pub(in crate::pty) struct OutputLifecycle;

impl OutputLifecycle {
    pub(in crate::pty) fn closing(&self) -> bool {
        false
    }
    pub(in crate::pty) fn stop_reading(&self, delivered: bool) -> bool {
        !delivered
    }
}

pub(in crate::pty) fn wrap_master(
    master: Box<dyn MasterPty + Send>,
) -> (Arc<dyn MasterPty + Send>, OutputLifecycle) {
    (Arc::from(master), OutputLifecycle)
}

pub(in crate::pty) fn runs_foreground_job(
    master: &(dyn MasterPty + Send),
    shell_pid: Option<u32>,
) -> bool {
    #[cfg(unix)]
    return match (master.process_group_leader(), shell_pid) {
        (Some(group), Some(shell)) => i64::from(group) != i64::from(shell),
        _ => false,
    };
    #[cfg(not(unix))]
    {
        let _ = (master, shell_pid);
        false
    }
}
