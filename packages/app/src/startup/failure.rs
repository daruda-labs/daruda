//! Terminal startup failures must survive exit and remain visible to the user.

use daruda_store::observability::{
    error_report::{ErrorReport, ErrorSeverity},
    log_writer::LogWriter,
};

#[track_caller]
pub(super) fn exit(domain: &str, message: &str, error: &dyn std::error::Error) -> ! {
    use crate::surface::strings as s;

    crate::globals::apply_locale_str(&daruda_config::Config::load().general.language);
    let caller = std::panic::Location::caller();
    let report = ErrorReport::new(message)
        .severity(ErrorSeverity::Error)
        .from_error(error)
        .at(caller.file(), caller.line())
        .dedup(domain)
        .build();
    let persistence = match LogWriter::log_sync(&report) {
        Ok(path) => s::startup::failure_log(path.display()),
        Err(error) => s::startup::failure_log_unavailable(error),
    };
    let detail = format!(
        "{}\n\n{}\n\n{persistence}",
        s::startup::failure_recovery(),
        report.to_plain_text()
    );
    if crate::smoke::requested() {
        // Automated smoke runs must fail without waiting for a dialog answer.
        println!("{detail}");
    } else {
        crate::platform::startup_failure::show(detail);
    }
    std::process::exit(1);
}
