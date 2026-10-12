//! Headless subprocess runner for interactive OAuth login (a [`CliCommand`]
//! the app resolves from the adapter's prepared installation, e.g.
//! `node <adapter entry> --cli auth login --claudeai`).
//!
//! Spawns the login command with piped (non-PTY) stdio — its environment
//! already carries whichever config-dir variable the auth domain uses —
//! lets the CLI open the system browser for OAuth, and
//! detects completion by the caller-supplied [`WaitPolicy`] — this module
//! never touches the browser or the OAuth callback itself. Mirrors orca's
//! `claude-accounts/service.ts` `runClaudeCommand` (the managed-login path,
//! around lines 903-1043): piped stdio with stdin kept open (the CLI's
//! OAuth callback server can bind its lifetime to stdin), a capped
//! in-memory output buffer scanned for a denial marker, and a poll-driven
//! timeout that cancels the process.
//!
//! Cancellation kills the process **group**, matching orca: a login command
//! that forks (the adapter forking the native CLI it wraps) leaves the forked
//! process holding the OAuth callback port, and a cancel that spared it would
//! block the very next attempt. `spawn_login` puts the child in its own group
//! so the signal reaches the tree and nothing else.
//!
//! The ACP session runner (`daruda_acp::session`) still tears down only its
//! direct child. The exposure is the same shape but not the same weight —
//! there a spawn ends when the session does, while here cancel is the ordinary
//! way a user abandons a sign-in, so a survivor is met on the next click.
//!
//! GPUI-free and blocking by design — the caller (the app's background
//! executor) is responsible for running [`spawn_login`] +
//! [`LoginProcess::wait`] off the render thread.

use std::io::Read;
use std::process::{Child, ChildStderr, ChildStdout, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::cli_command::CliCommand;

/// Output captured from the login process is capped so a chatty/misbehaving
/// CLI can't grow this buffer without bound; 64 KiB comfortably holds any
/// realistic OAuth error message.
const MAX_CAPTURED_OUTPUT: usize = 64 * 1024;

/// How often [`LoginProcess::wait`] polls `Child::try_wait` while the login
/// process is still running. Cheap enough to keep the timeout responsive
/// without busy-looping.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Extra time [`LoginProcess::wait`] allows the stdout/stderr drain threads
/// to catch up after the child reports its exit status, before snapshotting
/// captured output. The drain threads are never `join`ed (a login command
/// that forks — see the module doc's "Departure from orca" note — can leave
/// a grandchild holding the pipe's write end open, which would make a
/// `join` block forever waiting for EOF that never comes); this bounded
/// grace period is the deliberate std-only trade-off instead.
const EXIT_DRAIN_GRACE: Duration = Duration::from_millis(50);

/// What proves a domain's headless login finished. A property of the auth
/// domain ([`AccountRecipe::login_completion`](super::recipe::AccountRecipe::login_completion)),
/// carried as data so recipes stay `&'static`; pair it with a credentials
/// probe via [`Self::with_probe`] to get the runtime [`WaitPolicy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginCompletion {
    /// Process exit is the completion signal.
    OnExit,
    /// Credentials landing in the config dir is the signal; the process then
    /// gets `grace` to exit on its own before being cancelled. Exit is polled
    /// first on every tick, so a CLI that does exit never spends the `grace`
    /// — this is a defensive fallback, not the expected path.
    OnCredentials { grace: Duration },
}

impl LoginCompletion {
    /// Bind `credentials_landed` in where the policy needs it, so a probe
    /// can't be paired with a policy that would never call it.
    pub fn with_probe<'a>(self, credentials_landed: &'a dyn Fn() -> bool) -> WaitPolicy<'a> {
        match self {
            Self::OnExit => WaitPolicy::OnExit,
            Self::OnCredentials { grace } => WaitPolicy::OnCredentials {
                grace,
                credentials_landed,
            },
        }
    }
}

/// [`LoginCompletion`] with the credentials probe bound in — what
/// [`LoginProcess::wait`] actually runs against. The probe is injected
/// rather than resolved from a recipe here so this module stays testable
/// and domain-agnostic.
#[derive(Clone, Copy)]
pub enum WaitPolicy<'a> {
    OnExit,
    OnCredentials {
        grace: Duration,
        /// Answers "have credentials landed yet?". Callers pass
        /// `AccountRecipe::has_credentials`, which *parses* the credentials
        /// — a half-written file reads as "not yet", the safe direction,
        /// unlike orca's raw "new bytes in `auth.json`" watch.
        credentials_landed: &'a dyn Fn() -> bool,
    },
}

/// Result of a completed (or abandoned) headless login attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginOutcome {
    /// The login process exited 0 and no denial marker was seen in its
    /// captured output — the CLI persisted credentials into the account's
    /// injected config dir (or into the credential store scoped to it).
    Success,
    /// The login process's captured output matched [`is_oauth_denied`]: the
    /// user declined the OAuth consent screen.
    Denied,
    /// The login process did not exit within the caller-supplied timeout;
    /// it has been cancelled (see [`LoginProcess::cancel`]).
    TimedOut,
    /// The login process exited non-zero (and wasn't a denial), or the
    /// wait loop itself failed. Carries captured output / an error string
    /// for diagnostics.
    Failed(String),
}

/// Failure to even spawn the login process.
#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("failed to spawn login command: {0}")]
    Spawn(String),
}

/// Substring markers for an OAuth consent denial, checked (case-insensitive)
/// against the captured stdout+stderr. Approximates orca's
/// `CLAUDE_AUTH_DENIED_PATTERN` regex —
/// `/\baccess_denied\b|authorization (?:request )?(?:was )?denied|sign-?in
/// (?:was )?denied|login (?:was )?denied/i` — as a fixed marker list
/// instead of a regex, per this task's dependency guidance (no `regex` in
/// `daruda_agent`).
const DENIAL_MARKERS: &[&str] = &[
    "access_denied",
    "authorization denied",
    "authorization request denied",
    "authorization was denied",
    "sign-in denied",
    "sign in denied",
    "sign-in was denied",
    "sign in was denied",
    "signin denied",
    "login denied",
    "login was denied",
];

/// Pure OAuth-denial check over already-captured process output. See
/// `DENIAL_MARKERS` for the marker list and its orca source.
pub fn is_oauth_denied(output: &str) -> bool {
    let lower = output.to_lowercase();
    DENIAL_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// A capped byte buffer that keeps the *most recent* [`MAX_CAPTURED_OUTPUT`]
/// bytes written to it — mirrors orca's
/// `output.slice(-MAX_COMMAND_OUTPUT_CHARS)`, since a denial marker (or a
/// useful failure message) is most likely near the end of the CLI's
/// output, not the start.
#[derive(Debug, Default)]
struct CappedBuffer(Vec<u8>);

impl CappedBuffer {
    fn push(&mut self, data: &[u8]) {
        self.0.extend_from_slice(data);
        if self.0.len() > MAX_CAPTURED_OUTPUT {
            let overflow = self.0.len() - MAX_CAPTURED_OUTPUT;
            self.0.drain(..overflow);
        }
    }

    fn as_string(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }
}

/// Spawns a reader thread that drains `stream` into `buf` until EOF or a
/// read error. Deliberately not returned as a `JoinHandle` the caller waits
/// on — see [`EXIT_DRAIN_GRACE`] for why a `join` here would be unsafe to
/// rely on.
fn spawn_drain<R>(mut stream: R, buf: Arc<Mutex<CappedBuffer>>)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if let Ok(mut buf) = buf.lock() {
                        buf.push(&chunk[..n]);
                    } else {
                        break;
                    }
                }
            }
        }
    });
}

/// A spawned headless login process. The child is kept behind an
/// `Arc<Mutex<Child>>` so a [`LoginProcessHandle`] cloned out via
/// [`LoginProcess::handle`] can outlive the move of `self` into
/// `wait`'s caller (typically the app's background executor): the
/// handle's [`LoginProcessHandle::cancel`] and `wait`'s own
/// timeout-cancel path both kill the *same* child through the shared
/// lock, so an external cancel request racing the timeout is safe and
/// idempotent.
///
/// `child.stdin` is deliberately never `.take()`n: it stays open on the
/// `Child` for this struct's whole lifetime, matching orca's "keep stdin
/// open" requirement (the CLI's OAuth callback server can bind its
/// lifetime to stdin) until `wait` returns and `self` (and the last
/// `Arc` reference to the `Child` inside it) is dropped.
#[derive(Debug)]
pub struct LoginProcess {
    child: Arc<Mutex<Child>>,
    /// The child's tear-down handle. Held rather than derived from the pid:
    /// Windows needs a job object that exists before the descendants do.
    group: Arc<daruda_core::process::Group>,
    /// Set once the child has been reaped, so a concurrent cancel knows its
    /// pid is no longer this process's to name.
    reaped: Arc<AtomicBool>,
    stdout_buf: Arc<Mutex<CappedBuffer>>,
    stderr_buf: Arc<Mutex<CappedBuffer>>,
    timeout: Duration,
}

/// A cloneable, cancel-capable handle to a [`LoginProcess`]'s child,
/// obtained via [`LoginProcess::handle`] *before* `wait(self)` moves the
/// `LoginProcess` into a background executor. Lets a caller (e.g.
/// `Workspace`'s `PendingLogin::InProgress { handle }`) request
/// cancellation while `wait` is still blocked elsewhere.
#[derive(Debug, Clone)]
pub struct LoginProcessHandle {
    child: Arc<Mutex<Child>>,
    group: Arc<daruda_core::process::Group>,
    reaped: Arc<AtomicBool>,
}

impl LoginProcessHandle {
    /// Requests termination of the login process. Idempotent — safe to
    /// call more than once (e.g. once from an external cancel request and
    /// again from `wait`'s own timeout path); killing an already-exited
    /// child returns a harmless error that is discarded.
    pub fn cancel(&self) {
        if let Ok(mut child) = self.child.lock() {
            // The whole login tree, not just the process that was spawned.
            // The adapter forks the native CLI that owns the
            // OAuth callback server, and that server keeps its port bound; kill
            // only the direct child and the next sign-in cannot complete, which
            // is exactly the attempt a cancel exists to make room for.
            // `spawn_login` already made the child a group leader, so its pid
            // doubles as the group id.
            //
            // Gated on *reaped*, not on "still running": the direct child may
            // well have exited already — a wrapper may fork and go — and that is
            // precisely when the descendants need the signal. A zombie keeps
            // its pid until we reap it, so the group id stays ours to name.
            if !self.reaped.load(Ordering::Acquire)
                && let Err(error) = self.group.try_kill_tree()
            {
                daruda_store::observability::log_writer::LogWriter::log(
                    daruda_store::observability::error_report::ErrorReport::new(
                        "Failed to terminate login process tree",
                    )
                    .from_error(&error)
                    .at(file!(), line!())
                    .dedup("account.login.termination")
                    .build(),
                );
            }
            let _ = child.kill();
        }
    }
}

/// Spawns `command`'s login process with piped (non-PTY) stdio. The account's
/// config dir and strip list arrive inside `command`, folded in where the
/// adapter was prepared.
pub fn spawn_login(command: &CliCommand, timeout: Duration) -> Result<LoginProcess, LoginError> {
    let mut cmd = command.command();
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    // Isolate the child into its own process group, which buys two things:
    // signals aimed at *this* app's group (a terminal Ctrl+C in a dev build)
    // stop at the boundary, and the child's pid doubles as a group id that
    // `LoginProcessHandle::cancel` can signal to reach the descendants a
    // wrapped login forks.
    daruda_core::process::lead_own_group(&mut cmd);

    let mut child = cmd.spawn().map_err(|e| LoginError::Spawn(e.to_string()))?;
    // Immediately: on Windows the child is spawned suspended and this is what
    // lets it run, so nothing that could fail may sit in between.
    let group = match daruda_core::process::Group::try_adopt(child.id()) {
        Ok(group) => Arc::new(group),
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(LoginError::Spawn(error.to_string()));
        }
    };

    let stdout: ChildStdout = child.stdout.take().expect("stdout was piped");
    let stderr: ChildStderr = child.stderr.take().expect("stderr was piped");

    let stdout_buf = Arc::new(Mutex::new(CappedBuffer::default()));
    let stderr_buf = Arc::new(Mutex::new(CappedBuffer::default()));
    spawn_drain(stdout, Arc::clone(&stdout_buf));
    spawn_drain(stderr, Arc::clone(&stderr_buf));

    Ok(LoginProcess {
        group,
        child: Arc::new(Mutex::new(child)),
        reaped: Arc::new(AtomicBool::new(false)),
        stdout_buf,
        stderr_buf,
        timeout,
    })
}

/// Outcome of the poll loop in [`LoginProcess::wait`], before captured
/// output is folded in to produce the final [`LoginOutcome`].
enum PollResult {
    Exited(ExitStatus),
    /// [`WaitPolicy::OnCredentials`] only: credentials landed and the
    /// process outlived its grace window, so it was cancelled.
    CredentialsLanded,
    TimedOut,
    PollError(String),
}

/// How many bounded `try_wait` reap attempts [`reap_after_cancel`] makes
/// after sending `kill()`. A `SIGKILL`'d child's exit status is available
/// to the parent almost immediately, so this is generous slack, not a
/// real wait.
const REAP_ATTEMPTS: u32 = 20;

/// Interval between reap attempts in [`reap_after_cancel`]. Together with
/// [`REAP_ATTEMPTS`] this caps the reap wait at ~100ms.
const REAP_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Bounded `try_wait` loop run after [`LoginProcessHandle::cancel`] on the
/// timeout path, so the killed child is reaped instead of leaking a
/// zombie for the app's lifetime: `std::process::Child` has no reaping
/// `Drop`, so a `Child` that's merely dropped without ever being
/// `wait`ed leaves its process as a zombie until *this app* exits.
fn reap_after_cancel(child: &Mutex<Child>, reaped: &AtomicBool) {
    for _ in 0..REAP_ATTEMPTS {
        match child.lock() {
            Ok(mut child) => match child.try_wait() {
                Ok(Some(_)) => {
                    // Published so a later cancel does not signal a pid that is
                    // no longer ours — it may have been handed to someone else.
                    reaped.store(true, Ordering::Release);
                    return;
                }
                Err(_) => return,
                Ok(None) => {}
            },
            Err(_) => return,
        }
        thread::sleep(REAP_POLL_INTERVAL);
    }
}

impl LoginProcess {
    /// Requests termination of the login process. Idempotent — safe to
    /// call more than once (e.g. once from an external cancel request and
    /// again from `wait`'s own timeout path); killing an already-exited
    /// child returns a harmless error that is discarded. Convenience
    /// delegate to a [`LoginProcessHandle`] over the same shared child —
    /// prefer [`LoginProcess::handle`] when a caller needs to cancel from
    /// somewhere other than `self` (e.g. after `self` has moved into
    /// `wait` on a background executor).
    pub fn cancel(&self) {
        LoginProcessHandle {
            child: Arc::clone(&self.child),
            group: Arc::clone(&self.group),
            reaped: Arc::clone(&self.reaped),
        }
        .cancel();
    }

    /// Returns a cloneable, cancel-capable handle sharing this process's
    /// child. Must be called before [`LoginProcess::wait`] (which
    /// consumes `self`) — typically right after [`spawn_login`], so the
    /// caller can stash the handle (e.g. in `Workspace`'s
    /// `PendingLogin::InProgress { handle }`) and then move the
    /// `LoginProcess` itself into a background executor to `wait()`.
    pub fn handle(&self) -> LoginProcessHandle {
        LoginProcessHandle {
            child: Arc::clone(&self.child),
            group: Arc::clone(&self.group),
            reaped: Arc::clone(&self.reaped),
        }
    }

    /// The child's OS pid, so a test can assert it really was cancelled
    /// after [`Self::wait`] has consumed `self`.
    #[cfg(test)]
    fn child_pid(&self) -> u32 {
        self.child.lock().expect("child mutex").id()
    }

    /// Blocks until `policy`'s completion signal fires or `timeout` elapses
    /// (then: cancel + [`LoginOutcome::TimedOut`]), polling at
    /// `POLL_INTERVAL`. Consumes `self` so the piped `stdin` — never
    /// taken/closed — stays open for the process's entire run.
    ///
    /// Once credentials have landed the overall `timeout` no longer applies:
    /// what the login had to produce is already on disk, so only the grace
    /// window for a self-exit is left to run.
    pub fn wait(self, policy: WaitPolicy<'_>) -> LoginOutcome {
        let deadline = Instant::now() + self.timeout;
        let mut grace_deadline: Option<Instant> = None;
        let result = loop {
            let polled = match self.child.lock() {
                Ok(mut child) => child.try_wait(),
                Err(_) => break PollResult::PollError("login process mutex poisoned".to_string()),
            };
            match polled {
                Ok(Some(status)) => {
                    self.reaped.store(true, Ordering::Release);
                    break PollResult::Exited(status);
                }
                Ok(None) => {
                    let now = Instant::now();
                    match grace_deadline {
                        Some(expiry) if now >= expiry => {
                            self.cancel();
                            reap_after_cancel(&self.child, &self.reaped);
                            break PollResult::CredentialsLanded;
                        }
                        Some(_) => {}
                        None => {
                            if let WaitPolicy::OnCredentials {
                                grace,
                                credentials_landed,
                            } = policy
                                && credentials_landed()
                            {
                                grace_deadline = Some(now + grace);
                            } else if now >= deadline {
                                self.cancel();
                                reap_after_cancel(&self.child, &self.reaped);
                                break PollResult::TimedOut;
                            }
                        }
                    }
                    thread::sleep(POLL_INTERVAL);
                }
                Err(e) => break PollResult::PollError(e.to_string()),
            }
        };

        // Give the drain threads a short, bounded window to catch up with
        // whatever the process wrote right before exiting. Not a `join`:
        // see `EXIT_DRAIN_GRACE`'s doc for why an unbounded wait here is
        // unsafe with a login command that can fork.
        thread::sleep(EXIT_DRAIN_GRACE);
        let captured = {
            let stdout = self
                .stdout_buf
                .lock()
                .map(|b| b.as_string())
                .unwrap_or_default();
            let stderr = self
                .stderr_buf
                .lock()
                .map(|b| b.as_string())
                .unwrap_or_default();
            format!("{stdout}{stderr}")
        };

        match result {
            PollResult::TimedOut => LoginOutcome::TimedOut,
            // The credentials are on disk, which is the thing that matters;
            // the forced exit of a CLI that just wouldn't quit is not a
            // failure to report.
            PollResult::CredentialsLanded => LoginOutcome::Success,
            PollResult::PollError(e) => LoginOutcome::Failed(e),
            PollResult::Exited(status) => {
                if is_oauth_denied(&captured) {
                    LoginOutcome::Denied
                } else if status.success() {
                    LoginOutcome::Success
                } else {
                    let trimmed = captured.trim();
                    let detail = if trimmed.is_empty() {
                        format!("login process exited with {status}")
                    } else {
                        trimmed.to_string()
                    };
                    LoginOutcome::Failed(detail)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
