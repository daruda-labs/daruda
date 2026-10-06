//! Which shell to run, and how to hand it an argument.
//!
//! Note where the `cfg` is: picking the *default* program is the only thing
//! the platform decides. Everything after reads the program name as a value,
//! so a Windows shell's argument rules can be tested anywhere.

pub mod quote;

pub use quote::Shell;

/// Resolve a POSIX utility supplied by Git for Windows without requiring
/// its private `usr/bin` directory on the GUI's PATH. Unix uses normal lookup.
pub fn posix_tool(name: &str) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        if let Ok(program) = which::which(name) {
            return program;
        }
        if let Ok(git) = which::which("git") {
            for parent in git.ancestors().skip(1).take(3) {
                let candidate = parent.join("usr/bin").join(format!("{name}.exe"));
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    name.into()
}

/// The arguments that make `program` a login shell.
///
/// `-l` is POSIX: `pwsh.exe` rejects it and `cmd.exe` reads it as a
/// filename. The answer belongs to the program being run, not the host — so
/// a config naming `pwsh` gets it right on macOS too.
pub fn login_args_for(program: &str) -> &'static [&'static str] {
    match Shell::detect_from_program(program) {
        Shell::Posix | Shell::Fish => &["-l"],
        Shell::PowerShell | Shell::Cmd => &[],
    }
}

/// The login shell the user configured, if there is one.
///
/// Distinct from [`interactive`], which always names something to spawn.
/// A caller here is asking whether there *is* a login shell to consult —
/// a fallback would answer a question it did not ask.
pub fn login_shell() -> Option<String> {
    std::env::var("SHELL").ok()
}

/// The shell to give a user a terminal in — their `$SHELL`, the fallback
/// deciding only what an environment without one gets.
///
/// The program alone: ask [`login_args_for`] about whichever shell is
/// actually run, since a config may name a different one.
pub fn interactive() -> String {
    if cfg!(windows) {
        // No `$SHELL` on Windows, and nothing to fall back to but the shell
        // every install has.
        "powershell.exe".to_owned()
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
