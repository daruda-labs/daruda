//! Durable terminal observations, independent of window and task projections.
//!
//! The hook writer commits before publishing its replaceable status file.
//! Desktop lifecycle writers share the same stable per-terminal lock, so a
//! short session remains discoverable even if no watcher ever read its file.

use std::path::Path;

use daruda_store::observability::{
    error_report::{ErrorReport, ErrorSeverity},
    log_writer::LogWriter,
    system_info::redact_home,
};
use daruda_store::terminal_sessions::{
    Conversation, Observation, ObservationKind, SessionEvent, SessionHistory, SessionJournal,
    TerminalEndReason,
};

use super::route::{Harness, TerminalId};
use super::status_file::{StatusFile, lock::SessionLock};

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("terminal history I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Journal(#[from] daruda_store::terminal_sessions::HistoryError),
}

fn directory(status_dir: &Path, terminal: &TerminalId) -> std::path::PathBuf {
    daruda_store::persistence::terminal_history_dir(status_dir).join(terminal.as_str())
}

/// Hold the terminal's status lock across this call and the status publication.
pub(in crate::hooks) fn observe_locked(
    status_dir: &Path,
    file: &StatusFile,
) -> Result<(), HistoryError> {
    let Some(terminal) = file.terminal_id.as_ref() else {
        return Ok(());
    };
    let provider = match file.harness() {
        Harness::Claude => daruda_store::accounts::AccountRecipeId::Claude,
        Harness::Codex => daruda_store::accounts::AccountRecipeId::Codex,
    };
    let config_home = crate::accounts::recipe_for(provider).system_home_dir();
    let observation = Observation {
        conversation: Conversation {
            provider,
            config_home,
            session_id: file.session_id.clone(),
        },
        at: file.timestamp,
        cwd: file.cwd.clone(),
        transcript_path: file.transcript_path.clone(),
        agent_pid: file.agent_pid,
        kind: match file.last_event.as_str() {
            "SessionStart" => ObservationKind::SessionStart,
            "SessionEnd" => ObservationKind::SessionEnd,
            _ => ObservationKind::Update,
        },
    };
    SessionJournal::open(&directory(status_dir, terminal))?
        .commit(SessionEvent::Observed { observation })?;
    Ok(())
}

/// Desktop facts serialize with hook processes even after a pane is gone.
pub fn record(
    status_dir: &Path,
    terminal: &TerminalId,
    event: SessionEvent,
) -> Result<(), HistoryError> {
    std::fs::create_dir_all(status_dir)?;
    let _lock = SessionLock::acquire(status_dir, terminal.as_str())?;
    record_locked(status_dir, terminal, event)
}

fn record_locked(
    status_dir: &Path,
    terminal: &TerminalId,
    event: SessionEvent,
) -> Result<(), HistoryError> {
    SessionJournal::open(&directory(status_dir, terminal))?.commit(event)?;
    Ok(())
}

pub fn read(status_dir: &Path, terminal: &TerminalId) -> Result<SessionHistory, HistoryError> {
    Ok(SessionJournal::read(&directory(status_dir, terminal))?)
}

/// Enumerate retained terminal identities; pane deletion never removes them.
pub fn terminals(status_dir: &Path) -> Result<Vec<TerminalId>, HistoryError> {
    let entries =
        match std::fs::read_dir(daruda_store::persistence::terminal_history_dir(status_dir)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
    let mut terminals = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && let Some(terminal) = entry.file_name().to_str().and_then(TerminalId::parse)
        {
            terminals.push(terminal);
        }
    }
    terminals.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    Ok(terminals)
}

/// Capture identities before new terminals spawn; reconciliation itself may
/// run later without retiring terminals created after this snapshot.
pub fn reconcile_terminals(
    status_dir: &Path,
    terminals: &[TerminalId],
) -> Result<(), HistoryError> {
    let mut first_error = None;
    for terminal in terminals {
        let result = (|| {
            let _lock = SessionLock::acquire(status_dir, terminal.as_str())?;
            if !read(status_dir, terminal)?.terminal_is_closed() {
                record_locked(
                    status_dir,
                    terminal,
                    SessionEvent::TerminalClosed {
                        reason: TerminalEndReason::ObservationLost,
                        at: chrono::Utc::now(),
                    },
                )?;
            }
            Ok::<_, HistoryError>(())
        })();
        if let Err(error) = result {
            LogWriter::log(reconciliation_error_report(status_dir, terminal, &error));
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn reconciliation_error_report(
    status_dir: &Path,
    terminal: &TerminalId,
    error: &HistoryError,
) -> ErrorReport {
    ErrorReport::new("Terminal history recovery failed")
        .severity(ErrorSeverity::Warning)
        .from_error(error)
        .at(file!(), line!())
        .with_context("terminal", terminal.as_str())
        .with_context("path", redact_home(directory(status_dir, terminal)))
        .dedup(format!("terminal_history.reconcile.{terminal}"))
        .build()
}

#[cfg(test)]
mod tests;
