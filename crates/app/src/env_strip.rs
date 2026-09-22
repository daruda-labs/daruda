//! `daruda --env [-u NAME]… [NAME=VALUE]… <program> [args…]` — the part of
//! `env(1)`'s grammar daruda emits, for hosts that do not ship one.
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

/// What `--env` was asked to do: drop `unset`, apply `set`, then run
/// `program` with `args`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Request {
    pub unset: Vec<OsString>,
    pub set: Vec<(OsString, OsString)>,
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

/// `-u NAME` pairs, then `NAME=VALUE` assignments, then the program.
///
/// `env`'s grammar, because that is what the launch builder emits: a managed
/// runtime prefixes `npm_config_cpu=…` operands, and reading the first of
/// those as the program is how this ran nothing at all. A `-u` past the first
/// operand belongs to the program — option parsing stops at the operands.
fn parse(args: impl Iterator<Item = OsString>) -> Option<Request> {
    let mut args = args.peekable();
    let mut unset = Vec::new();
    while args.peek().is_some_and(|arg| *arg == *UNSET_FLAG) {
        args.next();
        unset.push(args.next()?);
    }
    let mut set = Vec::new();
    while let Some(assignment) = args.peek().and_then(|arg| split_assignment(arg)) {
        args.next();
        set.push(assignment);
    }
    let program = args.next()?;
    Some(Request {
        unset,
        set,
        program,
        args: args.collect(),
    })
}

/// `NAME=VALUE` split at the first `=`, or `None` for anything else.
///
/// UTF-8 only: daruda builds these lines itself and spells every operand it
/// emits in UTF-8, and an argument that is not one is the program.
fn split_assignment(arg: &OsStr) -> Option<(OsString, OsString)> {
    let text = arg.to_str()?;
    let (name, value) = text.split_once('=')?;
    (!name.is_empty()).then(|| (OsString::from(name), OsString::from(value)))
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
    for (name, value) in &request.set {
        command.env(name, value);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // Only returns on failure.
        let error = command.exec();
        unrunnable(&request.program, &error)
    }
    #[cfg(not(unix))]
    {
        daruda_core::process::lead_own_group(&mut command);
    }
    #[cfg(not(unix))]
    match command.spawn() {
        Ok(mut child) => {
            // Immediately, and for two reasons: the wrapper stays between
            // the app and the adapter, so without a job the tear-down that
            // kills it leaves `npx` and `node` holding the protocol pipes —
            // and the child was spawned suspended, so this is what runs it.
            let group = daruda_core::process::Group::adopt(child.id());
            let code = match child.wait() {
                Ok(status) => status.code().unwrap_or(1),
                Err(error) => unrunnable(&request.program, &error),
            };
            drop(group);
            code
        }
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
            set: Vec::new(),
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

    /// The shape a managed runtime actually emits. Reading the first
    /// `NAME=value` as the program is how this launched nothing at all.
    #[test]
    fn assignments_are_applied_and_the_program_comes_after_them() {
        let parsed = parsed(&[
            "-u",
            "ANTHROPIC_API_KEY",
            "npm_config_cpu=x64",
            "npm_config_os=win32",
            "npx",
            "-y",
            "pkg",
        ])
        .expect("a launch line must parse");

        assert_eq!(parsed.program, OsString::from("npx"));
        assert_eq!(parsed.args, ["-y", "pkg"].map(OsString::from));
        assert_eq!(
            parsed.set,
            [("npm_config_cpu", "x64"), ("npm_config_os", "win32")]
                .map(|(n, v)| (OsString::from(n), OsString::from(v)))
        );
    }

    /// A value may hold anything, including the character that split it.
    #[test]
    fn an_assignment_splits_at_the_first_equals_only() {
        let parsed = parsed(&["npm_config_cache=C:/a=b/c", "npx"]).expect("parses");

        assert_eq!(
            parsed.set,
            [(
                OsString::from("npm_config_cache"),
                OsString::from("C:/a=b/c")
            )]
        );
    }

    /// Nothing before the `=` is not an assignment — it is a program with an
    /// odd name, and guessing otherwise would swallow it.
    #[test]
    fn a_leading_equals_is_not_an_assignment() {
        let parsed = parsed(&["=weird", "--flag"]).expect("parses");

        assert!(parsed.set.is_empty());
        assert_eq!(parsed.program, OsString::from("=weird"));
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
