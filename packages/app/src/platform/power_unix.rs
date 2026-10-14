//! macOS and Linux hold sleep inhibition through a helper child process.
//!
//! The child is tied to daruda's lifetime, so a crash never leaves the
//! machine inhibited: `caffeinate -w` watches our pid, and Linux's `cat`
//! reads EOF once the stdin pipe this lease holds closes.

use super::{PowerError, SleepLease};
use std::process::{Child, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// Which helper builds the inhibition, chosen once from the build target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SleepOs {
    MacOs,
    Linux,
}

impl SleepOs {
    fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

const CAFFEINATE: &str = "/usr/bin/caffeinate";
const SYSTEMD_INHIBIT: &str = "systemd-inhibit";
/// Shown only by `systemd-inhibit --list`, beside other inhibitors' English
/// reasons, so it stays an untranslated diagnostic like a log line.
const LINUX_INHIBIT_WHY: &str = "Agent work in progress";
/// A helper that exits sooner than this after starting was refused (no
/// logind, polkit); one that ran longer simply ended and is started again.
const EARLY_EXIT: Duration = Duration::from_secs(5);

/// Program and arguments that inhibit system sleep for `pid`'s lifetime.
pub(super) fn sleep_command(os: SleepOs, pid: u32) -> (&'static str, Vec<String>) {
    match os {
        // -i: idle sleep, -s: system sleep on AC, -w: exit with `pid`.
        SleepOs::MacOs => (
            CAFFEINATE,
            ["-i", "-s", "-w"]
                .into_iter()
                .map(str::to_owned)
                .chain([pid.to_string()])
                .collect(),
        ),
        SleepOs::Linux => (
            SYSTEMD_INHIBIT,
            vec![
                "--what=sleep".to_owned(),
                "--who=daruda".to_owned(),
                format!("--why={LINUX_INHIBIT_WHY}"),
                "--mode=block".to_owned(),
                "cat".to_owned(),
            ],
        ),
    }
}

/// Holds inhibition for as long as its child runs.
pub(super) struct ChildLease {
    program: &'static str,
    args: Vec<String>,
    /// The running helper and when it started.
    child: Option<(Child, Instant)>,
    /// How the last child ended right after starting, until the next hold
    /// reports it.
    exited: Option<ExitStatus>,
    early_exit: Duration,
}

impl ChildLease {
    pub(super) fn for_this_os() -> Self {
        let (program, args) = sleep_command(SleepOs::current(), std::process::id());
        Self::new(program, args)
    }

    fn new(program: &'static str, args: Vec<String>) -> Self {
        Self {
            program,
            args,
            child: None,
            exited: None,
            early_exit: EARLY_EXIT,
        }
    }

    fn spawn(&self) -> Result<Child, PowerError> {
        // stdin stays piped for the child's lifetime: closing it is the
        // Linux helper's release signal and harmless to `caffeinate`.
        daruda_core::process::command(self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(PowerError::Spawn)
    }

    fn release(&mut self) -> Result<(), PowerError> {
        self.exited = None;
        let Some((mut child, _)) = self.child.take() else {
            return Ok(());
        };
        drop(child.stdin.take());
        child.kill().map_err(PowerError::Os)?;
        // A killed child is reaped at once, so this blocks the GUI thread
        // only for the kernel's exit bookkeeping.
        child.wait().map(|_| ()).map_err(PowerError::Os)
    }
}

impl SleepLease for ChildLease {
    fn set_active(&mut self, active: bool) -> Result<(), PowerError> {
        if !active {
            return self.release();
        }
        if self.is_held() {
            return Ok(());
        }
        // A helper that quit on its own (no logind, a polkit refusal) would
        // quit again at once; report it so the caller pauses before retrying.
        if let Some(status) = self.exited.take() {
            return Err(PowerError::Exited(status));
        }
        self.child = Some((self.spawn()?, Instant::now()));
        Ok(())
    }

    fn is_held(&mut self) -> bool {
        let Some((child, started)) = self.child.as_mut() else {
            return false;
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                if started.elapsed() < self.early_exit {
                    self.exited = Some(status);
                }
                self.child = None;
                false
            }
            // An unanswerable status keeps the child: respawning beside a
            // live one would stack inhibitors.
            Ok(None) | Err(_) => true,
        }
    }
}

impl Drop for ChildLease {
    fn drop(&mut self) {
        if let Err(error) = self.release() {
            crate::platform::report_error(
                "desktop.power.lease",
                "Sleep inhibitor release failed",
                &error,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// `cat` blocks on the piped stdin like the Linux helper, inhibiting nothing.
    fn stand_in() -> ChildLease {
        ChildLease::new("cat", Vec::new())
    }

    fn pid(lease: &ChildLease) -> u32 {
        lease.child.as_ref().expect("child held").0.id()
    }

    fn wait_until_released(lease: &mut ChildLease) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while lease.is_held() {
            assert!(Instant::now() < deadline, "exited child still reads held");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn macos_watches_daruda_with_caffeinate() {
        assert_eq!(
            sleep_command(SleepOs::MacOs, 4242),
            (
                CAFFEINATE,
                vec!["-i".into(), "-s".into(), "-w".into(), "4242".into()]
            )
        );
        assert_eq!(CAFFEINATE, "/usr/bin/caffeinate");
    }

    #[test]
    fn linux_blocks_sleep_but_leaves_the_lid_to_logind() {
        assert_eq!(
            sleep_command(SleepOs::Linux, 4242),
            (
                "systemd-inhibit",
                vec![
                    "--what=sleep".to_owned(),
                    "--who=daruda".to_owned(),
                    format!("--why={LINUX_INHIBIT_WHY}"),
                    "--mode=block".to_owned(),
                    "cat".to_owned(),
                ]
            )
        );
    }

    #[test]
    fn hold_is_idempotent_and_release_reaps_the_child() {
        let mut lease = stand_in();
        assert!(!lease.is_held());
        lease.set_active(true).unwrap();
        assert!(lease.is_held());
        let first = pid(&lease);
        lease.set_active(true).unwrap();
        assert_eq!(pid(&lease), first);
        lease.set_active(false).unwrap();
        assert!(lease.child.is_none(), "release reaped the child");
        assert!(!lease.is_held());
    }

    #[test]
    fn a_child_that_dies_on_its_own_reads_released() {
        let mut lease = stand_in();
        lease.set_active(true).unwrap();
        lease.child.as_mut().unwrap().0.kill().unwrap();
        wait_until_released(&mut lease);
        assert!(lease.child.is_none());
        assert!(matches!(lease.set_active(true), Err(PowerError::Exited(_))));
        lease.set_active(true).unwrap();
        assert!(lease.is_held());
    }

    /// A helper that quits right away surfaces as `Exited` instead of being
    /// respawned on every pass.
    #[test]
    fn a_helper_that_exits_at_once_reports_exited() {
        let mut lease = ChildLease::new("false", Vec::new());
        lease.set_active(true).unwrap();
        wait_until_released(&mut lease);
        match lease.set_active(true) {
            Err(PowerError::Exited(status)) => assert!(!status.success()),
            other => panic!("expected Exited, got {other:?}"),
        }
        assert!(!lease.is_held());
    }

    #[test]
    fn releasing_forgets_an_exit() {
        let mut lease = ChildLease::new("false", Vec::new());
        lease.set_active(true).unwrap();
        wait_until_released(&mut lease);
        lease.set_active(false).unwrap();
        assert!(lease.exited.is_none());
    }

    /// A helper that ran past the early-exit window and then ended is simply
    /// started again, so a stale exit never blocks a later hold.
    #[test]
    fn a_helper_that_ran_then_ended_is_started_again() {
        let mut lease = stand_in();
        lease.early_exit = Duration::ZERO;
        lease.set_active(true).unwrap();
        lease.child.as_mut().unwrap().0.kill().unwrap();
        wait_until_released(&mut lease);
        assert!(lease.exited.is_none());
        lease.set_active(true).unwrap();
        assert!(lease.is_held());
    }

    /// The stand-in holds a FIFO's only write end, so its reader sees EOF
    /// exactly when that process is gone — no pid that could be reused.
    #[test]
    fn drop_releases_the_child() {
        use std::io::Read as _;
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("held");
        let made = daruda_core::process::command("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        let mut lease = ChildLease::new(
            "sh",
            vec![
                "-c".to_owned(),
                "exec cat > \"$0\"".to_owned(),
                fifo.display().to_string(),
            ],
        );
        lease.set_active(true).unwrap();
        let (closed_tx, closed_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut drained = Vec::new();
            let read = std::fs::File::open(&fifo).and_then(|mut f| f.read_to_end(&mut drained));
            let _ = closed_tx.send(read.is_ok());
        });
        assert!(
            closed_rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "the held child keeps the FIFO open"
        );
        drop(lease);
        assert_eq!(closed_rx.recv_timeout(Duration::from_secs(5)), Ok(true));
    }

    #[test]
    fn a_missing_helper_reports_spawn() {
        let mut lease = ChildLease::new("/nonexistent/daruda-sleep-helper", Vec::new());
        assert!(matches!(lease.set_active(true), Err(PowerError::Spawn(_))));
        assert!(!lease.is_held());
    }
}
