//! Native subprocess fixtures for tests of process behavior, without a shell.

/// Executable built for the same target as the tests.
pub fn executable() -> &'static std::path::Path {
    std::path::Path::new(env!("TEST_PROCESS_EXE"))
}

/// A real foreign process whose lifetime stays within the test's scope.
pub struct RunningChild(std::process::Child);

impl RunningChild {
    pub fn id(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for RunningChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn fixture_command() -> std::process::Command {
    let command = std::process::Command::new(executable());
    #[cfg(windows)]
    let command = {
        let mut command = command;
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
        command
    };
    command
}

pub fn sleeping() -> RunningChild {
    RunningChild(
        fixture_command()
            .args(["--sleep-ms", "60000"])
            .spawn()
            .expect("spawn scoped process fixture"),
    )
}

/// Quote argv for APIs whose documented input uses shell-words syntax.
pub fn command_line(args: &[&str]) -> String {
    shell_words::join(
        std::iter::once(executable().to_str().expect("fixture path")).chain(args.iter().copied()),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn process_reports_output_exit_and_environment_independently() {
        let result = super::fixture_command()
            .args([
                "--stdout",
                "out",
                "--stderr",
                "err",
                "--require-env",
                "TEST_VALUE",
                "a b",
                "--exit",
                "7",
            ])
            .env("TEST_VALUE", "a b")
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(7));
        assert_eq!(result.stdout, b"out");
        assert_eq!(result.stderr, b"err");
    }

    #[test]
    fn command_line_round_trips_arguments_with_spaces_and_quotes() {
        let args = ["a b", "c'd", "C:\\a\\b"];
        let parsed = shell_words::split(&super::command_line(&args)).unwrap();
        assert_eq!(&parsed[1..], args);
    }

    #[cfg(windows)]
    #[test]
    fn an_orphaned_fixture_does_not_create_a_console() {
        use std::os::windows::process::CommandExt as _;
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("child.pid");
        let status = super::fixture_command()
            .args(["--orphan", pid_file.to_str().unwrap()])
            .env("TEST_PROCESS_CONSOLE_DIR", directory.path())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let pid = std::fs::read_to_string(pid_file).unwrap();
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                let _ = std::process::Command::new("taskkill.exe")
                    .creation_flags(CREATE_NO_WINDOW)
                    .args(["/F", "/PID", &self.0])
                    .output();
            }
        }
        let _cleanup = Cleanup(pid.clone());
        let state_file = directory.path().join(format!("{pid}.state"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let state = loop {
            if let Ok(state) = std::fs::read_to_string(&state_file)
                && !state.is_empty()
            {
                break state;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "descendant did not start"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(state, "none");
    }
}
