//! `daruda --env -u NAME … <program> [args…]` — `env(1)`'s remove-a-variable
//! form, for hosts that do not ship one.
//!
//! An adapter must not inherit a managed account's credentials, and the ACP
//! SDK only ever *adds* to a child's environment, so removing one takes a
//! wrapper. Windows has `env(1)` only if Git for Windows happens to be there.

use std::ffi::{OsStr, OsString};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::{LogPolicy, LogWriter};

/// The subcommand. The argv after it is `env(1)`'s own, so the command line a
/// launch builds reads the same on either platform.
pub(crate) const SUBCOMMAND: &str = "--env";

/// `env(1)`'s remove-a-variable flag.
const UNSET_FLAG: &str = "-u";

/// What `--env` was asked to do: drop `unset`, then run `program` with `args`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Request {
    pub unset: Vec<OsString>,
    pub program: OsString,
    pub args: Vec<OsString>,
}

/// Returns `Some(exit_code)` when invoked as `daruda --env …`.
pub(crate) fn route() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != *SUBCOMMAND {
        return None;
    }
    Some(match parse(args) {
        Some(request) => run(request),
        // Refuse rather than run something that was not asked for. daruda
        // builds this line itself, so a malformed one is daruda's own bug.
        None => {
            report("daruda --env was given no program to run", None);
            2
        }
    })
}

/// Read `-u NAME` pairs until the first argument that is not one; that
/// argument is the program.
///
/// `env`'s grammar, deliberately: a `-u` after the program belongs to the
/// program, not to us.
fn parse(args: impl Iterator<Item = OsString>) -> Option<Request> {
    let mut args = args.peekable();
    let mut unset = Vec::new();
    while args.peek().is_some_and(|arg| *arg == *UNSET_FLAG) {
        args.next();
        unset.push(args.next()?);
    }
    let program = args.next()?;
    Some(Request {
        unset,
        program,
        args: args.collect(),
    })
}

/// Drop the variables and hand the process over.
///
/// Unix replaces this process, so the adapter's parent stays whoever spawned
/// the wrapper and the pipes are untouched. Windows has no `exec`: the wrapper
/// stays, the child inherits its handles, and its exit code is forwarded.
fn run(request: Request) -> i32 {
    let mut command = daruda_core::process::command(&request.program);
    command.args(&request.args);
    for name in &request.unset {
        command.env_remove(name);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // Only returns on failure.
        let error = command.exec();
        unrunnable(&request.program, &error)
    }
    #[cfg(not(unix))]
    match command.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => unrunnable(&request.program, &error),
    }
}

/// What a shell reports for a command it could not run.
fn unrunnable(program: &OsStr, error: &std::io::Error) -> i32 {
    report(
        "daruda --env could not run the program it was given",
        Some((program, error)),
    );
    127
}

/// Never to stdout: that is the adapter's protocol stream, and a diagnostic
/// written into it is a malformed frame the client cannot parse. The log is
/// initialized here because this subcommand runs before the app's own
/// bootstrap — the same thing `--mcp` does for the same reason.
fn report(message: &str, cause: Option<(&OsStr, &std::io::Error)>) {
    LogWriter::init(LogPolicy::default());
    let mut builder = ErrorReport::new(message)
        .severity(ErrorSeverity::Warning)
        .at(file!(), line!())
        .dedup("env_strip.run");
    if let Some((program, error)) = cause {
        builder = builder
            .with_context("program", program.to_string_lossy())
            .with_context("reason", error.to_string());
    }
    LogWriter::log(builder.build());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Option<Request> {
        parse(args.iter().map(OsString::from))
    }

    fn request(unset: &[&str], program: &str, args: &[&str]) -> Request {
        Request {
            unset: unset.iter().map(OsString::from).collect(),
            program: program.into(),
            args: args.iter().map(OsString::from).collect(),
        }
    }

    #[test]
    fn unsets_are_read_until_the_program() {
        assert_eq!(
            parsed(&["-u", "A", "-u", "B", "node", "x.js"]),
            Some(request(&["A", "B"], "node", &["x.js"]))
        );
    }

    #[test]
    fn a_program_with_no_unsets_is_still_a_request() {
        assert_eq!(parsed(&["node"]), Some(request(&[], "node", &[])));
    }

    /// The grammar's whole point: everything past the program is the
    /// program's, including a `-u` that looks like ours.
    #[test]
    fn a_flag_after_the_program_belongs_to_the_program() {
        assert_eq!(
            parsed(&["-u", "A", "node", "-u", "B"]),
            Some(request(&["A"], "node", &["-u", "B"]))
        );
    }

    #[test]
    fn an_unset_without_a_name_is_refused() {
        assert_eq!(parsed(&["-u"]), None);
        assert_eq!(parsed(&["-u", "A", "-u"]), None);
    }

    #[test]
    fn nothing_to_run_is_refused() {
        assert_eq!(parsed(&[]), None);
        assert_eq!(parsed(&["-u", "A"]), None);
    }
}
