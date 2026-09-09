//! Working directory, standing instructions, and diagnostics for the orchestrator.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OpenError {
    /// The orchestrator's own working directory could not be created.
    CwdUnavailable(String),
    /// The control socket could not be served, so the orchestrator would have
    /// no tools. Refused rather than opened: an agent that cannot act is
    /// worse than no agent, because it will try anyway.
    ControlSurfaceUnavailable(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CwdUnavailable(e) => write!(f, "orchestrator cwd unavailable: {e}"),
            Self::ControlSurfaceUnavailable(e) => write!(f, "control surface unavailable: {e}"),
        }
    }
}

/// `default_data_dir()/orchestrator`. The orchestrator's capability is its
/// tools, not the filesystem — reading and editing code is a lane agent's job
/// — so it is rooted somewhere with nothing in it but its own instructions.
pub(crate) fn cwd() -> Result<std::path::PathBuf, OpenError> {
    let dir = daruda_store::persistence::default_data_dir().join("orchestrator");
    std::fs::create_dir_all(&dir).map_err(|e| OpenError::CwdUnavailable(e.to_string()))?;
    Ok(dir)
}

/// Leave the briefing in `dir` as a standing instruction file.
///
/// Both filenames, same text: an agent reads the one its own convention names
/// (`CLAUDE.md` for Claude, `AGENTS.md` for Codex), and which agent runs the
/// orchestrator is a setting the person can change after this file is written.
/// One that reads both sees the instructions twice, which costs a little
/// context and changes nothing.
///
/// Rewritten every start, so it cannot fall behind the tool vocabulary — and
/// best effort, because the prompt-borne [`briefing`](super::briefing::briefing)
/// is the copy the session actually depends on. A failure here is logged and
/// the orchestrator still opens.
pub(crate) fn install_instructions(dir: &std::path::Path) {
    let text = super::briefing::instructions();
    for name in ["CLAUDE.md", "AGENTS.md"] {
        let path = dir.join(name);
        if let Err(e) = std::fs::write(&path, &text) {
            LogWriter::log(
                ErrorReport::new("Orchestrator instruction file could not be written")
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .with_context("file", name.to_owned())
                    .at(file!(), line!())
                    .dedup("orchestrator.instructions")
                    .build(),
            );
        }
    }
}

/// Record a failed open. The caller answers the phone with its own wording, so
/// this only makes sure the concrete reason reaches the log.
pub(crate) fn log_open_failure(error: &OpenError) {
    LogWriter::log(
        ErrorReport::new("Orchestrator could not start")
            .severity(ErrorSeverity::Error)
            .at(file!(), line!())
            .with_context("reason", error.to_string())
            .dedup("orchestrator.open")
            .build(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_open_failure_has_a_diagnostic() {
        for case in [
            OpenError::CwdUnavailable("permission denied".into()),
            OpenError::ControlSurfaceUnavailable("already running".into()),
        ] {
            assert!(!case.to_string().is_empty(), "{case:?}");
        }
    }

    /// Both conventions get the file, and a second start rewrites rather than
    /// appending or leaving a stale copy.
    #[test]
    fn the_instructions_land_under_both_conventional_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        install_instructions(dir.path());
        for name in ["CLAUDE.md", "AGENTS.md"] {
            let text = std::fs::read_to_string(dir.path().join(name)).expect(name);
            assert_eq!(text, super::super::briefing::instructions(), "{name}");
        }

        install_instructions(dir.path());
        let again = std::fs::read_to_string(dir.path().join("CLAUDE.md")).expect("rewritten");
        assert_eq!(
            again,
            super::super::briefing::instructions(),
            "a restart replaces the file rather than growing it"
        );
    }

    /// An unwritable directory must not stop the orchestrator: the prompt
    /// carries the same briefing.
    #[test]
    fn an_unwritable_directory_is_survivable() {
        install_instructions(std::path::Path::new("/proc/nonexistent-daruda"));
    }

    #[test]
    fn the_cwd_is_created_under_the_profile_data_dir() {
        let dir = cwd().expect("cwd");
        assert!(dir.is_dir());
        assert!(dir.starts_with(daruda_store::persistence::default_data_dir()));
        assert_eq!(
            dir.file_name().and_then(|n| n.to_str()),
            Some("orchestrator")
        );
    }
}
