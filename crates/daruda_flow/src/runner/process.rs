//! A command node: one shell line acting as a gate. The only file in this
//! crate that starts a process — everything else is handed finished values.
//!
//! On Unix the child leads its own process group, so a timeout or a cancel
//! kills the whole tree. **Elsewhere only the direct child is killed**: a
//! gate's grandchildren (the compiler, the test runner) survive. This repo
//! is macOS-first and Windows is unported, and a difference this large is
//! written down rather than left to be discovered.

use crate::model::AgentSpec;
use crate::runner::{
    CANCELED, CancelToken, NodeFailure, NodeRunner, RunContext, RunResult, canceled, sleep,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A command node is a shell line, not an argv: the design's flagship gate
/// pipes and quotes, and this repo already builds shell strings elsewhere.
const SHELL: &str = "sh";

/// Design §9 — what a node's script orients itself by.
const RUN_DIR_VAR: &str = "DARUDA_FLOW_RUN_DIR";
const NODE_ID_VAR: &str = "DARUDA_FLOW_NODE_ID";
const ATTEMPT_VAR: &str = "DARUDA_FLOW_ATTEMPT";

/// Runs a command node. Holds the credentials to strip because that is a
/// property of the whole run, not of any one node: the union of every
/// launch spec's `strip_env`, computed by the host that owns the request.
pub struct ProcessRunner {
    strip_env: Vec<String>,
}

/// Why the wait ended. The child exiting is the only one that carries a
/// verdict; the other two mean the tree still has to be taken down.
enum Stop {
    Exited(std::io::Result<std::process::ExitStatus>),
    Timeout,
    Canceled,
}

impl ProcessRunner {
    /// `strip_env` is the union of every `LaunchSpec::strip_env` in the run.
    pub fn new(strip_env: Vec<String>) -> Self {
        Self { strip_env }
    }

    /// What this runner will unset. Readable so a host can assert it
    /// actually handed its credentials over — passing an empty list is a
    /// silent leak with no other symptom.
    pub fn strip_env(&self) -> &[String] {
        &self.strip_env
    }

    async fn execute(&self, ctx: &RunContext<'_>, run: &str, log_path: &Path) -> RunResult {
        let log = match open_log(ctx.log_dir, log_path) {
            Ok(log) => log,
            Err(e) => return failed(Vec::new(), format!("could not open {log_path:?}: {e}")),
        };
        // Everything past here has a log on disk, so the attempt is
        // reportable however it ends.
        let artifacts = vec![log_path.to_path_buf()];

        let mut child = match self.spawn(ctx, run, log) {
            Ok(child) => child,
            Err(e) => return failed(artifacts, format!("could not run `{run}`: {e}")),
        };
        let pid = child.id();
        let started = std::time::Instant::now();

        // Scoped so the racing futures are dropped before `child` is used
        // again — the losers of the race hold it.
        let stop = {
            let status = child.status();
            smol::future::or(
                async move { Stop::Exited(status.await) },
                smol::future::or(expire(ctx.timeout), watch_cancel(ctx.cancel)),
            )
            .await
        };

        let outcome = match stop {
            Stop::Exited(Ok(status)) if status.success() => Ok(()),
            Stop::Exited(Ok(status)) => Err(NodeFailure::Exit {
                code: status.code(),
            }),
            Stop::Exited(Err(e)) => {
                return failed(artifacts, format!("could not wait for `{run}`: {e}"));
            }
            Stop::Timeout => {
                let elapsed = started.elapsed();
                kill_tree(pid, &mut child).await;
                Err(NodeFailure::Timeout { elapsed })
            }
            Stop::Canceled => {
                kill_tree(pid, &mut child).await;
                Err(NodeFailure::SessionError(CANCELED.to_string()))
            }
        };
        RunResult {
            outcome,
            artifacts,
            usage: None,
        }
    }

    fn spawn(
        &self,
        ctx: &RunContext<'_>,
        run: &str,
        log: std::fs::File,
    ) -> std::io::Result<smol::process::Child> {
        let errors = log.try_clone()?;
        let mut cmd = std::process::Command::new(SHELL);
        cmd.arg("-c")
            .arg(run)
            .current_dir(ctx.cwd)
            .env(RUN_DIR_VAR, ctx.run_dir)
            .env(NODE_ID_VAR, ctx.node_id.as_str())
            .env(ATTEMPT_VAR, ctx.attempt.to_string());
        for name in &self.strip_env {
            cmd.env_remove(name);
        }
        // The child leads its own group so a stop reaches its whole tree.
        // `async_process` exposes no equivalent, hence the std detour.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);

        // Stdio must be set *after* the conversion: `From` clears the three
        // flags `spawn` reads, and spawn then overwrites anything std had
        // configured with `inherit()`.
        //
        // Both streams get the same open file, so the log interleaves the
        // way `>file 2>&1` does and survives a kill mid-write.
        let mut cmd = smol::process::Command::from(cmd);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(log))
            .stderr(std::process::Stdio::from(errors));
        cmd.spawn()
    }
}

fn open_log(log_dir: &Path, log_path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::create_dir_all(log_dir)?;
    std::fs::File::create(log_path)
}

/// A failure of the runner itself rather than of the gate. Not an exit
/// code: nothing exited, and inventing one would read as a real verdict.
fn failed(artifacts: Vec<PathBuf>, message: String) -> RunResult {
    RunResult {
        outcome: Err(NodeFailure::SessionError(message)),
        artifacts,
        usage: None,
    }
}

async fn expire(timeout: Duration) -> Stop {
    sleep(timeout).await;
    Stop::Timeout
}

async fn watch_cancel(cancel: &CancelToken) -> Stop {
    canceled(cancel).await;
    Stop::Canceled
}

/// `child.kill()` reaches only `sh`, and a gate's real work is its children.
/// The reap afterwards is what keeps a night of repeated timeouts from
/// accumulating zombies.
async fn kill_tree(pid: u32, child: &mut smol::process::Child) {
    #[cfg(unix)]
    {
        // SAFETY: `process_group(0)` made this pid the leader of a group
        // containing only this node's tree, and the child is unreaped here
        // so the OS cannot have reused the id.
        unsafe { libc::killpg(pid as libc::pid_t, libc::SIGKILL) };
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        let _ = child.kill();
    }
    let _ = child.status().await;
}

impl NodeRunner for ProcessRunner {
    fn run_agent<'a>(
        &'a self,
        _ctx: &'a RunContext<'a>,
        _agent: &'a AgentSpec,
        _prompt: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
        Box::pin(async move {
            failed(
                Vec::new(),
                "agent nodes are not this runner's — an ACP runner takes them".to_string(),
            )
        })
    }

    fn run_command<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        run: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
        let log_path = ctx.log_dir.join(format!(
            "{}.attempt-{}.evidence-{}.log",
            ctx.node_id, ctx.attempt, ctx.evidence_seq
        ));
        Box::pin(async move { self.execute(ctx, run, &log_path).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PermissionPolicy;
    use crate::runner::CancelToken;
    use std::path::Path;
    use std::time::Duration;

    /// The owned half of a `RunContext`, which borrows everything. Node id
    /// and attempt are fixed so the env test can assert on their text.
    struct Fixture {
        node: crate::NodeId,
        cwd: PathBuf,
        run_dir: PathBuf,
        log_dir: PathBuf,
        cancel: CancelToken,
        timeout: Duration,
    }

    impl Fixture {
        fn new(dir: &Path) -> Self {
            let run_dir = dir.join("run");
            Self {
                node: "gate".to_string(),
                cwd: dir.to_path_buf(),
                log_dir: run_dir.join("logs"),
                run_dir,
                cancel: CancelToken::default(),
                timeout: Duration::from_secs(30),
            }
        }

        fn context(&self) -> RunContext<'_> {
            RunContext {
                node_id: &self.node,
                attempt: 1,
                cwd: &self.cwd,
                run_dir: &self.run_dir,
                log_dir: &self.log_dir,
                output: None,
                evidence_seq: 1,
                timeout: self.timeout,
                permission: PermissionPolicy::Deny,
                cancel: &self.cancel,
            }
        }

        fn run(&self, runner: &ProcessRunner, line: &str) -> RunResult {
            smol::block_on(runner.run_command(&self.context(), line))
        }
    }

    fn run_command_in(dir: &Path, line: &str) -> RunResult {
        Fixture::new(dir).run(&ProcessRunner::new(Vec::new()), line)
    }

    fn read_log(result: &RunResult) -> String {
        std::fs::read_to_string(&result.artifacts[0]).expect("the log was written")
    }

    /// A shell line that records the pid of a *grandchild* — the process a
    /// `child.kill()` would leave behind — and then blocks for far longer
    /// than any test would wait.
    fn long_running_tree(pid_file: &Path) -> String {
        format!(
            "{SHELL} -c 'echo $$ > {}; sleep 30' & wait",
            pid_file.display()
        )
    }

    /// Asks the OS about one pid. Signal 0 performs the permission and
    /// existence checks without delivering anything.
    #[cfg(unix)]
    fn process_is_alive(pid: u32) -> bool {
        // SAFETY: signal 0 delivers nothing; it only reports whether the pid
        // is claimed. Any pid value is a valid argument.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    /// The kill is asynchronous and the grandchild is reparented before it
    /// is reaped, so "gone" is a bounded wait rather than an instant.
    #[cfg(unix)]
    fn wait_until_gone(pid: u32) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if !process_is_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[cfg(unix)]
    fn grandchild_pid(pid_file: &Path) -> u32 {
        // The shell writes the pid before sleeping, but the write races the
        // parent's timeout on a loaded machine.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(text) = std::fs::read_to_string(pid_file)
                && let Ok(pid) = text.trim().parse()
            {
                return pid;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the grandchild never recorded itself"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The gate's whole contract: exit 0 passes, anything else fails with
    /// the code, and either way the output is on disk for a repair to read.
    #[test]
    fn a_failing_command_reports_its_code_and_leaves_its_output() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = run_command_in(dir.path(), "echo out; echo err >&2; exit 3");

        assert_eq!(result.outcome, Err(NodeFailure::Exit { code: Some(3) }));
        let log = read_log(&result);
        // One file, both streams: a repair prompt reads one path, and the
        // interleaving is what a person would have seen in a terminal.
        assert!(log.contains("out"), "{log}");
        assert!(log.contains("err"), "{log}");
    }

    /// The record wants the log of a passing gate too — a gate that passed
    /// for the wrong reason is only visible in what it printed.
    #[test]
    fn a_command_that_exits_zero_passes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = run_command_in(dir.path(), "echo fine");

        assert_eq!(result.outcome, Ok(()));
        assert!(read_log(&result).contains("fine"));
        assert_eq!(result.usage, None, "a command is not a session");
    }

    /// The artifact name is the archive's and `run.md`'s key, so it is part
    /// of the contract rather than an implementation detail.
    #[test]
    fn the_log_is_named_for_its_node_attempt_and_evidence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = run_command_in(dir.path(), "true");

        let name = result.artifacts[0]
            .file_name()
            .expect("a file name")
            .to_string_lossy()
            .into_owned();
        assert_eq!(name, "gate.attempt-1.evidence-1.log");
    }

    /// `run` is a shell line, not an argv — the design's flagship gate is
    /// `grep -q '^VERDICT: PASS' …`, which needs quoting and a pipe to work.
    #[test]
    fn a_command_is_a_shell_line_not_an_argv() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = run_command_in(dir.path(), "printf 'a\\nb\\n' | grep -q '^b'");
        assert_eq!(result.outcome, Ok(()));
    }

    /// The node's `cwd`, not the process's — a flow runs in a lane the host
    /// chose, and a command that resolves paths against the wrong root would
    /// silently read a different tree.
    #[test]
    fn a_command_runs_in_the_nodes_working_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("marker"), "x").expect("marker");

        let result = run_command_in(dir.path(), "test -f marker");
        assert_eq!(result.outcome, Ok(()));
    }

    /// The timeout must kill the whole tree, not just the shell. Returning
    /// quickly proves nothing — killing `sh` alone does that too while the
    /// real work (a build, a test runner) keeps going.
    #[cfg(unix)]
    #[test]
    fn a_command_over_its_timeout_kills_its_children_too() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pid_file = dir.path().join("child.pid");
        let mut fixture = Fixture::new(dir.path());
        fixture.timeout = Duration::from_millis(200);

        let result = fixture.run(
            &ProcessRunner::new(Vec::new()),
            &long_running_tree(&pid_file),
        );
        assert!(
            matches!(result.outcome, Err(NodeFailure::Timeout { .. })),
            "{:?}",
            result.outcome
        );

        let pid = grandchild_pid(&pid_file);
        assert!(wait_until_gone(pid), "the grandchild outlived its node");
    }

    /// A cancel mid-command stops it for the same reason a timeout does, and
    /// reports the stop rather than a spurious exit code. It has to reach
    /// the grandchild too, for the same reason.
    #[cfg(unix)]
    #[test]
    fn a_canceled_command_stops_without_inventing_an_exit_code() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pid_file = dir.path().join("child.pid");
        let fixture = Fixture::new(dir.path());

        let cancel = fixture.cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            cancel.cancel();
        });

        let result = fixture.run(
            &ProcessRunner::new(Vec::new()),
            &long_running_tree(&pid_file),
        );
        assert!(
            !matches!(result.outcome, Err(NodeFailure::Exit { .. })),
            "a stop is not an exit status: {:?}",
            result.outcome
        );
        assert!(
            !matches!(result.outcome, Err(NodeFailure::Timeout { .. })),
            "a user stop must not read as a timeout: {:?}",
            result.outcome
        );

        let pid = grandchild_pid(&pid_file);
        assert!(wait_until_gone(pid), "the grandchild outlived the cancel");
    }

    /// Design §9: a node's script gets three variables to orient itself by.
    /// Without them a gate cannot find the outputs it is meant to check.
    #[test]
    fn a_command_is_told_which_run_node_and_attempt_it_is() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = run_command_in(
            dir.path(),
            &format!("echo \"${NODE_ID_VAR} ${ATTEMPT_VAR} ${RUN_DIR_VAR}\""),
        );

        let log = read_log(&result);
        assert!(log.contains("gate 1"), "{log}");
        assert!(
            log.contains(&dir.path().join("run").display().to_string()),
            "{log}"
        );
    }

    /// Design §9's stated cost, and the one mitigation it commits to: flow
    /// files are committed and shared, so the credentials the ACP path
    /// strips must not survive into a shell any flow author can write.
    ///
    /// The stripped name is one cargo already set in this process, because
    /// `std::env::set_var` is unsafe in edition 2024 and racing it against
    /// the spawns the rest of this suite does is exactly the hazard that
    /// made it unsafe.
    #[test]
    fn a_command_cannot_see_the_credentials_the_acp_path_strips() {
        let inherited = "CARGO_PKG_NAME";
        assert!(
            std::env::var_os(inherited).is_some(),
            "this test is only meaningful when {inherited} is actually inherited"
        );

        let dir = tempfile::tempdir().expect("tempdir");
        let runner = ProcessRunner::new(vec![inherited.to_string()]);
        let result = Fixture::new(dir.path()).run(&runner, &format!("echo \"[${inherited}]\""));

        assert!(read_log(&result).contains("[]"), "{}", read_log(&result));
    }

    /// …but everything else is inherited, because a gate that cannot see
    /// `PATH` or `HOME` simply does not run — which is why §9 chose
    /// inheritance over a whitelist.
    #[test]
    fn a_command_still_inherits_the_environment_it_needs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runner = ProcessRunner::new(vec!["CARGO_PKG_NAME".to_string()]);
        let result = Fixture::new(dir.path()).run(&runner, "echo \"[$PATH]\"");

        let log = read_log(&result);
        assert!(
            !log.contains("[]"),
            "stripping one name emptied the rest: {log}"
        );
    }
}
