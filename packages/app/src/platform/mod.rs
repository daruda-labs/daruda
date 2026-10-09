//! Thin wrappers around platform APIs that GPUI does not
//! abstract. Keep each module narrow — one OS API per file — so the
//! `unsafe` surface stays auditable.

pub mod attention;
pub(crate) mod authenticode;
#[cfg(all(windows, feature = "screenshot"))]
pub(crate) mod capture_windows;
pub(crate) mod desktop;
pub(crate) mod desktop_instance;
pub(crate) mod installer_update;
pub(crate) mod local_socket;
pub mod notifications;
pub(crate) mod ports;
#[cfg(windows)]
pub(crate) mod power;
pub mod presence;
pub(crate) mod startup_failure;
#[cfg(windows)]
mod taskbar_windows;
pub(crate) mod window_controls;

/// Record a native capability failure without coupling its domain to IPC.
#[track_caller]
pub(crate) fn report_error(domain: &str, message: &str, error: &dyn std::error::Error) {
    use daruda_store::observability::{
        error_report::{ErrorReport, ErrorSeverity},
        log_writer::LogWriter,
    };
    let caller = std::panic::Location::caller();
    LogWriter::log(
        ErrorReport::new(message)
            .severity(ErrorSeverity::Warning)
            .from_error(error)
            .at(caller.file(), caller.line())
            .dedup(domain)
            .build(),
    );
}
