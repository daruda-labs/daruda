//! Hydrate the process `PATH` from the user's login shell.
//!
//! A macOS `.app` launched from Finder / Dock / `open` inherits only launchd's
//! minimal `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`) and none of the user's
//! shell configuration. Tools installed via Homebrew, nvm, or npm-global —
//! including `npx`, which the ACP agent adapter is spawned as
//! (`daruda_acp::connection::AdapterCommand`) — then aren't on `PATH`, so the
//! spawn fails with `No such file or directory` (os error 2). A terminal
//! launch already inherits the shell `PATH`, so this is a no-op there.
//!
//! Mirrors zed's `load_login_shell_environment` and the approach every macOS
//! dev tool uses. We capture only `PATH` (not the full environment): it is what
//! subprocess discovery needs, and nvm / Homebrew put the active toolchain
//! directories on `PATH`, so a `PATH`-only capture is enough to find `npx`,
//! `node`, and `claude`.

use std::io::IsTerminal;

/// Sentinels wrapping the captured `PATH` on stdout. An interactive shell
/// (`-i`) runs the user's prompt / precmd hooks, which can emit stray output
/// (OSC 7 CWD reports, theme escapes, greetings) around our command. Bracketing
/// the value lets us extract exactly `PATH` and ignore that noise.
const PATH_START: &str = "__DARUDA_PATH_START__";
const PATH_END: &str = "__DARUDA_PATH_END__";

/// Read `PATH` from a login + interactive shell and set it on this process,
/// when launched as a GUI bundle. Best-effort: any failure leaves `PATH`
/// unchanged (no worse than before). Call once, early in `main`, before any
/// subprocess is spawned.
pub fn hydrate_path_from_login_shell() {
    // A terminal launch already carries the user's `PATH`; nothing to fix, and
    // spawning a shell would only add startup latency.
    if std::io::stdout().is_terminal() {
        return;
    }
    let Ok(shell) = std::env::var("SHELL") else {
        return;
    };
    // `-l -i` so both login (`.zprofile` / `.profile`) and interactive
    // (`.zshrc` / `.bashrc`) rc files run — Homebrew and nvm typically extend
    // `PATH` from one of those. The value is bracketed by the sentinels so
    // interactive-shell prompt noise on stdout doesn't corrupt it.
    let script = format!("printf '%s%s%s' '{PATH_START}' \"$PATH\" '{PATH_END}'");
    let output = std::process::Command::new(&shell)
        .args(["-l", "-i", "-c", &script])
        .output();
    let Ok(output) = output else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let Some(path) = extract_path(&String::from_utf8_lossy(&output.stdout)) else {
        return;
    };
    // SAFETY: called once at the very start of `main`, before any thread or
    // subprocess is spawned, so there is no concurrent environment access —
    // the safety contract of `std::env::set_var` under the 2024 edition.
    unsafe {
        std::env::set_var("PATH", path);
    }
}

/// Pull the `PATH` value out of the shell's stdout, taking only the text
/// between the sentinels (so surrounding prompt noise is dropped) and rejecting
/// an empty result. `PATH` itself is used verbatim — no trimming — since path
/// entries may legitimately contain spaces.
fn extract_path(raw: &str) -> Option<String> {
    let start = raw.find(PATH_START)? + PATH_START.len();
    let rest = &raw[start..];
    let end = rest.find(PATH_END)?;
    let path = &rest[..end];
    (!path.is_empty()).then(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrap(path: &str) -> String {
        format!("{PATH_START}{path}{PATH_END}")
    }

    #[test]
    fn extract_path_reads_between_sentinels() {
        assert_eq!(
            extract_path(&wrap("/opt/homebrew/bin:/usr/bin")).as_deref(),
            Some("/opt/homebrew/bin:/usr/bin"),
        );
    }

    #[test]
    fn extract_path_ignores_surrounding_prompt_noise() {
        // Simulate an interactive shell emitting an OSC 7 escape and a newline
        // around the bracketed value.
        let noisy = format!(
            "\u{1b}]7;file://host/Users/woo\u{7}{}\n",
            wrap("/Users/woo/.nvm/versions/node/v22.19.0/bin:/usr/bin"),
        );
        assert_eq!(
            extract_path(&noisy).as_deref(),
            Some("/Users/woo/.nvm/versions/node/v22.19.0/bin:/usr/bin"),
        );
    }

    #[test]
    fn extract_path_preserves_inner_spaces() {
        assert_eq!(
            extract_path(&wrap("/Applications/My App/bin:/usr/bin")).as_deref(),
            Some("/Applications/My App/bin:/usr/bin"),
        );
    }

    #[test]
    fn extract_path_rejects_empty_or_missing_sentinels() {
        assert_eq!(extract_path(&wrap("")).as_deref(), None);
        assert_eq!(extract_path("no sentinels here").as_deref(), None);
        assert_eq!(extract_path(PATH_START).as_deref(), None);
    }
}
