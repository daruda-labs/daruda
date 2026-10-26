//! Bounded execution and output collection for an authentication status probe.

use std::io::Read;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use daruda_store::observability::{error_report::ErrorReport, log_writer::LogWriter};

use super::super::CliCommand;

const MAX_CAPTURED_OUTPUT: usize = 64 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Keep the streams separate so wrapper diagnostics cannot corrupt JSON stdout.
fn drain(stream: impl Read + Send + 'static) -> std::io::Result<Receiver<Option<Vec<u8>>>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("auth-status-output".into())
        .spawn(move || {
            let mut stream = stream;
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 4096];
            let mut oversized = false;
            let reading = loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break (!oversized).then_some(bytes),
                    Ok(count) => {
                        if !oversized && bytes.len() + count <= MAX_CAPTURED_OUTPUT {
                            bytes.extend_from_slice(&chunk[..count]);
                        } else {
                            // Continue draining: closing a full pipe could abort
                            // the CLI before its valid reading reaches stderr.
                            oversized = true;
                            bytes.clear();
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break None,
                }
            };
            // The caller can time out before an inherited pipe closes.
            let _ = sender.send(reading);
        })?;
    Ok(receiver)
}

pub(super) fn output(command: &CliCommand, timeout: Duration) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut cmd = command.command();
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    // Both streams: the reading is not always on stdout. The codex CLI prints
    // its status line to stderr, so discarding it left that domain reporting
    // nothing at all — indistinguishable from being signed out.
    cmd.stderr(std::process::Stdio::piped());
    // Own group, so a probe that hangs can be torn down whole — the command
    // routinely forks (adapter → the CLI), exactly as the login path does.
    daruda_core::process::lead_own_group(&mut cmd);

    let mut child = cmd.spawn().ok()?;
    let group = match daruda_core::process::Group::try_adopt(child.id()) {
        Ok(group) => group,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            LogWriter::log(
                ErrorReport::new("Cannot own authentication probe process")
                    .from_error(&error)
                    .at(file!(), line!())
                    .build(),
            );
            return None;
        }
    };
    let deadline = Instant::now() + timeout;
    let stdout = match child.stdout.take().and_then(|stream| drain(stream).ok()) {
        Some(receiver) => receiver,
        None => {
            kill_probe_tree(&group, &mut child);
            return None;
        }
    };
    let stderr = match child.stderr.take().and_then(|stream| drain(stream).ok()) {
        Some(receiver) => receiver,
        None => {
            kill_probe_tree(&group, &mut child);
            return None;
        }
    };
    // Keep the leader unreaped until its pipes close. Its PID still owns the
    // group, so a timeout can safely stop descendants retaining those pipes.
    let captured = stdout
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .and_then(|stdout| {
            stderr
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map(|stderr| (stdout, stderr))
        });
    let (stdout, stderr) = match captured {
        Ok(output) => output,
        Err(_) => {
            kill_probe_tree(&group, &mut child);
            return None;
        }
    };
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(_) => {
                // The group requires a known unreaped child; a failed wait
                // cannot establish that ownership, so kill only the child.
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
        if Instant::now() >= deadline {
            kill_probe_tree(&group, &mut child);
            return None;
        }
        std::thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    }
    Some((stdout.unwrap_or_default(), stderr.unwrap_or_default()))
}

/// Stop the owned tree while its root remains unreaped, then reap it.
/// Waiting for pipe EOF first preserves ownership on output timeout.
fn kill_probe_tree(group: &daruda_core::process::Group, child: &mut std::process::Child) {
    // Unreaped — the `wait` below is the reap — so the pid is still this
    // process's to name, which is what the group requires.
    if let Err(error) = group.try_terminate_child(child) {
        LogWriter::log(
            ErrorReport::new("Failed to terminate authentication probe tree")
                .from_error(&error)
                .at(file!(), line!())
                .dedup("account.auth_probe.termination")
                .build(),
        );
    }
}

#[cfg(test)]
mod tests;
