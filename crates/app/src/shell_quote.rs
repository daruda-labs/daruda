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
    fn detect_shell_flavors() {
        let cases = [
            ("/bin/zsh", Shell::Posix),
            ("/bin/bash", Shell::Posix),
            ("sh", Shell::Posix),
            ("/usr/bin/dash", Shell::Posix),
            ("", Shell::Posix),
            ("/usr/bin/myshell", Shell::Posix),
            ("/bin/bash5", Shell::Posix),
            ("/usr/bin/bash-5.2", Shell::Posix),
            ("zsh-5.9", Shell::Posix),
            ("/bin/bash -l", Shell::Posix),
            ("/opt/homebrew/bin/fish", Shell::Fish),
            ("FISH", Shell::Fish),
            ("/usr/bin/fish --no-config", Shell::Fish),
            ("pwsh", Shell::PowerShell),
            ("pwsh.exe", Shell::PowerShell),
            ("pwsh -NoLogo -NoProfile", Shell::PowerShell),
            ("C:\\Windows\\System32\\powershell.exe", Shell::PowerShell),
            ("cmd.exe", Shell::Cmd),
            ("C:\\Windows\\cmd.exe", Shell::Cmd),
        ];

        for (program, expected) in cases {
            assert_eq!(Shell::detect_from_program(program), expected, "{program}");
        }
    }

    #[test]
    fn posix_quote_cases() {
        let cases = [
            ("/Users/me/foo.txt", "/Users/me/foo.txt"),
            ("relative/path-1.rs", "relative/path-1.rs"),
            ("-rf", "-rf"),
            ("--help", "--help"),
            ("/Users/me/My File.txt", "'/Users/me/My File.txt'"),
            ("/x/it's mine.txt", "'/x/it'\\''s mine.txt'"),
            ("", "''"),
            ("/Users/me/한글파일.txt", "'/Users/me/한글파일.txt'"),
            ("/x/файл", "'/x/файл'"),
            ("/x/🦀.rs", "'/x/🦀.rs'"),
            ("a\nb.txt", "'a\nb.txt'"),
            ("a\tb", "'a\tb'"),
            ("a\\b", "'a\\b'"),
            ("~/foo.txt", "'~/foo.txt'"),
            ("$HOME", "'$HOME'"),
            ("$HOME/foo", "'$HOME/foo'"),
            ("   ", "'   '"),
            ("a*", "'a*'"),
            ("a?b", "'a?b'"),
            ("[abc]", "'[abc]'"),
            ("{a,b}", "'{a,b}'"),
            ("a>b", "'a>b'"),
            ("a|b", "'a|b'"),
            ("a;b", "'a;b'"),
            ("a&b", "'a&b'"),
            ("a`b", "'a`b'"),
            ("a\"b", "'a\"b'"),
            ("/x/can't open this.txt", "'/x/can'\\''t open this.txt'"),
        ];

        for (input, expected) in cases {
            assert_eq!(quote_str(input, Shell::Posix), expected, "{input:?}");
        }
    }

    #[test]
    fn fish_quote_cases() {
        let cases = [
            ("it's", "'it\\'s'"),
            ("a\\b c", "'a\\\\b c'"),
            ("C:\\Users", "'C:\\\\Users'"),
        ];

        for (input, expected) in cases {
            assert_eq!(quote_str(input, Shell::Fish), expected, "{input:?}");
        }
    }

    #[test]
    fn powershell_quote_cases() {
        let cases = [
            ("it's", "'it''s'"),
            ("a''b", "'a''''b'"),
            ("C:\\Users\\me\\My File.txt", "'C:\\Users\\me\\My File.txt'"),
        ];

        for (input, expected) in cases {
            assert_eq!(quote_str(input, Shell::PowerShell), expected, "{input:?}");
        }
    }

    #[test]
    fn cmd_quote_cases() {
        let cases = [
            ("C:\\Program Files\\foo", "\"C:\\Program Files\\foo\""),
            (
                "C:\\Program Files\\app.exe",
                "\"C:\\Program Files\\app.exe\"",
            ),
            ("a\"b", "\"a\"\"b\""),
            ("C:\\Users\\me\\", "\"C:\\Users\\me\\\\\""),
            ("C:\\dir\\\\\\", "\"C:\\dir\\\\\\\\\\\\\""),
        ];

        for (input, expected) in cases {
            assert_eq!(quote_str(input, Shell::Cmd), expected, "{input:?}");
        }
    }

    #[test]
    fn format_paths_for_drop_cases() {
        let paths = vec![PathBuf::from("/a/foo.txt"), PathBuf::from("/a/bar baz.txt")];
        assert_eq!(
            format_paths_for_drop(&paths, Shell::Posix),
            "/a/foo.txt '/a/bar baz.txt'"
        );

        let paths: Vec<PathBuf> = Vec::new();
        assert_eq!(format_paths_for_drop(&paths, Shell::Posix), "");

        let paths = vec![PathBuf::from(""), PathBuf::from("/a/b.txt")];
        assert_eq!(format_paths_for_drop(&paths, Shell::Posix), "'' /a/b.txt");
    }

    #[test]
    fn quote_path_cases() {
        assert_eq!(
            quote_path(&PathBuf::from("/Users/me/My Photos"), Shell::Posix),
            "'/Users/me/My Photos'"
        );
        assert_eq!(
            quote_path(&PathBuf::from("/usr/bin/ls"), Shell::Posix),
            "/usr/bin/ls"
        );
    }

    #[test]
    fn posix_safe_set_excludes_non_ascii() {
        assert!(!is_posix_safe("café"));
        assert!(!is_posix_safe("한"));
    }
}
