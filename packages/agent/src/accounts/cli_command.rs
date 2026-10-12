//! An agent CLI invocation as argv, never a shell line.
//!
//! The caller resolves the program the session itself runs (the adapter's
//! prepared installation); this crate only spawns it. Taking argv rather than
//! a string keeps a `KEY=v` prefix or a quoted path from being re-read as the
//! program name.

use std::path::PathBuf;

/// What to run for a login or an auth-status read — complete: the caller's
/// preparation already folded in the account's environment and its strip
/// wrapper, exactly as for a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl CliCommand {
    pub(crate) fn command(&self) -> std::process::Command {
        let mut cmd = daruda_core::process::command_on_path(
            &self.program,
            daruda_core::process::child_path(self.env.iter().map(|(k, v)| (k, v))),
        );
        cmd.args(&self.args);
        cmd.envs(self.env.iter().map(|(k, v)| (k, v)));
        cmd
    }
}

/// A command spelled as a shell line, for tests that build fixtures that way.
#[cfg(test)]
pub(crate) fn from_line(line: &str) -> CliCommand {
    let mut words = shell_words::split(line).expect("a well-formed test command");
    let program = words.remove(0).into();
    CliCommand {
        program,
        args: words,
        env: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_is_never_split_out_of_the_args() {
        let command = CliCommand {
            program: "/opt/my node/bin/node".into(),
            args: vec!["FOO=1".into(), "--cli".into()],
            env: Vec::new(),
        };
        let cmd = command.command();
        assert_eq!(cmd.get_program(), "/opt/my node/bin/node");
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["FOO=1", "--cli"]);
    }
}
