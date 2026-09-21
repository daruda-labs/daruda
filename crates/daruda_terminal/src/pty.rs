//! PTY process management for daruda.
//!
//! Encapsulates PTY creation, shell spawning, and I/O channel setup.
//! Designed for testability: all PTY operations are grouped into
//! [`PtyHandle`] which owns reader/writer channels.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

/// Errors that can occur during PTY operations.
#[derive(Debug)]
pub enum PtyError {
    /// Failed to open a pseudo-terminal pair.
    OpenPty(String),
    /// Failed to spawn the shell process.
    SpawnShell(String),
    /// Failed to obtain reader or writer from the PTY master.
    Io(String),
}

impl std::fmt::Display for PtyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PtyError::OpenPty(e) => f.write_str(&crate::ux::strings::pty_open_failed(e)),
            PtyError::SpawnShell(e) => f.write_str(&crate::ux::strings::pty_spawn_shell_failed(e)),
            PtyError::Io(e) => f.write_str(&crate::ux::strings::pty_io_failed(e)),
        }
    }
}

impl std::error::Error for PtyError {}

/// Configuration for spawning a PTY session.
#[derive(Debug, Clone)]
pub struct PtyConfig {
    pub cols: u16,
    pub rows: u16,
    pub shell: String,
    pub env: Vec<(String, String)>,
    /// Initial working directory for the spawned shell. `None` lets the
    /// child inherit the parent process's cwd. Set this from the
    /// previously-focused pane's tracked cwd to mirror iTerm2's
    /// "Reuse previous session's directory".
    pub cwd: Option<PathBuf>,
}

impl Default for PtyConfig {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            shell: daruda_core::shell::interactive(),
            env: vec![
                ("TERM".into(), "xterm-256color".into()),
                ("COLORTERM".into(), "truecolor".into()),
                (
                    "TERM_PROGRAM".into(),
                    crate::ux::strings::TERM_PROGRAM_VALUE.into(),
                ),
            ],
            cwd: None,
        }
    }
}

/// Handle to a running PTY session.
///
/// Provides channels for stdin/stdout communication and the master
/// PTY reference for resize operations.
pub struct PtyHandle {
    /// Send bytes to the PTY's stdin.
    pub stdin_tx: mpsc::Sender<Vec<u8>>,
    /// Receive bytes from the PTY's stdout.
    pub stdout_rx: mpsc::Receiver<Vec<u8>>,
    /// Fires once when the shell process exits. Disconnection of the
    /// sender (waiter thread panic) is treated the same as an explicit
    /// signal so listeners never miss the termination event.
    pub exit_rx: mpsc::Receiver<()>,
    /// Reader / writer thread error reports. The PTY threads enqueue
    /// here when an I/O error makes them exit; the owning pane drains
    /// the channel from its stdout-poll loop and routes the report
    /// into [`crate::workspace::Workspace::report_error`] so the user
    /// sees the failure instead of having to read stderr (D5).
    /// Disconnected in stub handles.
    pub error_rx: mpsc::Receiver<ErrorReport>,
    /// Master PTY reference for resize operations. `None` in stub
    /// handles (test builds) where resize is a no-op.
    pub master: Option<Arc<dyn MasterPty + Send>>,
    /// PID of the shell process forked into this PTY's slave end.
    /// `None` in stub handles. Used by the PTY-tracker subsystem as the
    /// root from which to walk descendants in search of `claude` child
    /// processes.
    pub child_pid: Option<u32>,
}

impl PtyHandle {
    /// Decompose the handle into its parts, consuming it.
    /// Use this when you need to move `stdout_rx` into an async task.
    #[allow(clippy::type_complexity)]
    pub fn into_parts(
        self,
    ) -> (
        mpsc::Sender<Vec<u8>>,
        mpsc::Receiver<Vec<u8>>,
        mpsc::Receiver<()>,
        mpsc::Receiver<ErrorReport>,
        Option<Arc<dyn MasterPty + Send>>,
    ) {
        (
            self.stdin_tx,
            self.stdout_rx,
            self.exit_rx,
            self.error_rx,
            self.master,
        )
    }
}

impl PtyHandle {
    /// Resize the PTY to new dimensions. No-op when master is absent
    /// (stub handles used in tests).
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        let Some(master) = &self.master else {
            return Ok(());
        };
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Io(e.to_string()))
    }

    /// Send bytes to the PTY's stdin.
    pub fn write(&self, bytes: &[u8]) -> Result<(), PtyError> {
        self.stdin_tx
            .send(bytes.to_vec())
            .map_err(|e| PtyError::Io(e.to_string()))
    }
}

/// Spawn a PTY session with the given configuration.
///
/// Under test this delegates to [`spawn_pty_stub`] so that tests which
/// merely construct a `Workspace` do not incur a real shell start-up
/// (~0.5 s each). Tests that exercise real PTY I/O should call
/// [`spawn_pty_real`] directly.
///
/// Keyed on the `test-support` feature as well as `cfg(test)`: a dependent
/// crate's test build compiles this one *without* `cfg(test)`, so the gate
/// that names only `test` leaves every app-side workspace test spawning a
/// real shell — which is where the cost this stub exists to remove lives.
///
/// In production builds this is identical to [`spawn_pty_real`].
#[cfg(any(test, feature = "test-support"))]
pub fn spawn_pty(_config: &PtyConfig) -> Result<PtyHandle, PtyError> {
    spawn_pty_stub()
}

/// Production PTY spawn — opens a kernel PTY pair, forks the shell,
/// and wires the I/O channels.
///
/// Call this instead of `spawn_pty` when a test genuinely needs a real
/// shell (echo I/O, resize kernel events, exit-signal delivery).
pub fn spawn_pty_real(config: &PtyConfig) -> Result<PtyHandle, PtyError> {
    let pty_system = native_pty_system();
    let pty_pair = pty_system
        .openpty(PtySize {
            rows: config.rows,
            cols: config.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| PtyError::OpenPty(e.to_string()))?;

    let master: Arc<dyn MasterPty + Send> = Arc::from(pty_pair.master);

    let mut cmd = CommandBuilder::new(&config.shell);
    // Login args belong to the shell being run — `-l` is POSIX, and handing
    // it to a Windows shell is an error rather than a no-op. The config may
    // name a different shell than the default, so ask about that one.
    for arg in daruda_core::shell::login_args_for(&config.shell) {
        cmd.arg(arg);
    }
    for (key, value) in &config.env {
        cmd.env(key, value);
    }
    // Use the requested cwd when it exists on disk; otherwise fall back to
    // $HOME so the shell never silently inherits the parent process's cwd
    // (which is $HOME or an arbitrary build directory depending on how
    // daruda was launched).
    let effective_cwd = config
        .cwd
        .as_ref()
        .filter(|p| p.read_dir().is_ok())
        .cloned()
        .or_else(|| {
            let home = PathBuf::from(std::env::var_os("HOME")?);
            home.is_dir().then_some(home)
        });
    if let Some(cwd) = &effective_cwd {
        cmd.cwd(cwd);
    }

    let mut child = pty_pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| PtyError::SpawnShell(e.to_string()))?;
    // Capture the PID before the waiter thread takes ownership of
    // `child`. Used by PtyTracker as the descendant-walk root so we can
    // find `claude` children of this PTY.
    let child_pid = child.process_id();

    // The exit channel fires once when the shell process terminates.
    // We use a bounded-capacity channel so a listener that polls slowly
    // still gets the signal on its next check.
    let (exit_tx, exit_rx) = mpsc::channel::<()>();

    // Waiter thread: blocks until the child exits, then reports.
    thread::spawn(move || {
        let _ = child.wait();
        let _ = exit_tx.send(());
    });

    let mut pty_reader = master
        .try_clone_reader()
        .map_err(|e| PtyError::Io(e.to_string()))?;
    let mut pty_writer = master
        .take_writer()
        .map_err(|e| PtyError::Io(e.to_string()))?;

    let (stdin_tx, stdin_rx) = mpsc::channel::<Vec<u8>>();
    let (stdout_tx, stdout_rx) = mpsc::channel::<Vec<u8>>();
    let (error_tx, error_rx) = mpsc::channel::<ErrorReport>();

    // PTY writer thread: stdin channel → PTY
    let writer_error_tx = error_tx.clone();
    thread::spawn(move || {
        while let Ok(bytes) = stdin_rx.recv() {
            if let Err(e) = pty_writer.write_all(&bytes) {
                // PTY closed under us (shell exited, kernel error).
                // Surface to the workspace so debugging "why did my
                // keystrokes stop reaching the shell" doesn't require a
                // tracer rebuild. Best-effort send — if the receiver is
                // gone the pane has already torn down.
                let _ = writer_error_tx.send(
                    ErrorReport::new(crate::ux::strings::pty_writer_thread_died())
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .dedup("pty.writer")
                        .build(),
                );
                break;
            }
            let _ = pty_writer.flush();
        }
    });

    // PTY reader thread: PTY → stdout channel
    let reader_error_tx = error_tx;
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            let n = match pty_reader.read(&mut buf) {
                Ok(0) => {
                    // EOF — normal shell exit path.
                    break;
                }
                Ok(n) => n,
                Err(e) => {
                    let _ = reader_error_tx.send(
                        ErrorReport::new(crate::ux::strings::pty_reader_thread_died())
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .dedup("pty.reader")
                            .build(),
                    );
                    break;
                }
            };
            // ConPTY close waits for its output pipe to drain. Keep reading
            // to EOF even after the pane drops its receiver, or teardown hangs.
            if stdout_tx.send(buf[..n].to_vec()).is_err() && !cfg!(windows) {
                break;
            }
        }
    });

    Ok(PtyHandle {
        stdin_tx,
        stdout_rx,
        exit_rx,
        error_rx,
        master: Some(master),
        child_pid,
    })
}

/// Production entry point — calls `spawn_pty_real`. Only compiled when the
/// stub above is not, so `spawn_pty` never has two definitions.
#[cfg(not(any(test, feature = "test-support")))]
pub fn spawn_pty(config: &PtyConfig) -> Result<PtyHandle, PtyError> {
    spawn_pty_real(config)
}

/// Zero-cost stub used by dependent crates' tests. Returns live but silent
/// channels and no underlying subprocess — shell startup cost is eliminated
/// while the pane still reads as a running shell.
///
/// The senders are deliberately never dropped: a pane treats a signalled
/// *or* a disconnected `exit_rx` as shell termination, so letting them fall
/// out of scope would close every tab a test opens. One small leak per
/// stubbed pane, in test builds only.
#[cfg(any(test, feature = "test-support"))]
pub fn spawn_pty_stub() -> Result<PtyHandle, PtyError> {
    let (stdin_tx, _stdin_rx) = mpsc::channel::<Vec<u8>>();
    let (stdout_tx, stdout_rx) = mpsc::channel::<Vec<u8>>();
    let (exit_tx, exit_rx) = mpsc::channel::<()>();
    let (error_tx, error_rx) = mpsc::channel::<ErrorReport>();
    std::mem::forget((stdout_tx, exit_tx, error_tx));
    Ok(PtyHandle {
        stdin_tx,
        stdout_rx,
        exit_rx,
        error_rx,
        master: None,
        child_pid: None,
    })
}

/// Compute terminal grid dimensions from pixel size and cell metrics.
pub fn compute_grid_size(
    width_px: f32,
    height_px: f32,
    cell_width: f32,
    cell_height: f32,
) -> (u16, u16) {
    let cols = (width_px / cell_width.max(1.0)).floor().max(1.0) as u16;
    let rows = (height_px / cell_height.max(1.0)).floor().max(1.0) as u16;
    (cols, rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_shell() -> &'static str {
        if cfg!(windows) { "cmd.exe" } else { "sh" }
    }

    fn spawn_test_pty(config: &PtyConfig) -> PtyHandle {
        let handle = spawn_pty_real(config).expect("spawn_pty_real failed");
        if cfg!(windows) {
            // portable-pty enables cursor inheritance. Unlike the real VT,
            // this raw-I/O fixture must answer ConPTY's startup query itself.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut output = Vec::new();
            while !output.windows(4).any(|bytes| bytes == b"\x1b[6n") {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                let bytes = handle
                    .stdout_rx
                    .recv_timeout(remaining)
                    .expect("ConPTY cursor query");
                output.extend_from_slice(&bytes);
            }
            handle.write(b"\x1b[1;1R").expect("answer cursor query");
        }
        handle
    }

    #[test]
    fn test_pty_config_default() {
        let config = PtyConfig::default();
        assert_eq!(config.cols, 80);
        assert_eq!(config.rows, 24);
        assert!(!config.shell.is_empty());
        assert!(config.env.iter().any(|(k, _)| k == "TERM"));
        assert!(config.env.iter().any(|(k, _)| k == "TERM_PROGRAM"));
    }

    #[test]
    fn test_compute_grid_size() {
        let (cols, rows) = compute_grid_size(800.0, 600.0, 8.0, 16.0);
        assert_eq!(cols, 100);
        assert_eq!(rows, 37);
    }

    #[test]
    fn test_compute_grid_size_minimum() {
        let (cols, rows) = compute_grid_size(1.0, 1.0, 100.0, 100.0);
        assert_eq!(cols, 1);
        assert_eq!(rows, 1);
    }

    #[test]
    fn test_compute_grid_size_zero_cell() {
        let (cols, rows) = compute_grid_size(800.0, 600.0, 0.0, 0.0);
        // cell_width.max(1.0) prevents division by zero
        assert!(cols >= 1);
        assert!(rows >= 1);
    }

    #[test]
    fn test_spawn_pty_with_echo() {
        let config = PtyConfig {
            cols: 80,
            rows: 24,
            shell: test_shell().into(),
            env: vec![("TERM".into(), "dumb".into())],
            cwd: None,
        };
        let handle = spawn_test_pty(&config);

        // Send a command
        handle.write(b"echo PTY_ECHO_OK\r").expect("write failed");

        // Read output (with timeout)
        let mut output = String::new();
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(3) {
            if let Ok(bytes) = handle.stdout_rx.try_recv() {
                output.push_str(&String::from_utf8_lossy(&bytes));
                if output.contains("PTY_ECHO_OK") {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(
            output.contains("PTY_ECHO_OK"),
            "Expected PTY_ECHO_OK in output, got: {output}"
        );
    }

    #[test]
    fn test_pty_resize() {
        let config = PtyConfig {
            cols: 80,
            rows: 24,
            shell: test_shell().into(),
            env: vec![("TERM".into(), "dumb".into())],
            cwd: None,
        };
        let handle = spawn_test_pty(&config);
        // Resize should not error
        handle.resize(120, 40).expect("resize failed");
    }

    #[test]
    fn test_pty_error_display() {
        let err = PtyError::OpenPty("test".into());
        assert_eq!(err.to_string(), "Failed to open PTY: test");

        let err = PtyError::SpawnShell("not found".into());
        assert_eq!(err.to_string(), "Failed to spawn shell: not found");

        let err = PtyError::Io("broken pipe".into());
        assert_eq!(err.to_string(), "PTY I/O failed: broken pipe");
    }

    /// Probe portable-pty's behavior for a nonexistent shell path.
    /// On unix, `spawn_command` forks then execs; exec failure surfaces
    /// synchronously on macOS via posix_spawn's exit-status pipe, so we
    /// expect `PtyError::SpawnShell`. If a platform ever shifts to
    /// deferred reporting, we want the test to loudly flag it rather
    /// than silently pass.
    #[test]
    fn test_spawn_pty_rejects_nonexistent_shell() {
        let config = PtyConfig {
            cols: 80,
            rows: 24,
            shell: "/definitely/not/a/real/shell/zzzdaruda".into(),
            env: vec![],
            cwd: None,
        };
        let result = spawn_pty_real(&config);
        match result {
            Err(PtyError::SpawnShell(msg)) => {
                assert!(!msg.is_empty(), "error message should not be empty");
            }
            Err(other) => panic!("expected SpawnShell, got {other}"),
            Ok(_) => panic!("spawn_pty unexpectedly succeeded for nonexistent shell"),
        }
    }

    /// `exit_rx` must fire once the shell exits. We send `exit\n` and
    /// poll for up to 5 seconds — the waiter thread calls `child.wait`
    /// which returns as soon as the shell's main process terminates.
    #[test]
    fn test_exit_rx_fires_on_shell_exit() {
        let config = PtyConfig {
            cols: 80,
            rows: 24,
            shell: test_shell().into(),
            env: vec![("TERM".into(), "dumb".into())],
            cwd: None,
        };
        let handle = spawn_test_pty(&config);
        // Ask the shell to terminate. `exit` is a shell builtin in sh.
        handle.write(b"exit\r").expect("write failed");

        let start = std::time::Instant::now();
        let mut got_exit = false;
        while start.elapsed() < std::time::Duration::from_secs(5) {
            match handle.exit_rx.try_recv() {
                Ok(()) => {
                    got_exit = true;
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Waiter thread went away without signaling — treat
                    // as termination (matches the pane-side handling).
                    got_exit = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
        assert!(got_exit, "exit_rx did not fire within 5s after `exit`");
    }
}
