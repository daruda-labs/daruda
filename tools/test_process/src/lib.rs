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

pub fn sleeping() -> RunningChild {
    let mut command = std::process::Command::new(executable());
    command.args(["--sleep-ms", "60000"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    RunningChild(command.spawn().expect("spawn scoped process fixture"))
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
        let result = std::process::Command::new(super::executable())
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
}
