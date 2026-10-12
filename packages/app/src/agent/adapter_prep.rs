//! The one door through which the app prepares an ACP adapter: a session's
//! connect and an agent CLI run (login, auth status) alike.
//!
//! GPUI-free and blocking. It owns what every caller would otherwise repeat:
//! the shared install root, the offline guard for tests, and where
//! preparation notices go.

use daruda_acp::preparation::{PreparationCancellation, PreparationContext};
use daruda_acp::{AcpClientError, LaunchSpec, NodeProgress, PreparedAdapter};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

/// Why preparation fails under test, where no adapter is ever installed.
pub(crate) const OFFLINE_PREPARATION: &str = "adapters are not prepared in tests";

/// Install (or reuse) and validate `spec`'s adapter.
///
/// **Blocking**: it may download Node.js or install the adapter. Run it on
/// the background executor, never the UI update thread.
pub(crate) fn prepare(
    spec: &LaunchSpec,
    cancellation: &PreparationCancellation,
    progress: &mut dyn FnMut(NodeProgress),
) -> Result<PreparedAdapter, AcpClientError> {
    // Tests must never prepare: it runs `npm` against the registry into the
    // shared install root.
    if cfg!(test) {
        return Err(daruda_acp::NodeError::Download(OFFLINE_PREPARATION.to_owned()).into());
    }
    let canceled = || cancellation.is_canceled();
    let context = PreparationContext::new(&canceled, &log_notice);
    let adapter = daruda_acp::launch_env::prepare_adapter(
        spec,
        &daruda_store::persistence::node_install_dir(),
        progress,
        &context,
    )?;
    context.check()?;
    Ok(adapter)
}

/// Record a preparation notice (a cached-version fallback, a deferred
/// cleanup) in the log.
fn log_notice(detail: &str) {
    LogWriter::log(
        ErrorReport::new("ACP adapter preparation notice")
            .severity(ErrorSeverity::Warning)
            .with_context("detail", detail)
            .at(file!(), line!())
            .build(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_never_prepare_an_adapter() {
        let spec = LaunchSpec {
            command: "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
            strip_env: Vec::new(),
        };
        let Err(error) = prepare(&spec, &PreparationCancellation::default(), &mut |_| {}) else {
            panic!("a test run prepared an adapter");
        };
        assert!(error.to_string().contains(OFFLINE_PREPARATION), "{error}");
    }
}
