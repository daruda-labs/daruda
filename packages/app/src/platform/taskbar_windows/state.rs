//! Requested overlay state and last successful application, without Win32.

#[derive(Clone, Copy, Default)]
pub(super) struct Overlay {
    requested: usize,
    applied: Option<usize>,
}

impl Overlay {
    pub(super) const fn new() -> Self {
        Self {
            requested: 0,
            applied: None,
        }
    }

    pub(super) fn request(&mut self, count: usize) {
        self.requested = count;
    }

    pub(super) fn invalidate(&mut self) {
        self.applied = None;
    }

    pub(super) fn synchronize(
        &mut self,
        invalidated: bool,
        available: bool,
        apply: impl FnOnce(usize) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        if invalidated {
            self.invalidate();
        }
        if available && self.applied != Some(self.requested) {
            self.invalidate();
            apply(self.requested)?;
            self.applied = Some(self.requested);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
