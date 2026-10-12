use super::super::cli_command::from_line;
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
    spawn_login(&from_line(command), timeout).expect("spawn the test child")
}

fn spawn_fixture(args: &[&str], timeout: Duration) -> LoginProcess {
    spawn_for_test(&test_process::command_line(args), timeout)
}

/// The cancel path reaps the child before this liveness assertion.
fn process_is_running(pid: u32) -> bool {
    daruda_core::process::is_alive(pid)
}

#[test]
fn on_exit_reports_success_for_a_zero_exit() {
    let process = spawn_fixture(&["--exit", "0"], TEST_LONG_TIMEOUT);
    assert_eq!(process.wait(WaitPolicy::OnExit), LoginOutcome::Success);
}

#[test]
fn on_exit_reports_failed_for_a_nonzero_exit() {
    let process = spawn_fixture(&["--exit", "1"], TEST_LONG_TIMEOUT);
    assert!(matches!(
        process.wait(WaitPolicy::OnExit),
        LoginOutcome::Failed(_)
    ));
}

#[test]
fn on_exit_times_out_when_the_process_never_exits() {
    let process = spawn_fixture(&["--sleep-ms", "30000"], TEST_TIMEOUT);
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

    let process = spawn_fixture(&["--sleep-ms", "400"], TEST_LONG_TIMEOUT);
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

    let process = spawn_fixture(&["--sleep-ms", "30000"], TEST_LONG_TIMEOUT);
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
    let process = spawn_fixture(&["--sleep-ms", "30000"], TEST_TIMEOUT);
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
    let process = spawn_fixture(&["--exit", "1"], TEST_LONG_TIMEOUT);
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
    let process = spawn_fixture(&["--stderr", "access_denied", "--exit", "1"], TEST_TIMEOUT);
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

/// Poll until `pid` is gone, or give up. `kill -0` asks only whether the
/// process exists, so an orphan reparented to init still answers.
#[cfg(unix)]
fn gone_within(pid: &str, budget: Duration) -> bool {
    let deadline = std::time::Instant::now() + budget;
    while std::time::Instant::now() < deadline {
        let alive = daruda_core::process::command("kill")
            .args(["-0", pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !alive {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// The case the group signal exists for, and the one the first attempt at
/// it missed: the process daruda spawned **exits first**, leaving the CLI
/// it forked holding the OAuth callback port. A wrapper is exactly that shape.
///
/// Gating the group signal on "not yet reaped" skipped precisely this —
/// the direct child is gone, so a cancel touched nothing and the next
/// sign-in could not bind the port.
#[cfg(unix)]
#[test]
fn cancelling_kills_a_grandchild_whose_parent_already_exited() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pid_file = dir.path().join("grandchild.pid");
    // No `wait`: the shell exits as soon as it has forked and reported.
    let command = format!(
        "/bin/sh -c 'sh -c \"while :; do sleep 0.05; done\" & echo $! > {}'",
        pid_file.display()
    );
    let login = spawn_login(&from_line(&command), Duration::from_secs(20)).expect("spawn");
    let handle = login.handle();

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let pid = loop {
        if let Ok(raw) = std::fs::read_to_string(&pid_file) {
            let trimmed = raw.trim().to_string();
            if !trimmed.is_empty() {
                break trimmed;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the forked grandchild never reported its pid"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    // Let the direct child exit before cancelling, which is the whole point.
    std::thread::sleep(Duration::from_millis(300));

    handle.cancel();
    assert!(
        gone_within(&pid, Duration::from_secs(5)),
        "grandchild {pid} survived a cancel its parent had already left"
    );
}

/// A cancelled sign-in has to take the whole login tree with it.
///
/// The login runs through the adapter, which forks the CLI that actually holds
/// the OAuth callback port; killing only the process daruda spawned leaves
/// that port bound, so the *next* attempt cannot complete — and cancel is
/// the ordinary recovery path when a user abandons a sign-in.
/// `spawn_login` already makes the child a process-group leader; this
/// covers the other half.
#[cfg(unix)]
#[test]
fn cancelling_kills_a_grandchild_the_direct_child_forked() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pid_file = dir.path().join("grandchild.pid");
    let command = format!(
        "/bin/sh -c 'sh -c \"while :; do sleep 0.05; done\" & echo $! > {} ; wait'",
        pid_file.display()
    );
    let login = spawn_login(&from_line(&command), Duration::from_secs(20)).expect("spawn");
    let handle = login.handle();

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let pid = loop {
        if let Ok(raw) = std::fs::read_to_string(&pid_file) {
            let trimmed = raw.trim().to_string();
            if !trimmed.is_empty() {
                break trimmed;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the forked grandchild never reported its pid"
        );
        std::thread::sleep(Duration::from_millis(25));
    };

    handle.cancel();
    assert!(
        gone_within(&pid, Duration::from_secs(5)),
        "grandchild {pid} survived the cancel — the login tree outlives it"
    );
}

/// Native probe exiting 0 only when `want`'s variable holds `expected`
/// and `avoid`'s does not — the cross-domain leak being pinned is a
/// second, wrong-domain variable carrying the same config dir.
fn env_probe(want: &str, avoid: &str, expected: &str) -> String {
    test_process::command_line(&[
        "--require-env",
        want,
        expected,
        "--reject-env-value",
        avoid,
        expected,
    ])
}

/// The account environment in the shape preparation folds it in: one pair
/// naming the recipe's own config-dir variable.
fn config_dir_inject(recipe: AccountRecipeId, dir: &str) -> Vec<(String, String)> {
    vec![(
        recipe_for(recipe).config_dir_env().to_string(),
        dir.to_string(),
    )]
}

fn probe_outcome(probe: &str, env: &[(String, String)]) -> LoginOutcome {
    let command = CliCommand {
        env: env.to_vec(),
        ..from_line(probe)
    };
    spawn_login(&command, TEST_LONG_TIMEOUT)
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
    let outcome = spawn_login(&from_line("/bin/echo hi"), Duration::from_secs(5))
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
    let outcome = spawn_login(&from_line("/bin/sleep 5"), Duration::from_secs(1))
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
    let process = spawn_login(&from_line("/bin/sleep 5"), Duration::from_secs(10))
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
