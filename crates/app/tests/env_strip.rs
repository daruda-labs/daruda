//! `daruda --env` against the real binary.
//!
//! The unit tests beside the module cover its argv grammar; only a spawn can
//! say whether the variable is actually gone from the child, which is the
//! whole reason the subcommand exists.

use std::process::Command;

/// The built binary. An integration test is what makes this available — the
/// app has no library target to reach the module through.
const DARUDA: &str = env!("CARGO_BIN_EXE_daruda");

/// Exits 0 only when every named variable is absent from its environment.
fn probe(unset: &[&str], require_absent: &[&str]) -> std::process::Output {
    let mut command = Command::new(DARUDA);
    command.arg("--env");
    for name in unset {
        command.args(["-u", name]);
    }
    command.arg(test_process::executable());
    for name in require_absent {
        command.args(["--absent-env", name]);
    }
    command
        .env("ANTHROPIC_API_KEY", "leaked")
        .env("CLAUDE_CODE_OAUTH_TOKEN", "leaked")
        .output()
        .expect("spawn daruda --env")
}

#[test]
fn a_named_variable_is_gone_from_the_child() {
    let out = probe(&["ANTHROPIC_API_KEY"], &["ANTHROPIC_API_KEY"]);
    assert!(
        out.status.success(),
        "the child still saw the variable: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn every_named_variable_is_gone() {
    let out = probe(
        &["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"],
        &["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"],
    );
    assert!(out.status.success(), "one of the two survived");
}

/// The fixture must be able to fail, or the two above prove nothing: an
/// unnamed variable has to still reach the child.
#[test]
fn a_variable_that_was_not_named_still_reaches_the_child() {
    let out = probe(&["ANTHROPIC_API_KEY"], &["CLAUDE_CODE_OAUTH_TOKEN"]);
    assert!(
        !out.status.success(),
        "the probe passed with a variable it was told to find absent — \
         it cannot be distinguishing anything"
    );
}

/// The child's exit code is the wrapper's, or a failing adapter would look
/// like a clean one.
#[test]
fn the_child_s_exit_code_is_forwarded() {
    let out = Command::new(DARUDA)
        .args(["--env", "-u", "ANTHROPIC_API_KEY"])
        .arg(test_process::executable())
        .args(["--exit", "3"])
        .output()
        .expect("spawn daruda --env");
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn a_program_that_does_not_exist_is_reported_not_silently_ok() {
    let out = Command::new(DARUDA)
        .args(["--env", "-u", "A", "no-such-program-daruda-env-test"])
        .output()
        .expect("spawn daruda --env");
    assert!(!out.status.success());
}

/// The wrapper's stdout *is* the adapter's JSON-RPC stream. A diagnostic
/// written there is a frame the client cannot parse, so every failure path
/// has to stay off it — which is exactly when a diagnostic is tempting.
#[test]
fn nothing_the_wrapper_says_reaches_the_protocol_stream() {
    for args in [
        vec!["--env", "-u", "A", "no-such-program-daruda-env-test"],
        vec!["--env", "-u", "A"],
        vec!["--env"],
    ] {
        let out = Command::new(DARUDA)
            .args(&args)
            .output()
            .expect("spawn daruda --env");
        assert!(
            out.stdout.is_empty(),
            "{args:?} wrote to the protocol stream: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

/// Nothing to run is a usage error, never a launch.
#[test]
fn an_unset_with_no_program_is_refused() {
    let out = Command::new(DARUDA)
        .args(["--env", "-u", "A"])
        .output()
        .expect("spawn daruda --env");
    assert_eq!(out.status.code(), Some(2));
}
