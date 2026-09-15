//! Cancellable preparation subprocesses with bounded, continuously drained output.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use smol::io::{AsyncRead, AsyncReadExt};

use crate::preparation::{PreparationContext, PreparationError, PreparationKind};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(180);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const OUTPUT_LIMIT: usize = 64 * 1024;
const READ_SIZE: usize = 8192;

pub(crate) fn output(
    command: &mut Command,
    context: &PreparationContext<'_>,
) -> Result<String, PreparationError> {
    output_with_timeout(command, context, COMMAND_TIMEOUT)
}

fn output_with_timeout(
    command: &mut Command,
    context: &PreparationContext<'_>,
    timeout: Duration,
) -> Result<String, PreparationError> {
    context.check()?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let replacement = Command::new(command.get_program());
    let mut command = smol::process::Command::from(std::mem::replace(command, replacement));
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut group = ProcessGroup(Some(child.id()));
    smol::block_on(async {
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let collect = async {
            let (stdout, stderr, status) =
                futures::try_join!(capture(stdout, false), capture(stderr, true), async {
                    child.status().await.map_err(PreparationError::from)
                })?;
            Ok::<_, PreparationError>((stdout, stderr, status))
        };
        let interrupt = async {
            let started = Instant::now();
            loop {
                context.check()?;
                if started.elapsed() >= timeout {
                    return Err(PreparationError::new(
                        PreparationKind::Timeout,
                        "package preparation command timed out",
                    ));
                }
                // ALLOW: this GPUI-free worker has no BackgroundExecutor.
                #[allow(clippy::disallowed_methods)]
                smol::Timer::after(POLL_INTERVAL).await;
            }
        };
        let result = smol::future::or(collect, interrupt).await;
        match result {
            Ok((stdout, stderr, status)) => {
                group.0 = None;
                if !status.success() {
                    return Err(exit_error(&stdout, &stderr, status));
                }
                String::from_utf8(stdout).map_err(|error| {
                    PreparationError::new(PreparationKind::Process, error.to_string())
                })
            }
            Err(error) => {
                group.kill();
                // The tree is stopped before its staging directory can be removed.
                let _ = child.kill();
                let _ = child.status().await;
                Err(error)
            }
        }
    })
}

struct ProcessGroup(Option<u32>);

impl ProcessGroup {
    fn kill(&mut self) {
        if let Some(pid) = self.0.take() {
            #[cfg(unix)]
            // SAFETY: this child started its own process group, never the host's.
            unsafe {
                libc::killpg(pid as libc::pid_t, libc::SIGKILL);
            }
            #[cfg(not(unix))]
            let _ = pid;
        }
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.kill();
    }
}

async fn capture(
    mut reader: impl AsyncRead + Unpin,
    tail: bool,
) -> Result<Vec<u8>, PreparationError> {
    let mut bytes = Vec::new();
    let mut buffer = [0; READ_SIZE];
    loop {
        let n = reader.read(&mut buffer).await?;
        if n == 0 {
            return Ok(bytes);
        }
        if bytes.len() + n > OUTPUT_LIMIT {
            if !tail {
                return Err(PreparationError::new(
                    PreparationKind::Process,
                    "package manager output exceeded limit",
                ));
            }
            bytes.drain(..bytes.len() + n - OUTPUT_LIMIT);
        }
        bytes.extend_from_slice(&buffer[..n]);
        // A continuously readable stderr pipe must not starve cancellation.
        smol::future::yield_now().await;
    }
}

fn exit_error(stdout: &[u8], stderr: &[u8], status: std::process::ExitStatus) -> PreparationError {
    let value = serde_json::from_slice::<serde_json::Value>(stdout).ok();
    let code = value.as_ref().and_then(|v| v["error"]["code"].as_str());
    let kind = match code {
        Some(
            "ENOTFOUND" | "ETIMEDOUT" | "ECONNRESET" | "ECONNREFUSED" | "EAI_AGAIN" | "ENETUNREACH"
            | "EHOSTUNREACH" | "ENOTCACHED" | "E502" | "E503" | "E504",
        ) => PreparationKind::Network,
        Some("EINTEGRITY" | "ESIGNATUREVERIFY") => PreparationKind::Integrity,
        Some(
            "EACCES" | "EPERM" | "ENOENT" | "E401" | "E403" | "E404" | "ETARGET" | "EBADENGINE"
            | "EINVALIDTAGNAME",
        ) => PreparationKind::Configuration,
        _ => PreparationKind::Process,
    };
    let detail = value
        .as_ref()
        .and_then(|v| v.get("error"))
        .map(ToString::to_string)
        .unwrap_or_else(|| String::from_utf8_lossy(stderr).trim().to_owned());
    PreparationError::new(
        kind,
        format!("package preparation exited with {status}: {detail}"),
    )
}

#[cfg(test)]
mod tests;
