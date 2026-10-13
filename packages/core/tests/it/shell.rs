//! Public API tests for shell.

use daruda_core::shell::*;

#[test]
fn a_posix_shell_takes_a_login_flag() {
    assert_eq!(login_args_for("/bin/zsh"), &["-l"]);
    assert_eq!(login_args_for("/bin/bash"), &["-l"]);
    assert_eq!(login_args_for("fish"), &["-l"]);
}

/// Runs on every host, which is the point: handing `-l` to these is an
/// error, and nobody should have to be on Windows to find that out.
#[test]
fn a_windows_shell_takes_no_login_flag() {
    assert!(login_args_for("powershell.exe").is_empty());
    assert!(login_args_for("pwsh").is_empty());
    assert!(login_args_for(r"C:\Windows\System32\cmd.exe").is_empty());
}

/// A config naming an absolute path, with arguments of its own, still
/// classifies — `detect_from_program` reads the stem.
#[test]
fn the_answer_follows_the_program_not_the_host() {
    assert_eq!(login_args_for("/usr/local/bin/bash -i"), &["-l"]);
    assert!(login_args_for("pwsh -NoProfile").is_empty());
}

/// Whatever it picks has to be something the quoter can classify, since
/// the terminal quotes paths for the shell it just spawned.
#[test]
fn the_interactive_shell_is_one_the_quoter_recognizes() {
    let program = interactive();

    assert!(!program.is_empty());
    let _ = Shell::detect_from_program(&program);
}
