//! Signal producer teardown before ClosePseudoConsole waits on its output pipe.

use portable_pty::{MasterPty, PtySize};
use std::sync::{Arc, Mutex, MutexGuard};

pub(super) struct DrainingMaster {
    // Field order matters: release this marker before closing the native PTY.
    pub(super) _alive: Arc<()>,
    pub(super) inner: Mutex<Box<dyn MasterPty + Send>>,
}

impl DrainingMaster {
    fn lock(&self) -> gpui::Result<MutexGuard<'_, Box<dyn MasterPty + Send>>> {
        self.inner
            .lock()
            .map_err(|_| std::io::Error::other("PTY master lock poisoned").into())
    }
}

impl MasterPty for DrainingMaster {
    fn resize(&self, size: PtySize) -> gpui::Result<()> {
        self.lock()?.resize(size)
    }
    fn get_size(&self) -> gpui::Result<PtySize> {
        self.lock()?.get_size()
    }
    fn try_clone_reader(&self) -> gpui::Result<Box<dyn std::io::Read + Send>> {
        self.lock()?.try_clone_reader()
    }
    fn take_writer(&self) -> gpui::Result<Box<dyn std::io::Write + Send>> {
        self.lock()?.take_writer()
    }
}
