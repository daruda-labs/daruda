//! Ordered desktop lifecycle facts for the hook-owned terminal journals.

use std::path::PathBuf;

use daruda_agent::hooks::route::TerminalId;
use daruda_store::observability::{error_report::ErrorReport, log_writer::LogWriter};
use daruda_store::terminal_sessions::SessionEvent;
use gpui::App;

pub(super) struct Writer {
    sender: smol::channel::Sender<Command>,
}

enum Command {
    Record {
        terminal: TerminalId,
        event: SessionEvent,
    },
    Flush(std::sync::mpsc::Sender<bool>),
}

impl Writer {
    pub(super) fn spawn(cx: &App) -> Self {
        #[cfg(test)]
        let scratch = tempfile::tempdir().expect("scratch terminal history");
        #[cfg(test)]
        let directory = scratch.path().to_path_buf();
        #[cfg(not(test))]
        let directory =
            daruda_agent::hooks::status_file::default_dir().expect("profile status directory");
        let (writer, receiver) = Self::channel();
        cx.background_executor()
            .spawn(async move {
                #[cfg(test)]
                let _scratch = scratch;
                consume(directory, receiver).await;
            })
            .detach();
        writer
    }

    fn channel() -> (Self, smol::channel::Receiver<Command>) {
        let (sender, receiver) = smol::channel::unbounded();
        (Self { sender }, receiver)
    }

    pub(super) fn flush(&self, until: std::time::Instant) -> bool {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.sender.try_send(Command::Flush(sender)).is_ok()
            && receiver.recv_timeout(until.saturating_duration_since(std::time::Instant::now()))
                == Ok(true)
    }

    fn record(&self, terminal: TerminalId, event: SessionEvent) {
        if let Err(error) = self.sender.try_send(Command::Record { terminal, event }) {
            LogWriter::log(
                ErrorReport::new("Terminal history writer unavailable")
                    .message(error.to_string())
                    .build(),
            );
        }
    }
}

async fn consume(directory: PathBuf, receiver: smol::channel::Receiver<Command>) {
    let mut failed = false;
    while let Ok(command) = receiver.recv().await {
        match command {
            Command::Record { terminal, event } => {
                if let Err(error) =
                    daruda_agent::hooks::history::record(&directory, &terminal, event)
                {
                    failed = true;
                    LogWriter::log(
                        ErrorReport::new("Terminal history write failed")
                            .from_error(&error)
                            .with_context("terminal", terminal.as_str())
                            .build(),
                    );
                }
            }
            Command::Flush(sender) => {
                // SILENT-OK: the quit deadline may expire before this queued barrier.
                let _ = sender.send(!failed);
            }
        }
    }
}

pub(in crate::runs) fn flush(until: std::time::Instant, cx: &App) -> bool {
    cx.try_global::<super::RunsHost>()
        .and_then(|host| host.cli.as_ref())
        .is_none_or(|cli| cli.history.flush(until))
}

/// Capture event times at dispatch; a single process queue preserves their
/// order without blocking UI callbacks on hook-process locks or disk sync.
pub(crate) fn record(terminal: TerminalId, event: SessionEvent, cx: &App) {
    if let Some(cli) = cx
        .try_global::<super::RunsHost>()
        .and_then(|host| host.cli.as_ref())
    {
        cli.history.record(terminal, event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::terminal_sessions::{TerminalContext, TerminalEndReason};

    #[test]
    fn draining_the_writer_preserves_order_and_context_without_a_task() {
        let directory = tempfile::tempdir().unwrap();
        let terminal = TerminalId::parse("terminal-a").unwrap();
        let context = TerminalContext {
            workspace: daruda_store::project::WorkspaceUuid::new(),
            lane: None,
            account: None,
            cwd: Some(PathBuf::from("/work")),
        };
        let (writer, receiver) = Writer::channel();
        writer.record(
            terminal.clone(),
            SessionEvent::Context {
                context: context.clone(),
            },
        );
        writer.record(
            terminal.clone(),
            SessionEvent::TerminalClosed {
                reason: TerminalEndReason::TerminalExited,
                at: chrono::Utc::now(),
            },
        );
        writer.record(
            terminal.clone(),
            SessionEvent::TerminalClosed {
                reason: TerminalEndReason::PaneClosed,
                at: chrono::Utc::now(),
            },
        );
        drop(writer);
        smol::block_on(consume(directory.path().to_path_buf(), receiver));
        let history = daruda_agent::hooks::history::read(directory.path(), &terminal).unwrap();
        assert_eq!(history.context, Some(context));
        assert!(history.terminal_is_closed());
        assert!(history.executions.is_empty());
    }

    #[test]
    fn a_flush_waits_for_durable_writes_and_reports_a_failed_record() {
        let directory = tempfile::tempdir().unwrap();
        let terminal = TerminalId::parse("terminal-a").unwrap();
        let (writer, receiver) = Writer::channel();
        let path = directory.path().to_path_buf();
        let worker = std::thread::spawn(move || smol::block_on(consume(path, receiver)));
        writer.record(
            terminal.clone(),
            SessionEvent::TerminalClosed {
                reason: TerminalEndReason::PaneClosed,
                at: chrono::Utc::now(),
            },
        );
        assert!(writer.flush(std::time::Instant::now() + std::time::Duration::from_secs(5)));
        assert!(
            daruda_agent::hooks::history::read(directory.path(), &terminal)
                .unwrap()
                .terminal_is_closed()
        );
        drop(writer);
        worker.join().unwrap();

        let blocked = directory.path().join("blocked");
        std::fs::write(&blocked, b"not a directory").unwrap();
        let (writer, receiver) = Writer::channel();
        let worker = std::thread::spawn(move || smol::block_on(consume(blocked, receiver)));
        writer.record(
            terminal,
            SessionEvent::TerminalClosed {
                reason: TerminalEndReason::PaneClosed,
                at: chrono::Utc::now(),
            },
        );
        assert!(!writer.flush(std::time::Instant::now() + std::time::Duration::from_secs(5)));
        drop(writer);
        worker.join().unwrap();
    }
}
