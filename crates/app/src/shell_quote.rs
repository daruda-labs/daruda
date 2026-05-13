//! Shell-aware path quoting for drag-and-drop into the terminal input.
//!
//! When a path is dropped into the input area it must reach the shell as a
//! single token even when it contains spaces or shell metacharacters. The
//! correct quoting depends on the shell flavour the focused pane is running,
//! so this module sniffs the shell's executable name from `PtyConfig::shell`
//! and applies the matching escape rules.
//!
//! Pure data / algorithm — no GPUI imports.

use std::path::{Path, PathBuf};

/// Shell flavour for quoting. Falls back to [`Shell::Posix`] for unknown
/// programs so a typo or custom wrapper still produces a quoted token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(clippy::enum_variant_names)] // PowerShell is the product name; renaming to Pwsh hurts readability
pub(crate) enum Shell {
    /// `bash`, `zsh`, `sh`, `dash`, `ksh` — POSIX rules. Inside `'…'` an
    /// embedded `'` is escaped as `'\''`.
    #[default]
    Posix,
    /// `fish` — single-quoted strings are literal; only `'` and `\` are
    /// escaped (with a leading backslash).
    Fish,
    /// `pwsh`, `powershell` — single-quoted strings are literal; an embedded
    /// `'` is doubled (`''`).
    PowerShell,
    /// Windows `cmd.exe` — wraps in `"…"`; an embedded `"` is doubled (`""`).
    Cmd,
}

impl Shell {
    /// Detect the shell flavour from a program path or name. The match is
    /// case-insensitive and uses the file stem so `/opt/homebrew/bin/fish`,
    /// `fish`, and `Fish.exe` all resolve to [`Shell::Fish`].
    ///
    /// Robust against:
    /// - Windows-style paths captured on a Unix host (splits on both `/`
    ///   and `\`) — `Path::file_stem` only treats `/` as a separator on
    ///   Unix and would otherwise glue the whole string into one filename.
    /// - Programs carrying arguments (`/bin/bash -l`, `fish --no-config`)
    ///   — the first whitespace-delimited token is treated as the program.
    pub(crate) fn detect_from_program(program: &str) -> Self {
        let program = program.split_ascii_whitespace().next().unwrap_or(program);
        let basename = program.rsplit(['/', '\\']).next().unwrap_or("");
        let stem = basename
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or(basename);
        match stem.to_ascii_lowercase().as_str() {
            "fish" => Shell::Fish,
            "pwsh" | "powershell" => Shell::PowerShell,
            "cmd" => Shell::Cmd,
            _ => Shell::Posix,
        }
    }
}

/// Quote `path` so the active shell receives it as a single token.
pub(crate) fn quote_path(path: &Path, shell: Shell) -> String {
    quote_str(&path.to_string_lossy(), shell)
}

/// Quote an arbitrary string using [`Shell`]-specific rules.
pub(crate) fn quote_str(s: &str, shell: Shell) -> String {
    match shell {
        Shell::Posix => quote_posix(s),
        Shell::Fish => quote_fish(s),
        Shell::PowerShell => quote_pwsh(s),
        Shell::Cmd => quote_cmd(s),
    }
}

/// Format multiple dropped paths for insertion at the cursor. Paths are
/// quoted individually and joined with a single space so the shell tokenizer
/// sees them as separate arguments.
pub(crate) fn format_paths_for_drop(paths: &[PathBuf], shell: Shell) -> String {
    paths
        .iter()
        .map(|p| quote_path(p, shell))
        .collect::<Vec<_>>()
        .join(" ")
}

const POSIX_SAFE: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_./";

fn is_posix_safe(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| POSIX_SAFE.contains(&b))
}

fn quote_posix(s: &str) -> String {
    if is_posix_safe(s) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn quote_fish(s: &str) -> String {
    if is_posix_safe(s) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        match ch {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    out.push('\'');
    out
}

fn quote_pwsh(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn quote_cmd(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        if ch == '"' {
            out.push_str("\"\"");
        } else {
            out.push(ch);
        }
    }
    // Trailing backslashes immediately before the closing `"` are
    // interpreted as escaping it under C runtime parsing
    // (CommandLineToArgvW), turning `"C:\Users\foo\"` into an unterminated
    // string and leaking the next argv token into our path. Double every
    // trailing backslash so each ends up as a literal `\` instead.
    let trailing = out.len() - out.trim_end_matches('\\').len();
    for _ in 0..trailing {
        out.push('\\');
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_known_shells() {
        assert_eq!(Shell::detect_from_program("/bin/zsh"), Shell::Posix);
        assert_eq!(Shell::detect_from_program("/bin/bash"), Shell::Posix);
        assert_eq!(Shell::detect_from_program("sh"), Shell::Posix);
        assert_eq!(Shell::detect_from_program("/usr/bin/dash"), Shell::Posix);
        assert_eq!(
            Shell::detect_from_program("/opt/homebrew/bin/fish"),
            Shell::Fish
        );
        assert_eq!(Shell::detect_from_program("FISH"), Shell::Fish);
        assert_eq!(Shell::detect_from_program("pwsh"), Shell::PowerShell);
        assert_eq!(
            Shell::detect_from_program("C:\\Windows\\System32\\powershell.exe"),
            Shell::PowerShell
        );
        assert_eq!(Shell::detect_from_program("cmd.exe"), Shell::Cmd);
    }

    #[test]
    fn unknown_shell_falls_back_to_posix() {
        assert_eq!(Shell::detect_from_program(""), Shell::Posix);
        assert_eq!(Shell::detect_from_program("/usr/bin/myshell"), Shell::Posix);
    }

    #[test]
    fn posix_skips_quotes_for_safe_paths() {
        assert_eq!(
            quote_str("/Users/me/foo.txt", Shell::Posix),
            "/Users/me/foo.txt"
        );
        assert_eq!(
            quote_str("relative/path-1.rs", Shell::Posix),
            "relative/path-1.rs"
        );
    }

    #[test]
    fn posix_quotes_paths_with_spaces() {
        assert_eq!(
            quote_str("/Users/me/My File.txt", Shell::Posix),
            "'/Users/me/My File.txt'"
        );
    }

    #[test]
    fn posix_escapes_internal_single_quote() {
        assert_eq!(
            quote_str("/x/it's mine.txt", Shell::Posix),
            "'/x/it'\\''s mine.txt'"
        );
    }

    #[test]
    fn posix_quotes_metacharacters() {
        // $, *, ;, |, &, (, ), <, >, !, # — all need quoting.
        assert_eq!(quote_str("$HOME", Shell::Posix), "'$HOME'");
        assert_eq!(quote_str("a;b", Shell::Posix), "'a;b'");
        assert_eq!(quote_str("a*b", Shell::Posix), "'a*b'");
        assert_eq!(quote_str("a&b", Shell::Posix), "'a&b'");
    }

    #[test]
    fn posix_empty_string_is_quoted() {
        assert_eq!(quote_str("", Shell::Posix), "''");
    }

    #[test]
    fn fish_escapes_single_quote_with_backslash() {
        assert_eq!(quote_str("it's", Shell::Fish), "'it\\'s'");
    }

    #[test]
    fn fish_escapes_backslash() {
        assert_eq!(quote_str("a\\b c", Shell::Fish), "'a\\\\b c'");
    }

    #[test]
    fn pwsh_doubles_single_quote() {
        assert_eq!(quote_str("it's", Shell::PowerShell), "'it''s'");
        assert_eq!(
            quote_str("C:\\Users\\me\\My File.txt", Shell::PowerShell),
            "'C:\\Users\\me\\My File.txt'"
        );
    }

    #[test]
    fn cmd_doubles_double_quote() {
        assert_eq!(
            quote_str("C:\\Program Files\\foo", Shell::Cmd),
            "\"C:\\Program Files\\foo\""
        );
        assert_eq!(quote_str("a\"b", Shell::Cmd), "\"a\"\"b\"");
    }

    #[test]
    fn format_paths_joins_with_space() {
        let paths = vec![PathBuf::from("/a/foo.txt"), PathBuf::from("/a/bar baz.txt")];
        assert_eq!(
            format_paths_for_drop(&paths, Shell::Posix),
            "/a/foo.txt '/a/bar baz.txt'"
        );
    }

    #[test]
    fn format_paths_empty_yields_empty() {
        let paths: Vec<PathBuf> = Vec::new();
        assert_eq!(format_paths_for_drop(&paths, Shell::Posix), "");
    }

    #[test]
    fn quote_path_handles_pathbuf() {
        let p = PathBuf::from("/Users/me/My Photos");
        assert_eq!(quote_path(&p, Shell::Posix), "'/Users/me/My Photos'");
    }

    // ----------------------------------------------------------------
    // Edge cases — characters that look harmless but aren't
    // ----------------------------------------------------------------

    #[test]
    fn posix_quotes_unicode_paths() {
        // Korean — multi-byte UTF-8 always trips the safe-set check, so
        // it ends up single-quoted. The bytes inside the quotes are
        // copied verbatim (`out.push(ch)` is char-aware).
        assert_eq!(
            quote_str("/Users/me/한글파일.txt", Shell::Posix),
            "'/Users/me/한글파일.txt'"
        );
        assert_eq!(quote_str("/x/файл", Shell::Posix), "'/x/файл'");
        assert_eq!(quote_str("/x/🦀.rs", Shell::Posix), "'/x/🦀.rs'");
    }

    #[test]
    fn posix_quotes_path_with_newline() {
        // macOS Finder allows newlines in filenames. Inside POSIX single
        // quotes a newline is literal — the shell receives one argv
        // entry containing the LF byte.
        assert_eq!(quote_str("a\nb.txt", Shell::Posix), "'a\nb.txt'");
    }

    #[test]
    fn posix_quotes_path_with_tab() {
        assert_eq!(quote_str("a\tb", Shell::Posix), "'a\tb'");
    }

    #[test]
    fn posix_quotes_path_with_backslash() {
        // Backslash isn't in POSIX_SAFE, so it triggers quoting. Inside
        // single quotes the backslash is literal — no double-escape.
        assert_eq!(quote_str("a\\b", Shell::Posix), "'a\\b'");
    }

    #[test]
    fn posix_quotes_tilde_paths() {
        // We must NOT let `~` expand. Quoting prevents it.
        assert_eq!(quote_str("~/foo.txt", Shell::Posix), "'~/foo.txt'");
    }

    #[test]
    fn posix_quotes_dollar_var_literally() {
        // `$HOME` inside the path must reach the shell as a literal,
        // not get expanded to the user's home directory.
        assert_eq!(quote_str("$HOME/foo", Shell::Posix), "'$HOME/foo'");
    }

    #[test]
    fn posix_leading_dash_passes_through_unchanged() {
        // `-rf` only contains characters in POSIX_SAFE so quoting skips
        // it — same behaviour as the `shell-escape` crate. The shell
        // won't word-split it, but `argv`-parsing programs may still
        // interpret it as a flag. The caller (e.g. a future Cmd-drag
        // → `cd <path>` handler) is responsible for adding `--` or `./`
        // when flag-injection matters.
        assert_eq!(quote_str("-rf", Shell::Posix), "-rf");
        assert_eq!(quote_str("--help", Shell::Posix), "--help");
    }

    #[test]
    fn posix_quotes_only_spaces() {
        assert_eq!(quote_str("   ", Shell::Posix), "'   '");
    }

    #[test]
    fn posix_quotes_glob_metacharacters() {
        assert_eq!(quote_str("a*", Shell::Posix), "'a*'");
        assert_eq!(quote_str("a?b", Shell::Posix), "'a?b'");
        assert_eq!(quote_str("[abc]", Shell::Posix), "'[abc]'");
        assert_eq!(quote_str("{a,b}", Shell::Posix), "'{a,b}'");
    }

    #[test]
    fn posix_quotes_redirect_and_pipe() {
        assert_eq!(quote_str("a>b", Shell::Posix), "'a>b'");
        assert_eq!(quote_str("a|b", Shell::Posix), "'a|b'");
        assert_eq!(quote_str("a;b", Shell::Posix), "'a;b'");
        assert_eq!(quote_str("a&b", Shell::Posix), "'a&b'");
        assert_eq!(quote_str("a`b", Shell::Posix), "'a`b'");
    }

    #[test]
    fn posix_quotes_double_quote_literally() {
        // Inside POSIX `'…'`, double-quote is literal — no escaping needed.
        assert_eq!(quote_str("a\"b", Shell::Posix), "'a\"b'");
    }

    #[test]
    fn posix_safe_set_excludes_high_ascii_bytes() {
        // Multi-byte UTF-8 starts with bytes >= 0x80, none of which are
        // in POSIX_SAFE. So `is_posix_safe` correctly flags it for
        // quoting without ever splitting a UTF-8 sequence mid-codepoint.
        assert!(!is_posix_safe("café"));
        assert!(!is_posix_safe("한"));
    }

    #[test]
    fn fish_double_escapes_backslashes() {
        // Verify `\\` inside fish single-quotes is escaped to `\\\\`,
        // because fish interprets `\\` as a literal backslash.
        assert_eq!(quote_str("C:\\Users", Shell::Fish), "'C:\\\\Users'");
    }

    #[test]
    fn pwsh_handles_consecutive_quotes() {
        assert_eq!(quote_str("a''b", Shell::PowerShell), "'a''''b'");
    }

    #[test]
    fn cmd_handles_path_separators_literally() {
        // Backslashes inside cmd's `"..."` are literal — no escape needed
        // for mid-string ones.
        assert_eq!(
            quote_str("C:\\Program Files\\app.exe", Shell::Cmd),
            "\"C:\\Program Files\\app.exe\""
        );
    }

    #[test]
    fn cmd_doubles_trailing_backslash_to_block_argv_injection() {
        // Dropping a directory from Explorer yields a path ending in `\`.
        // Without doubling, `"C:\Users\me\"` is parsed as an unterminated
        // string by CommandLineToArgvW and the closing `"` is treated as
        // a literal — letting the next dropped path or appended text
        // leak into the same argv token (the security concern).
        assert_eq!(
            quote_str("C:\\Users\\me\\", Shell::Cmd),
            "\"C:\\Users\\me\\\\\""
        );
        // Multiple trailing backslashes — each is doubled.
        assert_eq!(
            quote_str("C:\\dir\\\\\\", Shell::Cmd),
            "\"C:\\dir\\\\\\\\\\\\\""
        );
    }

    #[test]
    fn detect_handles_versioned_shell_names() {
        // /bin/bash5 or /usr/bin/bash-5.2 — versioned names fall through
        // to Posix (the safe default) rather than mismatching.
        assert_eq!(Shell::detect_from_program("/bin/bash5"), Shell::Posix);
        assert_eq!(
            Shell::detect_from_program("/usr/bin/bash-5.2"),
            Shell::Posix
        );
        assert_eq!(Shell::detect_from_program("zsh-5.9"), Shell::Posix);
    }

    #[test]
    fn detect_handles_extension_on_windows() {
        // file_stem strips `.exe` so `cmd.exe` and `pwsh.exe` map to
        // the right flavour.
        assert_eq!(
            Shell::detect_from_program("C:\\Windows\\cmd.exe"),
            Shell::Cmd
        );
        assert_eq!(Shell::detect_from_program("pwsh.exe"), Shell::PowerShell);
    }

    #[test]
    fn detect_strips_args_before_matching() {
        // `shell_program` may carry args (`/bin/bash -l`). The first
        // whitespace-delimited token is the program — strip the rest so
        // detection works correctly for the non-Posix shells too.
        assert_eq!(Shell::detect_from_program("/bin/bash -l"), Shell::Posix);
        assert_eq!(
            Shell::detect_from_program("/usr/bin/fish --no-config"),
            Shell::Fish
        );
        assert_eq!(
            Shell::detect_from_program("pwsh -NoLogo -NoProfile"),
            Shell::PowerShell
        );
    }

    #[test]
    fn empty_path_in_multi_drop_still_quotes() {
        // An entry with `PathBuf::from("")` produces `''` so the count
        // of argv tokens still matches the count of dropped files.
        let paths = vec![PathBuf::from(""), PathBuf::from("/a/b.txt")];
        assert_eq!(format_paths_for_drop(&paths, Shell::Posix), "'' /a/b.txt");
    }

    #[test]
    fn single_safe_path_is_not_wrapped() {
        // Idempotence: a path that's already shell-safe round-trips
        // unchanged through quote_path. Avoids visual noise like
        // `'/usr/bin/ls'` for plain paths.
        let p = PathBuf::from("/usr/bin/ls");
        assert_eq!(quote_path(&p, Shell::Posix), "/usr/bin/ls");
    }

    #[test]
    fn path_with_spaces_and_quotes_combo() {
        // Mixed: spaces force quoting, internal `'` triggers the
        // close-reopen dance.
        assert_eq!(
            quote_str("/x/can't open this.txt", Shell::Posix),
            "'/x/can'\\''t open this.txt'"
        );
    }
}
