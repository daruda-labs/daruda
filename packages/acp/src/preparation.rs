//! Cancellation and classified failures for work done before ACP starts.
pub(crate) mod process;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// The recovery category survives npm diagnostics and host localization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparationKind {
    Network,
    Timeout,
    Canceled,
    Configuration,
    Integrity,
    InvalidPackage,
    Io,
    Process,
}

impl PreparationKind {
    pub fn retryable(self) -> bool {
        matches!(self, Self::Network | Self::Timeout | Self::Io)
    }

    pub(crate) fn allows_cached(self) -> bool {
        matches!(self, Self::Network | Self::Timeout)
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{detail}")]
pub struct PreparationError {
    pub kind: PreparationKind,
    pub detail: String,
}

impl PreparationError {
    pub(crate) fn new(kind: PreparationKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

impl From<std::io::Error> for PreparationError {
    fn from(error: std::io::Error) -> Self {
        let kind = match error.kind() {
            std::io::ErrorKind::PermissionDenied => PreparationKind::Configuration,
            _ => PreparationKind::Io,
        };
        Self::new(kind, error.to_string())
    }
}

impl From<anyhow::Error> for PreparationError {
    fn from(error: anyhow::Error) -> Self {
        let detail = format!("{error:#}");
        let kind = error
            .downcast_ref::<Self>()
            .map(|error| error.kind)
            .or_else(|| {
                error.downcast_ref::<std::io::Error>().map(|error| {
                    if error.kind() == std::io::ErrorKind::PermissionDenied {
                        PreparationKind::Configuration
                    } else {
                        PreparationKind::Io
                    }
                })
            })
            .unwrap_or(PreparationKind::InvalidPackage);
        Self::new(kind, detail)
    }
}

/// An existing host cancellation source can be borrowed without another poller.
pub struct PreparationContext<'a> {
    canceled: &'a dyn Fn() -> bool,
    notice: &'a dyn Fn(&str),
}

impl Default for PreparationContext<'_> {
    fn default() -> Self {
        Self {
            canceled: &|| false,
            notice: &|_| {},
        }
    }
}

impl<'a> PreparationContext<'a> {
    pub fn new(canceled: &'a dyn Fn() -> bool, notice: &'a dyn Fn(&str)) -> Self {
        Self { canceled, notice }
    }

    pub fn check(&self) -> Result<(), PreparationError> {
        if (self.canceled)() {
            Err(PreparationError::new(
                PreparationKind::Canceled,
                "adapter preparation canceled",
            ))
        } else {
            Ok(())
        }
    }

    pub(crate) fn notice(&self, detail: &str) {
        (self.notice)(detail);
    }
}

/// Dropping the foreground owner interrupts its blocking preparation worker.
#[derive(Clone, Default)]
pub struct PreparationCancellation(Arc<AtomicBool>);

impl PreparationCancellation {
    pub fn is_canceled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub fn cancel_on_drop(&self) -> CancelPreparationOnDrop {
        CancelPreparationOnDrop(self.clone())
    }
}

pub struct CancelPreparationOnDrop(PreparationCancellation);

impl Drop for CancelPreparationOnDrop {
    fn drop(&mut self) {
        self.0.0.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_owner_cancels_worker_and_is_not_retryable() {
        let cancel = PreparationCancellation::default();
        let guard = cancel.cancel_on_drop();
        let check = || cancel.is_canceled();
        let context = PreparationContext::new(&check, &|_| {});
        assert!(context.check().is_ok());
        drop(guard);
        let error = context.check().unwrap_err();
        assert_eq!(error.kind, PreparationKind::Canceled);
        assert!(!error.kind.retryable());
    }
}
