//! Headless subprocess runner for interactive OAuth login (the command
//! produced by `AgentLaunch::login_command` in `daruda_config::agent`, e.g.
//! `npx -y <adapter>@latest --cli auth login --claudeai`).
//!
//! Spawns the login command with piped (non-PTY) stdio and the caller's
//! `inject_env` — which carries whichever config-dir variable the auth
//! domain uses — lets the CLI open the system browser for OAuth, and
//! detects completion by the caller-supplied [`WaitPolicy`] — this module
//! never touches the browser or the OAuth callback itself. Mirrors orca's
//! `claude-accounts/service.ts` `runClaudeCommand` (the managed-login path,
//! around lines 903-1043): piped stdio with stdin kept open (the CLI's
//! OAuth callback server can bind its lifetime to stdin), a capped
//! in-memory output buffer scanned for a denial marker, and a poll-driven
//! timeout that cancels the process.
//!
//! **Departure from orca**: orca cancels via the POSIX process *group*
//! (`process.kill(-child.pid)`, `detached: true` on spawn) so a login
//! command that forks (e.g. an `npx`-wrapped adapter) is torn down along
//! with any child it spawned. This module cancels only the directly
//! spawned child via `std::process::Child::kill()` — the same limitation
//! `daruda_acp`'s agent-session process management already accepts for the
//! identical `npx`-spawn shape (it has no group-kill either; see
//! `daruda_acp::session`), and adding a `libc`/`nix` dependency for a
//! group-wide `SIGTERM` here was intentionally out of scope for this task
//! (see the account-switcher SDD Task 3 brief's escalation note). If an
//! `npx`-wrapped login command ever orphans a grandchild process, the ACP
//! session runner has the identical exposure — worth revisiting for both
//! together, not this module in isolation.
//!
//! GPUI-free and blocking by design — the caller (the app's background
//! executor) is responsible for running [`spawn_login`] +
//! [`LoginProcess::wait`] off the render thread.

use std::io::Read;
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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
    #[error("login command is empty")]
    EmptyCommand,
}

/// Tokenizes `command` with [`shell_words::split`] (quote-aware). Falls
/// back to a plain whitespace split on a parse error (e.g. an unmatched
/// quote in a hand-written command) so a malformed string still yields
/// *some* tokens instead of dropping the command entirely — mirrors
/// `daruda_acp::node::split_env_prefixed_tokens`.
fn tokenize(command: &str) -> Vec<String> {
    shell_words::split(command)
        .unwrap_or_else(|_| command.split_whitespace().map(str::to_string).collect())
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
/// [`DENIAL_MARKERS`] for the marker list and its orca source.
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
}

impl LoginProcessHandle {
    /// Requests termination of the login process. Idempotent — safe to
    /// call more than once (e.g. once from an external cancel request and
    /// again from `wait`'s own timeout path); killing an already-exited
    /// child returns a harmless error that is discarded.
    pub fn cancel(&self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }
}

/// Spawns `command`'s login process with piped (non-PTY) stdio, `strip_env`
/// removed and then `inject_env` applied — in that order, so a stale
/// inherited value can't survive. `inject_env` is the only source for the
/// account's config dir, since which variable carries it is auth-domain
/// specific ([`AccountRecipe::config_dir_env`](super::recipe::AccountRecipe::config_dir_env)).
pub fn spawn_login(
    command: &str,
    inject_env: &[(String, String)],
    strip_env: &[&str],
    timeout: Duration,
) -> Result<LoginProcess, LoginError> {
    let tokens = tokenize(command);
    let (program, args) = tokens.split_first().ok_or(LoginError::EmptyCommand)?;

    let mut cmd = Command::new(program);
    cmd.args(args);
    for name in strip_env {
        cmd.env_remove(name);
    }
    for (name, value) in inject_env {
        cmd.env(name, value);
    }
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    // Isolate the child into its own process group so signals delivered to
    // *this* app's process group (e.g. a terminal Ctrl+C in a dev build)
    // don't also reach the login process. This does not give `cancel` a
    // group-wide kill (see the module doc's "Departure from orca" note) —
    // it only changes what the child receives from outside, not what we
    // send it.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let mut child = cmd.spawn().map_err(|e| LoginError::Spawn(e.to_string()))?;

    let stdout: ChildStdout = child.stdout.take().expect("stdout was piped");
    let stderr: ChildStderr = child.stderr.take().expect("stderr was piped");

    let stdout_buf = Arc::new(Mutex::new(CappedBuffer::default()));
    let stderr_buf = Arc::new(Mutex::new(CappedBuffer::default()));
    spawn_drain(stdout, Arc::clone(&stdout_buf));
    spawn_drain(stderr, Arc::clone(&stderr_buf));

    Ok(LoginProcess {
        child: Arc::new(Mutex::new(child)),
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
fn reap_after_cancel(child: &Mutex<Child>) {
    for _ in 0..REAP_ATTEMPTS {
        match child.lock() {
            Ok(mut child) => match child.try_wait() {
                Ok(Some(_)) | Err(_) => return,
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
    /// [`POLL_INTERVAL`]. Consumes `self` so the piped `stdin` — never
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
                Ok(Some(status)) => break PollResult::Exited(status),
                Ok(None) => {
                    let now = Instant::now();
                    match grace_deadline {
                        Some(expiry) if now >= expiry => {
                            self.cancel();
                            reap_after_cancel(&self.child);
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
                                reap_after_cancel(&self.child);
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
mod tests {
    use super::super::recipe::recipe_for;
    use super::*;
    use daruda_store::accounts::AccountRecipeId;

    /// Test-scoped grace/timeout values — short enough to keep the suite
    /// fast, long enough to survive a loaded CI machine's scheduling jitter.
    const TEST_GRACE: Duration = Duration::from_millis(300);
    const TEST_TIMEOUT: Duration = Duration::from_millis(600);
    /// Timeout for the cases that must run a child to completion rather
    /// than time out.
    const TEST_LONG_TIMEOUT: Duration = Duration::from_secs(10);

    fn spawn_for_test(command: &str, timeout: Duration) -> LoginProcess {
        spawn_login(command, &[], &[], timeout).expect("spawn the test child")
    }

    /// Whether `pid` is still a live (non-reaped) process. `ps -p` exits
    /// non-zero once the process is gone, and `wait`'s cancel path reaps the
    /// child, so a killed child leaves no zombie for `ps` to find.
    fn process_is_running(pid: u32) -> bool {
        Command::new("/bin/ps")
            .arg("-p")
            .arg(pid.to_string())
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn on_exit_reports_success_for_a_zero_exit() {
        let process = spawn_for_test("/usr/bin/true", TEST_LONG_TIMEOUT);
        assert_eq!(process.wait(WaitPolicy::OnExit), LoginOutcome::Success);
    }

    #[test]
    fn on_exit_reports_failed_for_a_nonzero_exit() {
        let process = spawn_for_test("/usr/bin/false", TEST_LONG_TIMEOUT);
        assert!(matches!(
            process.wait(WaitPolicy::OnExit),
            LoginOutcome::Failed(_)
        ));
    }

    #[test]
    fn on_exit_times_out_when_the_process_never_exits() {
        let process = spawn_for_test("/bin/sleep 30", TEST_TIMEOUT);
        let pid = process.child_pid();
        assert_eq!(process.wait(WaitPolicy::OnExit), LoginOutcome::TimedOut);
        assert!(!process_is_running(pid), "the timed-out child must be gone");
    }

    #[test]
    fn on_credentials_reports_success_when_the_process_exits_within_the_grace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let auth = dir.path().join("auth.json");
        let writer = auth.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            std::fs::write(&writer, b"{}").expect("write credentials");
        });

        let process = spawn_for_test("/bin/sh -c 'sleep 0.4'", TEST_LONG_TIMEOUT);
        let landed = || auth.exists();
        assert_eq!(
            process.wait(WaitPolicy::OnCredentials {
                grace: Duration::from_secs(5),
                credentials_landed: &landed,
            }),
            LoginOutcome::Success
        );
    }

    #[test]
    fn on_credentials_cancels_and_succeeds_when_the_process_never_exits() {
        let dir = tempfile::tempdir().expect("tempdir");
        let auth = dir.path().join("auth.json");
        let writer = auth.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            std::fs::write(&writer, b"{}").expect("write credentials");
        });

        let process = spawn_for_test("/bin/sleep 30", TEST_LONG_TIMEOUT);
        let pid = process.child_pid();
        let landed = || auth.exists();
        let started = Instant::now();
        assert_eq!(
            process.wait(WaitPolicy::OnCredentials {
                grace: TEST_GRACE,
                credentials_landed: &landed,
            }),
            LoginOutcome::Success
        );
        assert!(
            started.elapsed() < TEST_LONG_TIMEOUT,
            "credentials landing must end the wait long before the overall timeout"
        );
        assert!(
            !process_is_running(pid),
            "a child that outlived the grace window must be cancelled"
        );
    }

    #[test]
    fn on_credentials_times_out_when_credentials_never_land() {
        let process = spawn_for_test("/bin/sleep 30", TEST_TIMEOUT);
        let pid = process.child_pid();
        let landed = || false;
        assert_eq!(
            process.wait(WaitPolicy::OnCredentials {
                grace: TEST_GRACE,
                credentials_landed: &landed,
            }),
            LoginOutcome::TimedOut
        );
        assert!(!process_is_running(pid), "the timed-out child must be gone");
    }

    #[test]
    fn on_credentials_lets_an_earlier_exit_decide_the_outcome() {
        let process = spawn_for_test("/usr/bin/false", TEST_LONG_TIMEOUT);
        let landed = || false;
        assert!(matches!(
            process.wait(WaitPolicy::OnCredentials {
                grace: TEST_GRACE,
                credentials_landed: &landed,
            }),
            LoginOutcome::Failed(_)
        ));
    }

    #[test]
    fn on_credentials_lets_an_earlier_denial_decide_the_outcome() {
        let process = spawn_for_test("/bin/sh -c 'echo access_denied 1>&2; exit 1'", TEST_TIMEOUT);
        let landed = || false;
        assert_eq!(
            process.wait(WaitPolicy::OnCredentials {
                grace: TEST_GRACE,
                credentials_landed: &landed,
            }),
            LoginOutcome::Denied
        );
    }

    #[test]
    fn detects_oauth_denied_marker() {
        assert!(is_oauth_denied("... error: access_denied ..."));
        assert!(is_oauth_denied("Authorization was denied by the user."));
        assert!(!is_oauth_denied("Login successful"));
    }

    #[test]
    fn denial_check_is_case_insensitive() {
        assert!(is_oauth_denied("ACCESS_DENIED"));
        assert!(is_oauth_denied("Sign-In Denied"));
    }

    #[test]
    fn tokenizes_command_with_shell_words() {
        let toks = tokenize("npx -y pkg --cli auth login --claudeai");
        assert_eq!(
            toks,
            vec!["npx", "-y", "pkg", "--cli", "auth", "login", "--claudeai"]
        );
    }

    #[test]
    fn tokenize_handles_quoted_path_with_space() {
        let toks = tokenize("claude --config '/tmp/my dir'");
        assert_eq!(toks, vec!["claude", "--config", "/tmp/my dir"]);
    }

    #[test]
    fn tokenize_falls_back_to_whitespace_on_unmatched_quote() {
        let toks = tokenize("claude --config 'unterminated");
        assert_eq!(toks, vec!["claude", "--config", "'unterminated"]);
    }

    #[test]
    fn spawn_login_rejects_empty_command() {
        let err = spawn_login("   ", &[], &[], Duration::from_secs(1))
            .expect_err("empty command must fail to spawn");
        assert!(matches!(err, LoginError::EmptyCommand));
    }

    /// Shell probe exiting 0 only when `want`'s variable holds `expected`
    /// and `avoid`'s does not — the cross-domain leak being pinned is a
    /// second, wrong-domain variable carrying the same config dir.
    fn env_probe(want: &str, avoid: &str, expected: &str) -> String {
        format!(r#"/bin/sh -c 'test "${want}" = "{expected}" && test "${avoid}" != "{expected}"'"#)
    }

    /// `inject_env` in the shape the production caller builds it: one pair
    /// naming the recipe's own config-dir variable.
    fn config_dir_inject(recipe: AccountRecipeId, dir: &str) -> Vec<(String, String)> {
        vec![(
            recipe_for(recipe).config_dir_env().to_string(),
            dir.to_string(),
        )]
    }

    fn probe_outcome(probe: &str, inject: &[(String, String)]) -> LoginOutcome {
        spawn_login(probe, inject, &[], TEST_LONG_TIMEOUT)
            .expect("spawn the env probe")
            .wait(WaitPolicy::OnExit)
    }

    #[test]
    fn a_codex_login_child_gets_codex_home_and_no_claude_config_dir() {
        let dir = "/tmp/daruda-login-env-codex";
        assert_eq!(
            probe_outcome(
                &env_probe("CODEX_HOME", "CLAUDE_CONFIG_DIR", dir),
                &config_dir_inject(AccountRecipeId::Codex, dir)
            ),
            LoginOutcome::Success
        );
    }

    #[test]
    fn a_claude_login_child_gets_claude_config_dir_and_no_codex_home() {
        let dir = "/tmp/daruda-login-env-claude";
        assert_eq!(
            probe_outcome(
                &env_probe("CLAUDE_CONFIG_DIR", "CODEX_HOME", dir),
                &config_dir_inject(AccountRecipeId::Claude, dir)
            ),
            LoginOutcome::Success
        );
    }

    #[test]
    fn the_env_probe_fails_when_nothing_is_injected() {
        // Negative control: the two assertions above must be proving the
        // injection rather than passing on a probe that always exits 0.
        let dir = "/tmp/daruda-login-env-codex";
        assert!(matches!(
            probe_outcome(&env_probe("CODEX_HOME", "CLAUDE_CONFIG_DIR", dir), &[]),
            LoginOutcome::Failed(_)
        ));
    }

    /// Real spawn smoke test — not run in CI (spawns an actual process).
    /// Documents the happy path: a trivial command that exits 0 with clean
    /// output must resolve to `Success` well within the timeout.
    #[test]
    #[ignore = "spawns a real subprocess; run manually with --ignored"]
    fn spawn_and_wait_success_smoke() {
        let outcome = spawn_login("/bin/echo hi", &[], &[], Duration::from_secs(5))
            .expect("spawn should succeed")
            .wait(WaitPolicy::OnExit);
        assert_eq!(outcome, LoginOutcome::Success);
    }

    /// Real spawn smoke test for the timeout + cancel path — not run in
    /// CI. `sleep 5` outlives the 1s timeout, so `wait` must cancel it and
    /// return `TimedOut` rather than blocking for the full 5s. This also
    /// exercises the timeout-path reap (Finding 2): if `reap_after_cancel`
    /// regressed into not reaping, this test would still pass (it can't
    /// observe zombie state through `std` alone), but the reap loop runs
    /// unconditionally on every timeout, so any panic/hang in it would
    /// surface here.
    #[test]
    #[ignore = "spawns a real subprocess; run manually with --ignored"]
    fn spawn_and_wait_times_out_and_cancels_smoke() {
        let outcome = spawn_login("/bin/sleep 5", &[], &[], Duration::from_secs(1))
            .expect("spawn should succeed")
            .wait(WaitPolicy::OnExit);
        assert_eq!(outcome, LoginOutcome::TimedOut);
    }

    /// Compile+shape test for Finding 1: `LoginProcessHandle` must be
    /// cloneable so `Workspace` can stash one clone (e.g.
    /// `PendingLogin::InProgress { handle }`) while another clone (or the
    /// original) is used elsewhere. No process is spawned here.
    #[test]
    fn login_process_handle_is_clone() {
        fn assert_clone<T: Clone>() {}
        assert_clone::<LoginProcessHandle>();
    }

    /// Real spawn smoke test proving a cloned [`LoginProcessHandle`]
    /// cancels the *same* child as the [`LoginProcess`] it was obtained
    /// from, independently of `self` — the shape Finding 1 exists for:
    /// grab `handle()` before `wait(self)` moves the process into a
    /// background executor, then cancel from elsewhere while `wait` is
    /// still blocked. `sleep 5` with a generous 10s timeout means only
    /// the handle's cancel (fired after 200ms from another thread) can
    /// end this early.
    #[test]
    #[ignore = "spawns a real subprocess; run manually with --ignored"]
    fn handle_clone_cancels_independently_of_self_smoke() {
        let process = spawn_login("/bin/sleep 5", &[], &[], Duration::from_secs(10))
            .expect("spawn should succeed");

        let handle = process.handle();
        let cloned = handle.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            cloned.cancel();
        });

        let start = Instant::now();
        let outcome = process.wait(WaitPolicy::OnExit);
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "cancel via the cloned handle should end wait() well before the 10s timeout"
        );
        assert!(
            matches!(outcome, LoginOutcome::Failed(_)),
            "expected a killed-process Failed outcome, got {outcome:?}"
        );
    }
}
